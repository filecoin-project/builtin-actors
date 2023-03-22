use std::cell::RefCell;

use frc46_token::receiver::{FRC46TokenReceived, FRC46_TOKEN_TYPE};
use frc46_token::token::types::{
    BurnReturn, MintReturn, TransferFromParams, TransferFromReturn, TransferParams, TransferReturn,
};
use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;
use num_traits::Zero;

use fil_actor_datacap::testing::check_state_invariants;
use fil_actor_datacap::{Actor as DataCapActor, DestroyParams, Method, MintParams, State};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    ActorError, DATACAP_TOKEN_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: DATACAP_TOKEN_ACTOR_ADDR,
        caller: RefCell::new(SYSTEM_ACTOR_ADDR),
        caller_type: RefCell::new(*SYSTEM_ACTOR_CODE_ID),
        ..Default::default()
    }
}

#[allow(dead_code)]
pub fn new_harness() -> (Harness, MockRuntime) {
    let rt = new_runtime();
    let h = Harness { governor: VERIFIED_REGISTRY_ACTOR_ADDR };
    h.construct_and_verify(&rt, &h.governor);
    (h, rt)
}

pub struct Harness {
    pub governor: Address,
}

impl Harness {
    pub fn construct_and_verify(&self, rt: &MockRuntime, registry: &Address) {
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        let ret = rt
            .call::<DataCapActor>(
                Method::Constructor as MethodNum,
                IpldBlock::serialize_cbor(registry).unwrap(),
            )
            .unwrap();

        assert!(ret.is_none());
        rt.verify();

        let state: State = rt.get_state();
        assert_eq!(self.governor, state.governor);
    }

    pub fn mint(
        &self,
        rt: &MockRuntime,
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
        rt.expect_send_simple(
            *to,
            frc42_dispatch::method_hash!("Receive"),
            IpldBlock::serialize_cbor(&hook_params).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );

        let params = MintParams { to: *to, amount: amount.clone(), operators };
        rt.set_caller(*VERIFREG_ACTOR_CODE_ID, VERIFIED_REGISTRY_ACTOR_ADDR);
        let ret = rt.call::<DataCapActor>(
            Method::MintExported as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )?;

        rt.verify();
        Ok(ret.unwrap().deserialize().unwrap())
    }

    pub fn destroy(
        &self,
        rt: &MockRuntime,
        owner: &Address,
        amount: &TokenAmount,
    ) -> Result<BurnReturn, ActorError> {
        rt.expect_validate_caller_addr(vec![VERIFIED_REGISTRY_ACTOR_ADDR]);

        let params = DestroyParams { owner: *owner, amount: amount.clone() };

        rt.set_caller(*VERIFREG_ACTOR_CODE_ID, VERIFIED_REGISTRY_ACTOR_ADDR);
        let ret = rt.call::<DataCapActor>(
            Method::DestroyExported as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )?;

        rt.verify();
        Ok(ret.unwrap().deserialize().unwrap())
    }

    pub fn transfer(
        &self,
        rt: &MockRuntime,
        from: &Address,
        to: &Address,
        amount: &TokenAmount,
        operator_data: RawBytes,
    ) -> Result<TransferReturn, ActorError> {
        rt.expect_validate_caller_any();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *from);

        // Expect the token receiver hook to be called.
        let hook_params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(
                &FRC46TokenReceived {
                    from: from.id().unwrap(),
                    to: to.id().unwrap(),
                    operator: from.id().unwrap(),
                    amount: amount.clone(),
                    operator_data: operator_data.clone(),
                    token_data: Default::default(),
                },
                "hook payload",
            )?,
        };
        // UniversalReceiverParams
        rt.expect_send_simple(
            *to,
            frc42_dispatch::method_hash!("Receive"),
            IpldBlock::serialize_cbor(&hook_params).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );

        let params = TransferParams { to: *to, amount: amount.clone(), operator_data };
        let ret = rt.call::<DataCapActor>(
            Method::TransferExported as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )?;

        rt.verify();
        Ok(ret.unwrap().deserialize().unwrap())
    }

    pub fn transfer_from(
        &self,
        rt: &MockRuntime,
        operator: &Address,
        from: &Address,
        to: &Address,
        amount: &TokenAmount,
        operator_data: RawBytes,
    ) -> Result<TransferFromReturn, ActorError> {
        rt.expect_validate_caller_any();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *operator);

        // Expect the token receiver hook to be called.
        let hook_params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(
                &FRC46TokenReceived {
                    from: from.id().unwrap(),
                    to: to.id().unwrap(),
                    operator: operator.id().unwrap(),
                    amount: amount.clone(),
                    operator_data: operator_data.clone(),
                    token_data: Default::default(),
                },
                "hook payload",
            )?,
        };
        // UniversalReceiverParams
        rt.expect_send_simple(
            *to,
            frc42_dispatch::method_hash!("Receive"),
            IpldBlock::serialize_cbor(&hook_params).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );

        let params =
            TransferFromParams { to: *to, from: *from, amount: amount.clone(), operator_data };
        let ret = rt.call::<DataCapActor>(
            Method::TransferFromExported as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )?;

        rt.verify();
        Ok(ret.unwrap().deserialize().unwrap())
    }

    // Reads the total supply from state directly.
    pub fn get_supply(&self, rt: &MockRuntime) -> TokenAmount {
        rt.get_state::<State>().token.supply
    }

    // Reads a balance from state directly.
    pub fn get_balance(&self, rt: &MockRuntime, address: &Address) -> TokenAmount {
        rt.expect_validate_caller_any();
        let ret = rt
            .call::<DataCapActor>(
                Method::BalanceExported as MethodNum,
                IpldBlock::serialize_cbor(&address).unwrap(),
            )
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();
        rt.verify();
        ret
    }

    // Reads allowance from state directly
    pub fn get_allowance_between(
        &self,
        rt: &MockRuntime,
        owner: &Address,
        operator: &Address,
    ) -> TokenAmount {
        rt.get_state::<State>()
            .token
            .get_allowance_between(rt.store(), owner.id().unwrap(), operator.id().unwrap())
            .unwrap()
    }

    pub fn check_state(&self, rt: &MockRuntime) {
        let (_, acc) = check_state_invariants(&rt.get_state(), rt.store());
        acc.assert_empty();
    }
}
