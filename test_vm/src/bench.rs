use std::cell::RefCell;

use fvm_ipld_encoding::{ipld_block::IpldBlock, RawBytes, DAG_CBOR};
use fvm_shared::{address::Address, crypto::signature::SignatureType, ActorID, METHOD_SEND};
use fvm_workbench_api::{wrangler::ExecutionWrangler, ExecutionResult};
use fvm_workbench_builtin_actors::genesis::GenesisResult;
use fvm_workbench_builtin_actors::util::*;

use crate::{util::apply_ok_, MessageResult, TestVMError, VM};

pub struct Benchmarker<'a> {
    pub wrangler: RefCell<ExecutionWrangler<'a>>,
    pub execution_results: RefCell<Vec<ExecutionResult>>,
    genesis: GenesisResult,
}

impl<'a> Benchmarker<'a> {
    pub fn new(wrangler: ExecutionWrangler<'a>, genesis: GenesisResult) -> Self {
        Self {
            wrangler: RefCell::new(wrangler),
            execution_results: RefCell::new(Vec::new()),
            genesis,
        }
    }
}

impl<'a> VM for Benchmarker<'a> {
    fn send_message(
        &self,
        from: Address,
        to: Address,
        value: fvm_shared::econ::TokenAmount,
        method: fvm_shared::MethodNum,
        params: Option<fvm_ipld_encoding::ipld_block::IpldBlock>,
    ) -> Result<MessageResult, TestVMError> {
        let params = params.map_or(RawBytes::default(), |b| RawBytes::from(b.data));
        let res = self
            .wrangler
            .borrow_mut()
            .execute(from, to, method, params, value)
            .map_err(|e| TestVMError { msg: e.to_string() })?;

        self.execution_results.borrow_mut().push(res.clone());

        Ok(MessageResult {
            code: res.receipt.exit_code,
            message: res.message,
            ret: Some(IpldBlock { codec: DAG_CBOR, data: res.receipt.return_data.into() }),
        })
    }

    fn resolve_address(&self, addr: &Address) -> Option<Address> {
        let res = self.wrangler.borrow().resolve_address(addr).map_or(None, |a| a);
        res.map(|id| Address::new_id(id))
    }

    fn create_accounts_seeded(
        &self,
        count: u64,
        balance: fvm_shared::econ::TokenAmount,
        typ: fvm_shared::crypto::signature::SignatureType,
        seed: u64,
    ) -> Result<Vec<Address>, TestVMError> {
        let keys = match typ {
            SignatureType::Secp256k1 => make_secp_keys(seed, count),
            SignatureType::BLS => make_bls_keys(seed, count),
        };

        // Send funds from faucet to pk address, creating account actor
        for key in keys.iter() {
            apply_ok_(
                self,
                Address::new_id(self.genesis.faucet_id),
                key.addr,
                balance.clone(),
                METHOD_SEND,
                None::<RawBytes>,
            );
        }
        // Resolve pk address to return ID of account actor
        let addresses: Vec<Address> =
            keys.into_iter().map(|key| self.resolve_address(&key.addr).unwrap()).collect();
        // let accounts =
        //     keys.into_iter().enumerate().map(|(i, key)| Account { id: ids[i], key }).collect();
        Ok(addresses)
    }
}
