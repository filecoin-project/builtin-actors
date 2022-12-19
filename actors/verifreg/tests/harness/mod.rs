use frc46_token::receiver::{FRC46TokenReceived, FRC46_TOKEN_TYPE};
use frc46_token::token::types::{BurnParams, BurnReturn, TransferParams};
use frc46_token::token::TOKEN_PRECISION;
use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::{MethodNum, HAMT_BIT_WIDTH};

use fil_actor_verifreg::ext::datacap::TOKEN_PRECISION;
use fil_actor_verifreg::testing::check_state_invariants;
use fil_actor_verifreg::{
    ext, Actor as VerifregActor, AddVerifiedClientParams, AddVerifierParams, Allocation,
    AllocationID, AllocationRequest, AllocationRequests, AllocationsResponse, Claim,
    ClaimAllocationsParams, ClaimAllocationsReturn, ClaimExtensionRequest, ClaimID, DataCap,
    ExtendClaimTermsParams, ExtendClaimTermsReturn, GetClaimsParams, GetClaimsReturn, Method,
    RemoveExpiredAllocationsParams, RemoveExpiredAllocationsReturn, RemoveExpiredClaimsParams,
    RemoveExpiredClaimsReturn, SectorAllocationClaim, State,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::policy_constants::{
    MAXIMUM_VERIFIED_ALLOCATION_TERM, MINIMUM_VERIFIED_ALLOCATION_TERM,
};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    make_empty_map, ActorError, AsActorError, BatchReturn, DATACAP_TOKEN_ACTOR_ADDR,
    STORAGE_MARKET_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;

pub const ROOT_ADDR: Address = Address::new_id(101);

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: ROOT_ADDR,
        caller: SYSTEM_ACTOR_ADDR,
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
    let h = Harness { root: ROOT_ADDR };
    h.construct_and_verify(&mut rt, &h.root);
    (h, rt)
}

pub struct Harness {
    pub root: Address,
}

impl Harness {
    pub fn construct_and_verify(&self, rt: &mut MockRuntime, root_param: &Address) {
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        let ret = rt
            .call::<VerifregActor>(
                Method::Constructor as MethodNum,
                IpldBlock::serialize_cbor(root_param).unwrap(),
            )
            .unwrap();

        assert!(ret.is_none());
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
            DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::BalanceOf as MethodNum,
            IpldBlock::serialize_cbor(&verifier_resolved).unwrap(),
            TokenAmount::zero(),
            IpldBlock::serialize_cbor(&BigIntSer(&(cap * TOKEN_PRECISION))).unwrap(),
            ExitCode::OK,
        );

