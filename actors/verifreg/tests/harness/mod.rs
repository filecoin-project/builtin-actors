use fil_fungible_token::receiver::types::{
    FRC46TokenReceived, UniversalReceiverParams, FRC46_TOKEN_TYPE,
};
use fil_fungible_token::token::types::{BurnParams, BurnReturn, TransferParams};
use fil_fungible_token::token::TOKEN_PRECISION;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::{BigIntDe, BigIntSer};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::SectorNumber;
use fvm_shared::{ActorID, MethodNum, HAMT_BIT_WIDTH};
use lazy_static::lazy_static;
use num_traits::{ToPrimitive, Zero};

use fil_actor_verifreg::testing::check_state_invariants;
use fil_actor_verifreg::{
    ext, Actor as VerifregActor, AddVerifierClientParams, AddVerifierParams, Allocation,
    AllocationID, AllocationRequest, AllocationRequests, ClaimAllocationsParams,
    ClaimAllocationsReturn, DataCap, Method, RemoveExpiredAllocationsParams,
    RemoveExpiredAllocationsReturn, RestoreBytesParams, SectorAllocationClaim, State,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::policy_constants::{
    MAXIMUM_VERIFIED_ALLOCATION_TERM, MINIMUM_VERIFIED_ALLOCATION_TERM,
};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    make_empty_map, ActorError, AsActorError, MapMap, DATACAP_TOKEN_ACTOR_ADDR,
    STORAGE_MARKET_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};

lazy_static! {
    pub static ref ROOT_ADDR: Address = Address::new_id(101);
}

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: *VERIFIED_REGISTRY_ACTOR_ADDR,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

// Sets the miner code/type for an actor ID
pub fn add_miner(rt: &mut MockRuntime, id: ActorID) {
    rt.set_address_actor_type(Address::new_id(id), *MINER_ACTOR_CODE_ID);
}

pub fn new_harness() -> (Harness, MockRuntime) {
    let mut rt = new_runtime();
    let h = Harness { root: *ROOT_ADDR };
    h.construct_and_verify(&mut rt, &h.root);
    (h, rt)
}

pub struct Harness {
    pub root: Address,
}

impl Harness {
    pub fn construct_and_verify(&self, rt: &mut MockRuntime, root_param: &Address) {
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        let ret = rt
            .call::<VerifregActor>(
                Method::Constructor as MethodNum,
                &RawBytes::serialize(root_param).unwrap(),
            )
            .unwrap();

        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        let empty_map = make_empty_map::<_, ()>(&rt.store, HAMT_BIT_WIDTH).flush().unwrap();
        let state: State = rt.get_state();
        assert_eq!(self.root, state.root_key);
        assert_eq!(empty_map, state.verifiers);
    }

    pub fn add_verifier(
        &self,
        rt: &mut MockRuntime,
        verifier: &Address,
        allowance: &DataCap,
    ) -> Result<(), ActorError> {
        self.add_verifier_with_existing_cap(rt, verifier, allowance, &DataCap::zero())
    }

    pub fn add_verifier_with_existing_cap(
        &self,
        rt: &mut MockRuntime,
        verifier: &Address,
        allowance: &DataCap,
        cap: &DataCap, // Mocked data cap balance of the prospective verifier
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![self.root]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.root);
        let verifier_resolved = rt.get_id_address(verifier).unwrap_or(*verifier);
        // Expect checking the verifier's token balance.
        rt.expect_send(
            *DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::BalanceOf as MethodNum,
            RawBytes::serialize(&verifier_resolved).unwrap(),
            TokenAmount::zero(),
            serialize(&BigIntSer(&(cap * TOKEN_PRECISION)), "").unwrap(),
            ExitCode::OK,
        );

        let params = AddVerifierParams { address: *verifier, allowance: allowance.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::AddVerifier as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        self.assert_verifier_allowance(rt, verifier, allowance);
        Ok(())
    }

    pub fn remove_verifier(
        &self,
        rt: &mut MockRuntime,
        verifier: &Address,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![self.root]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.root);
        let ret = rt.call::<VerifregActor>(
            Method::RemoveVerifier as MethodNum,
            &RawBytes::serialize(verifier).unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        self.assert_verifier_removed(rt, verifier);
        Ok(())
    }

