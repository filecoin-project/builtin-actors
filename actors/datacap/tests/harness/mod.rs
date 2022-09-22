use fil_fungible_token::receiver::types::{
    FRC46TokenReceived, UniversalReceiverParams, FRC46_TOKEN_TYPE,
};
use fil_fungible_token::token::types::MintReturn;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;
use num_traits::Zero;

use fil_actor_datacap::testing::check_state_invariants;
use fil_actor_datacap::{Actor as DataCapActor, Method, MintParams, State};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    ActorError, DATACAP_TOKEN_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, UNIVERSAL_RECEIVER_HOOK_METHOD_NUM,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: DATACAP_TOKEN_ACTOR_ADDR,
        caller: SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

#[allow(dead_code)]
pub fn new_harness() -> (Harness, MockRuntime) {
    let mut rt = new_runtime();
    let h = Harness { registry: VERIFIED_REGISTRY_ACTOR_ADDR };
    h.construct_and_verify(&mut rt, &h.registry);
    (h, rt)
}

pub struct Harness {
    pub registry: Address,
}

impl Harness {
    pub fn construct_and_verify(&self, rt: &mut MockRuntime, registry: &Address) {
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        let ret = rt
            .call::<DataCapActor>(
                Method::Constructor as MethodNum,
                &RawBytes::serialize(registry).unwrap(),
            )
            .unwrap();

        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        let state: State = rt.get_state();
        assert_eq!(self.registry, state.governor);
    }

    pub fn mint(
        &self,
        rt: &mut MockRuntime,
        to: &Address,
        amount: &TokenAmount,
        operators: Vec<Address>,
    ) -> Result<MintReturn, ActorError> {
        rt.expect_validate_caller_addr(vec![VERIFIED_REGISTRY_ACTOR_ADDR]);

        // Expect the token receiver hook to be called.
        let hook_params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(
                &FRC46TokenReceived {
                    from: DATACAP_TOKEN_ACTOR_ADDR.id().unwrap(),
                    to: to.id().unwrap(),
                    operator: VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap(),
                    amount: amount.clone(),
                    operator_data: Default::default(),
                    token_data: Default::default(),
                },
                "hook payload",
            )?,
        };
        // UniversalReceiverParams
        rt.expect_send(
            *to,
            UNIVERSAL_RECEIVER_HOOK_METHOD_NUM,
            serialize(&hook_params, "hook params")?,
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        let params = MintParams { to: *to, amount: amount.clone(), operators };
        rt.set_caller(*VERIFREG_ACTOR_CODE_ID, VERIFIED_REGISTRY_ACTOR_ADDR);
        let ret =
            rt.call::<DataCapActor>(Method::Mint as MethodNum, &serialize(&params, "params")?)?;

        rt.verify();
        Ok(ret.deserialize().unwrap())
    }

    // Reads the total supply from state directly.
    pub fn get_supply(&self, rt: &MockRuntime) -> TokenAmount {
        rt.get_state::<State>().token.supply
    }

    // Reads a balance from state directly.
    pub fn get_balance(&self, rt: &MockRuntime, address: &Address) -> TokenAmount {
        rt.get_state::<State>().token.get_balance(rt.store(), address.id().unwrap()).unwrap()
    }

    pub fn check_state(&self, rt: &MockRuntime) {
        let (_, acc) = check_state_invariants(&rt.get_state(), rt.store());
        acc.assert_empty();
    }
}