        let params = AddVerifierParams { address: *verifier, allowance: allowance.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::AddVerifier as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )?;
        assert!(ret.is_none());
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
            IpldBlock::serialize_cbor(verifier).unwrap(),
        )?;
        assert!(ret.is_none());
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
            IpldBlock::serialize_cbor(&mint_params).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );

        let params = AddVerifiedClientParams { address: *client, allowance: allowance.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::AddVerifiedClient as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )?;
        assert!(ret.is_none());
        rt.verify();

        // Confirm the verifier was added to state.
        self.assert_client_allowance(rt, client, expected_allowance);
        Ok(())
    }

    pub fn assert_client_allowance(&self, rt: &MockRuntime, client: &Address, allowance: &DataCap) {
        let client_id_addr = rt.get_id_address(client).unwrap();
        assert_eq!(*allowance, self.get_client_allowance(rt, &client_id_addr));
    }

    pub fn get_client_allowance(&self, rt: &MockRuntime, client: &Address) -> DataCap {
        let clients = load_clients(rt);
        let BigIntDe(allowance) = clients.get(&client.to_bytes()).unwrap().unwrap();
        allowance.clone()
    }

    pub fn assert_client_removed(&self, rt: &MockRuntime, client: &Address) {
        let client_id_addr = rt.get_id_address(client).unwrap();
        let clients = load_clients(rt);
        assert!(!clients.contains_key(&client_id_addr.to_bytes()).unwrap())
    }

    pub fn add_verifier_and_client(
        &self,
        rt: &mut MockRuntime,
        verifier: &Address,
        client: &Address,
        verifier_allowance: &DataCap,
        client_allowance: &DataCap,
    ) {
        self.add_verifier(rt, verifier, verifier_allowance).unwrap();
        self.add_client(rt, verifier, client, client_allowance, client_allowance).unwrap();
    }

    pub fn use_bytes(
        &self,
        rt: &mut MockRuntime,
        client: &Address,
        amount: &DataCap,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*MARKET_ACTOR_CODE_ID, STORAGE_MARKET_ACTOR_ADDR);
        let params = UseBytesParams { address: *client, deal_size: amount.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::UseBytes as MethodNum,
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
        rt.expect_validate_caller_addr(vec![STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*MARKET_ACTOR_CODE_ID, STORAGE_MARKET_ACTOR_ADDR);
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
        let (_, acc) = check_state_invariants(&rt.get_state(), rt.store(), rt.epoch);
        acc.assert_empty();
    }

    // TODO this should be implemented through a call to verifreg but for now it modifies state directly
    pub fn create_alloc(
        &self,
        rt: &mut MockRuntime,
        alloc: &Allocation,
    ) -> Result<AllocationID, ActorError> {
        let mut st: State = rt.get_state();
        let mut allocs = st.load_allocs(rt.store()).unwrap();
        let alloc_id = st.next_allocation_id;
        assert!(allocs
            .put_if_absent(alloc.client, alloc_id, alloc.clone())
            .context_code(ExitCode::USR_ILLEGAL_STATE, "faild to put")?);
        st.next_allocation_id += 1;
        st.allocations = allocs.flush().expect("failed flushing allocation table");
        rt.replace_state(&st);
        Ok(alloc_id)
    }

    pub fn load_alloc(
        &self,
        rt: &mut MockRuntime,
        client: ActorID,
        id: AllocationID,
    ) -> Option<Allocation> {
        let st: State = rt.get_state();
        let mut allocs = st.load_allocs(rt.store()).unwrap();
        allocs.get(client, id).unwrap().cloned()
    }

    // Invokes the ClaimAllocations actor method
    pub fn claim_allocations(
        &self,
        rt: &mut MockRuntime,
        provider: ActorID,
        claim_allocs: Vec<SectorAllocationClaim>,
        datacap_burnt: u64,
        all_or_nothing: bool,
    ) -> Result<ClaimAllocationsReturn, ActorError> {
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        rt.set_caller(*MINER_ACTOR_CODE_ID, Address::new_id(provider));

        if datacap_burnt > 0 {
            rt.expect_send(
                DATACAP_TOKEN_ACTOR_ADDR,
                ext::datacap::Method::Burn as MethodNum,
                IpldBlock::serialize_cbor(&BurnParams {
                    amount: TokenAmount::from_whole(datacap_burnt.to_i64().unwrap()),
                })
                .unwrap(),
                TokenAmount::zero(),
                IpldBlock::serialize_cbor(&BurnReturn { balance: TokenAmount::zero() }).unwrap(),
                ExitCode::OK,
            );
        }

        let params = ClaimAllocationsParams { sectors: claim_allocs, all_or_nothing };
        let ret = rt
            .call::<VerifregActor>(
                Method::ClaimAllocations as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            )?
            .unwrap()
            .deserialize()
            .expect("failed to deserialize claim allocations return");
        rt.verify();
        Ok(ret)
    }

    // Invokes the RemoveExpiredAllocations actor method.
    pub fn remove_expired_allocations(
        &self,
        rt: &mut MockRuntime,
        client: ActorID,
        allocation_ids: Vec<AllocationID>,
        expected_datacap: u64,
    ) -> Result<RemoveExpiredAllocationsReturn, ActorError> {
        rt.expect_validate_caller_any();

        rt.expect_send(
            *DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Transfer as MethodNum,
            IpldBlock::serialize_cbor(&TransferParams {
                to: Address::new_id(client),
                amount: TokenAmount::from_whole(expected_datacap.to_i64().unwrap()),
                operator_data: RawBytes::default(),
            })
            .unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );

        let params = RemoveExpiredAllocationsParams { client, allocation_ids };
        let ret = rt
            .call::<VerifregActor>(
                Method::RemoveExpiredAllocations as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            )?
            .unwrap()
            .deserialize()
            .expect("failed to deserialize remove expired allocations return");
        rt.verify();
        Ok(ret)
    }

    // Invokes the RemoveExpiredClaims actor method.
    pub fn remove_expired_claims(
        &self,
        rt: &mut MockRuntime,
        provider: ActorID,
        claim_ids: Vec<ClaimID>,
    ) -> Result<RemoveExpiredClaimsReturn, ActorError> {
        rt.expect_validate_caller_any();

        let params = RemoveExpiredClaimsParams { provider, claim_ids };
        let ret = rt
            .call::<VerifregActor>(
                Method::RemoveExpiredClaims as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            )?
            .unwrap()
            .deserialize()
            .expect("failed to deserialize remove expired claims return");
        rt.verify();
        Ok(ret)
    }

    pub fn load_claim(
        &self,
        rt: &mut MockRuntime,
        provider: ActorID,
        id: ClaimID,
    ) -> Option<Claim> {
        let st: State = rt.get_state();
        let mut claims = st.load_claims(rt.store()).unwrap();
        claims.get(provider, id).unwrap().cloned()
    }

    pub fn receive_tokens(
        &self,
        rt: &mut MockRuntime,
        payload: FRC46TokenReceived,
        expected_alloc_results: BatchReturn,
        expected_extension_results: BatchReturn,
        expected_alloc_ids: Vec<AllocationID>,
        expected_burn: u64,
    ) -> Result<(), ActorError> {
        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, *DATACAP_TOKEN_ACTOR_ADDR);
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };

        if !expected_burn.is_zero() {
            rt.expect_send(
                DATACAP_TOKEN_ACTOR_ADDR,
                ext::datacap::Method::Burn as MethodNum,
                IpldBlock::serialize_cbor(&BurnParams {
                    amount: TokenAmount::from_whole(expected_burn),
                })
                .unwrap(),
                TokenAmount::zero(),
                IpldBlock::serialize_cbor(&BurnReturn { balance: TokenAmount::zero() }).unwrap(),
                ExitCode::OK,
            );
        }

        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        let ret = rt.call::<VerifregActor>(
            Method::UniversalReceiverHook as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )?;
        assert_eq!(
            AllocationsResponse {
                allocation_results: expected_alloc_results,
                extension_results: expected_extension_results,
                new_allocations: expected_alloc_ids,
            },
            ret.unwrap().deserialize().unwrap()
        );
        rt.verify();
        Ok(())
    }

    // Creates a claim directly in state.
    pub fn create_claim(&self, rt: &mut MockRuntime, claim: &Claim) -> Result<ClaimID, ActorError> {
        let mut st: State = rt.get_state();
        let mut claims = st.load_claims(rt.store()).unwrap();
        let id = st.next_allocation_id;
        assert!(claims
            .put_if_absent(claim.provider, id, claim.clone())
            .context_code(ExitCode::USR_ILLEGAL_STATE, "faild to put")?);
        st.next_allocation_id += 1;
        st.claims = claims.flush().expect("failed flushing allocation table");
        rt.replace_state(&st);
        Ok(id)
    }

    pub fn get_claims(
        &self,
        rt: &mut MockRuntime,
        provider: ActorID,
        claim_ids: Vec<ClaimID>,
    ) -> Result<GetClaimsReturn, ActorError> {
        rt.expect_validate_caller_any();
        let params = GetClaimsParams { claim_ids, provider };
        let ret = rt
            .call::<VerifregActor>(
                Method::GetClaims as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            )?
            .unwrap()
            .deserialize()
            .expect("failed to deserialize get claims return");
        rt.verify();
        Ok(ret)
    }

    pub fn extend_claim_terms(
        &self,
        rt: &mut MockRuntime,
        params: &ExtendClaimTermsParams,
    ) -> Result<ExtendClaimTermsReturn, ActorError> {
        rt.expect_validate_caller_any();
        let ret = rt
            .call::<VerifregActor>(
                Method::ExtendClaimTerms as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            )?
            .unwrap()
            .deserialize()
            .expect("failed to deserialize extend claim terms return");
        rt.verify();
        Ok(ret)
    }
}

