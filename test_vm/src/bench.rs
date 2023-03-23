use std::cell::RefCell;

use fvm_ipld_encoding::{ipld_block::IpldBlock, RawBytes, DAG_CBOR};
use fvm_shared::address::Address;
use fvm_workbench_api::{wrangler::ExecutionWrangler, ExecutionResult};

use crate::{MessageResult, TestVMError, VM};

pub struct Benchmarker<'a> {
    pub wrangler: RefCell<ExecutionWrangler<'a>>,
    pub execution_results: RefCell<Vec<ExecutionResult>>,
}

impl<'a> Benchmarker<'a> {
    pub fn new(wrangler: ExecutionWrangler<'a>) -> Self {
        Self { wrangler: RefCell::new(wrangler), execution_results: RefCell::new(Vec::new()) }
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
}
