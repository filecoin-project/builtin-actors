use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::{BigIntDe, BigIntSer};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{MethodNum, HAMT_BIT_WIDTH};
use lazy_static::lazy_static;
use num_traits::{Signed, Zero};

use fil_actor_verifreg::ext::datacap::TOKEN_PRECISION;
use fil_actor_verifreg::testing::check_state_invariants;
use fil_actor_verifreg::{
    ext, Actor as VerifregActor, AddVerifierClientParams, AddVerifierParams, Allocation,
    ClaimAllocationParams, ClaimAllocationReturn, DataCap, Method, RestoreBytesParams,
    SectorAllocationClaim, State, UseBytesParams,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    make_empty_map, make_map_with_root_and_bitwidth, ActorError, AsActorError, Map, MapMap,
    DATACAP_TOKEN_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};

lazy_static! {
    pub static ref ROOT_ADDR: Address = Address::new_id(101);
}

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: *ROOT_ADDR,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
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
        let verifiers = load_verifiers(rt);
        let BigIntDe(allowance) = verifiers.get(&verifier.to_bytes()).unwrap().unwrap();
        allowance.clone()
    }

    pub fn assert_verifier_removed(&self, rt: &MockRuntime, verifier: &Address) {
        let verifier_id_addr = rt.get_id_address(verifier).unwrap();
        let verifiers = load_verifiers(rt);
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
        let mint_params =
            ext::datacap::MintParams { to: client_resolved, amount: allowance * TOKEN_PRECISION };
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

    pub fn use_bytes(
        &self,
        rt: &mut MockRuntime,
        client: &Address,
        amount: &DataCap,
        result: ExitCode,    // Mocked exit code from the token destroy
        remaining: &DataCap, // Mocked remaining balance after token destroy
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![*STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*MARKET_ACTOR_CODE_ID, *STORAGE_MARKET_ACTOR_ADDR);
        let client_resolved = rt.get_id_address(client).unwrap_or(*client);

        // Expect tokens to be destroyed.
        let destroy_params = ext::datacap::DestroyParams {
            owner: client_resolved,
            amount: amount * TOKEN_PRECISION,
        };
        rt.expect_send(
            *DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Destroy as MethodNum,
            RawBytes::serialize(&destroy_params).unwrap(),
            TokenAmount::zero(),
            serialize(&BigIntSer(&(remaining * TOKEN_PRECISION)), "").unwrap(),
            result,
        );

        // Expect second destroy if remaining balance is below minimum.
        if remaining.is_positive() && remaining < &rt.policy.minimum_verified_deal_size {
            let destroy_params = ext::datacap::DestroyParams {
                owner: client_resolved,
                amount: remaining * TOKEN_PRECISION,
            };
            rt.expect_send(
                *DATACAP_TOKEN_ACTOR_ADDR,
                ext::datacap::Method::Destroy as MethodNum,
                RawBytes::serialize(&destroy_params).unwrap(),
                TokenAmount::zero(),
                serialize(&BigIntSer(&TokenAmount::zero()), "").unwrap(),
                result,
            );
        }

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
        rt.expect_validate_caller_addr(vec![*STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*MARKET_ACTOR_CODE_ID, *STORAGE_MARKET_ACTOR_ADDR);
        let client_resolved = rt.get_id_address(client).unwrap_or(*client);

        // Expect tokens to be minted.
        let mint_params =
            ext::datacap::MintParams { to: client_resolved, amount: amount * TOKEN_PRECISION };
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
    pub fn create_alloc(&self, rt: &mut MockRuntime, alloc: Allocation) -> Result<(), ActorError> {
        let mut st: State = rt.get_state();
        let mut allocs =
            MapMap::from_root(rt.store(), &st.allocations, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load allocations table")?;
        assert!(allocs
            .put_if_absent(alloc.client, st.next_allocation_id, alloc)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "faild to put")?);
        st.next_allocation_id += 1;
        st.allocations = allocs.flush().expect("failed flushing allocation table");
        rt.replace_state(&st);

        Ok(())
    }

    pub fn claim_allocations(
        &self,
        rt: &mut MockRuntime,
        provider: Address,
        claim_allocs: Vec<SectorAllocationClaim>,
    ) -> Result<ClaimAllocationReturn, ActorError> {
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        rt.set_caller(*MINER_ACTOR_CODE_ID, provider);

        let params = ClaimAllocationParams { sectors: claim_allocs };

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
}

fn load_verifiers(rt: &MockRuntime) -> Map<MemoryBlockstore, BigIntDe> {
    let state: State = rt.get_state();
    make_map_with_root_and_bitwidth::<_, BigIntDe>(&state.verifiers, &rt.store, HAMT_BIT_WIDTH)
        .unwrap()
}