pub fn make_alloc(data_id: &str, client: ActorID, provider: ActorID, size: u64) -> Allocation {
    Allocation {
        client,
        provider,
        data: make_piece_cid(data_id.as_bytes()),
        size: PaddedPieceSize(size),
        term_min: MINIMUM_VERIFIED_ALLOCATION_TERM,
        term_max: MINIMUM_VERIFIED_ALLOCATION_TERM * 2,
        expiration: 100,
    }
}

// Creates an allocation request for fixed data with default terms.
pub fn make_alloc_req(rt: &MockRuntime, provider: ActorID, size: u64) -> AllocationRequest {
    AllocationRequest {
        provider,
        data: make_piece_cid("1234".as_bytes()),
        size: PaddedPieceSize(size),
        term_min: MINIMUM_VERIFIED_ALLOCATION_TERM,
        term_max: MAXIMUM_VERIFIED_ALLOCATION_TERM,
        expiration: rt.epoch + 100,
    }
}

pub fn make_extension_req(
    provider: ActorID,
    claim: ClaimID,
    term_max: ChainEpoch,
) -> ClaimExtensionRequest {
    ClaimExtensionRequest { provider, claim, term_max }
}

// Creates the expected allocation from a request.
pub fn alloc_from_req(client: ActorID, req: &AllocationRequest) -> Allocation {
    Allocation {
        client,
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
    alloc: &Allocation,
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

#[allow(clippy::too_many_arguments)]
pub fn make_claim(
    data_id: &str,
    client: ActorID,
    provider: ActorID,
    size: u64,
    term_min: i64,
    term_max: i64,
    term_start: i64,
    sector: u64,
) -> Claim {
    Claim {
        provider,
        client,
        data: make_piece_cid(data_id.as_bytes()),
        size: PaddedPieceSize(size),
        term_min,
        term_max,
        term_start,
        sector,
    }
}

pub fn claim_from_alloc(alloc: &Allocation, term_start: ChainEpoch, sector: SectorNumber) -> Claim {
    Claim {
        provider: alloc.provider,
        client: alloc.client,
        data: alloc.data,
        size: alloc.size,
        term_min: alloc.term_min,
        term_max: alloc.term_max,
        term_start,
        sector,
    }
}

pub fn make_receiver_hook_token_payload(
    client: ActorID,
    alloc_requests: Vec<AllocationRequest>,
    extension_requests: Vec<ClaimExtensionRequest>,
    datacap_received: u64,
) -> FRC46TokenReceived {
    // let total_size: u64 = alloc_requests.iter().map(|r| r.size.0).sum();
    let payload =
        AllocationRequests { allocations: alloc_requests, extensions: extension_requests };
    FRC46TokenReceived {
        from: client,
        to: VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap(),
        operator: client,
        amount: TokenAmount::from_whole(datacap_received as i64),
        operator_data: serialize(&payload, "operator data").unwrap(),
        token_data: Default::default(),
    }
}

pub fn assert_allocation(
    rt: &MockRuntime,
    client: ActorID,
    id: AllocationID,
    expected: &Allocation,
) {
    let st: State = rt.get_state();
    let store = &rt.store();
    let mut allocs = st.load_allocs(store).unwrap();

    assert_eq!(expected, allocs.get(client, id).unwrap().unwrap());
}

pub fn assert_claim(rt: &MockRuntime, provider: ActorID, id: ClaimID, expected: &Claim) {
    let st: State = rt.get_state();
    let store = &rt.store();
    let mut claims = st.load_claims(store).unwrap();

    assert_eq!(expected, claims.get(provider, id).unwrap().unwrap());
}

pub fn assert_alloc_claimed(
    rt: &MockRuntime,
    client: ActorID,
    provider: ActorID,
    id: ClaimID,
    alloc: &Allocation,
    epoch: ChainEpoch,
    sector: SectorNumber,
) -> Claim {
    let st: State = rt.get_state();
    let store = &rt.store();

    // Alloc is gone
    let mut allocs = st.load_allocs(&store).unwrap();
    assert!(allocs.get(client, id).unwrap().is_none());

    // Claim is present
    let expected_claim = claim_from_alloc(alloc, epoch, sector);
    let mut claims = st.load_claims(store).unwrap();
    assert_eq!(&expected_claim, claims.get(provider, id).unwrap().unwrap());
    expected_claim
}