    pub fn assert_verifier_allowance(
        &self,
        rt: &MockRuntime,
        verifier: &Address,
        allowance: &DataCap,
    ) {
        let verifier_id_addr = rt.get_id_address(verifier).unwrap();
        assert_eq!(*allowance, self.get_verifier_allowance(rt, &verifier_id_addr));
    }

    pub fn get_verifier_allowance(&self, rt: &MockRuntime, verifier: &Address) -> DataCap {
        let verifiers = rt.get_state::<State>().load_verifiers(&rt.store).unwrap();
        let BigIntDe(allowance) = verifiers.get(&verifier.to_bytes()).unwrap().unwrap();
        allowance.clone()
    }

    pub fn assert_verifier_removed(&self, rt: &MockRuntime, verifier: &Address) {
        let verifier_id_addr = rt.get_id_address(verifier).unwrap();
        let verifiers = rt.get_state::<State>().load_verifiers(&rt.store).unwrap();
        assert!(!verifiers.contains_key(&verifier_id_addr.to_bytes()).unwrap())
    }

    pub fn add_client(
        &self,
        rt: &mut MockRuntime,
        verifier: &Address,
        client: &Address,
        allowance: &DataCap,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_any();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *verifier);
        let client_resolved = rt.get_id_address(client).unwrap_or(*client);

        // Expect tokens to be minted.
        let mint_params = ext::datacap::MintParams {
            to: client_resolved,
            amount: TokenAmount::from_whole(allowance.to_i64().unwrap()),
            operators: vec![*STORAGE_MARKET_ACTOR_ADDR],
        };
        rt.expect_send(
            *DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Mint as MethodNum,
            RawBytes::serialize(&mint_params).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        let params = AddVerifierClientParams { address: *client, allowance: allowance.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::AddVerifiedClient as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        Ok(())
    }

    pub fn restore_bytes(
        &self,
        rt: &mut MockRuntime,
        client: &Address,
        amount: &DataCap,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![*STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*MARKET_ACTOR_CODE_ID, *STORAGE_MARKET_ACTOR_ADDR);
        let client_resolved = rt.get_id_address(client).unwrap_or(*client);

        // Expect tokens to be minted.
        let mint_params = ext::datacap::MintParams {
            to: client_resolved,
            amount: TokenAmount::from_whole(amount.to_i64().unwrap()),
            operators: vec![*STORAGE_MARKET_ACTOR_ADDR],
        };
        rt.expect_send(
            *DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Mint as MethodNum,
            RawBytes::serialize(&mint_params).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        let params = RestoreBytesParams { address: *client, deal_size: amount.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::RestoreBytes as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();
        Ok(())
    }

    pub fn check_state(&self, rt: &MockRuntime) {
        let (_, acc) = check_state_invariants(&rt.get_state(), rt.store());
        acc.assert_empty();
    }

    // TODO this should be implemented through a call to verifreg but for now it modifies state directly
    pub fn create_alloc(&self, rt: &mut MockRuntime, alloc: &Allocation) -> Result<(), ActorError> {
        let mut st: State = rt.get_state();
        let mut allocs =
            MapMap::from_root(rt.store(), &st.allocations, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load allocations table")?;
        assert!(allocs
            .put_if_absent(alloc.client, st.next_allocation_id, alloc.clone())
            .context_code(ExitCode::USR_ILLEGAL_STATE, "faild to put")?);
        st.next_allocation_id += 1;
        st.allocations = allocs.flush().expect("failed flushing allocation table");
        rt.replace_state(&st);

        Ok(())
    }

    // Invokes the ClaimAllocations actor method
    pub fn claim_allocations(
        &self,
        rt: &mut MockRuntime,
        provider: Address,
        claim_allocs: Vec<SectorAllocationClaim>,
        datacap_burnt: u64,
    ) -> Result<ClaimAllocationsReturn, ActorError> {
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        rt.set_caller(*MINER_ACTOR_CODE_ID, provider);

        rt.expect_send(
            *DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Burn as MethodNum,
            RawBytes::serialize(&BurnParams {
                amount: TokenAmount::from_whole(datacap_burnt.to_i64().unwrap()),
            })
            .unwrap(),
            TokenAmount::zero(),
            RawBytes::serialize(&BurnReturn { balance: TokenAmount::zero() }).unwrap(),
            ExitCode::OK,
        );

        let params = ClaimAllocationsParams { sectors: claim_allocs };
        let ret = rt
            .call::<VerifregActor>(
                Method::ClaimAllocations as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            )?
            .deserialize()
            .expect("failed to deserialize claim allocations return");
        rt.verify();
        Ok(ret)
    }

    // Invokes the RemoveExpiredAllocations actor method.
    pub fn remove_expired_allocations(
        &self,
        rt: &mut MockRuntime,
        client: &Address,
        allocation_ids: Vec<AllocationID>,
        expected_datacap: u64,
    ) -> Result<RemoveExpiredAllocationsReturn, ActorError> {
        rt.expect_validate_caller_any();

        rt.expect_send(
            *DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Transfer as MethodNum,
            RawBytes::serialize(&TransferParams {
                to: *client,
                amount: TokenAmount::from_whole(expected_datacap.to_i64().unwrap()),
                operator_data: RawBytes::default(),
            })
            .unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        let params = RemoveExpiredAllocationsParams { client: *client, allocation_ids };
        let ret = rt
            .call::<VerifregActor>(
                Method::RemoveExpiredAllocations as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            )?
            .deserialize()
            .expect("failed to deserialize remove expired allocations return");
        rt.verify();
        Ok(ret)
    }

    pub fn receive_tokens(
        &self,
        rt: &mut MockRuntime,
        payload: FRC46TokenReceived,
    ) -> Result<(), ActorError> {
        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, *DATACAP_TOKEN_ACTOR_ADDR);
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };

        rt.expect_validate_caller_addr(vec![*DATACAP_TOKEN_ACTOR_ADDR]);
        let ret = rt.call::<VerifregActor>(
            Method::UniversalReceiverHook as MethodNum,
            &serialize(&params, "hook params").unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();
        Ok(())
    }
}

pub fn make_alloc(data_id: &str, client: &Address, provider: &Address, size: u64) -> Allocation {
    Allocation {
        client: *client,
        provider: *provider,
        data: make_piece_cid(data_id.as_bytes()),
        size: PaddedPieceSize(size),
        term_min: 1000,
        term_max: 2000,
        expiration: 100,
    }
}

// Creates an allocation request for fixed data with default terms.
pub fn make_alloc_req(rt: &MockRuntime, provider: ActorID, size: u64) -> AllocationRequest {
    AllocationRequest {
        provider: Address::new_id(provider),
        data: make_piece_cid("1234".as_bytes()),
        size: PaddedPieceSize(size),
        term_min: MINIMUM_VERIFIED_ALLOCATION_TERM,
        term_max: MAXIMUM_VERIFIED_ALLOCATION_TERM,
        expiration: rt.epoch + 100,
    }
}

// Creates the expected allocation from a request.
pub fn alloc_from_req(client: &Address, req: &AllocationRequest) -> Allocation {
    Allocation {
        client: *client,
        provider: req.provider,
        data: req.data,
        size: req.size,
        term_min: req.term_min,
        term_max: req.term_max,
        expiration: req.expiration,
    }
}

pub fn make_claim_req(
    id: AllocationID,
    alloc: Allocation,
    sector_id: SectorNumber,
    sector_expiry: ChainEpoch,
) -> SectorAllocationClaim {
    SectorAllocationClaim {
        client: alloc.client,
        allocation_id: id,
        data: alloc.data,
        size: alloc.size,
        sector: sector_id,
        sector_expiry,
    }
}

pub fn make_receiver_hook_token_payload(
    client: ActorID,
    requests: Vec<AllocationRequest>,
) -> FRC46TokenReceived {
    let total_size: u64 = requests.iter().map(|r| r.size.0).sum();
    let payload = AllocationRequests { requests };
    FRC46TokenReceived {
        from: client,
        to: VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap(),
        operator: client,
        amount: TokenAmount::from_whole(total_size as i64),
        operator_data: serialize(&payload, "operator data").unwrap(),
        token_data: Default::default(),
    }
}
