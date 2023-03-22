use fvm_ipld_encoding::RawBytes;
use fvm_workbench_api::wrangler::ExecutionWrangler;
use fvm_workbench_api::Message;

use crate::VM;

impl<'a> VM for ExecutionWrangler<'a> {
    fn send_message(
        &self,
        from: fvm_shared::address::Address,
        to: fvm_shared::address::Address,
        value: fvm_shared::econ::TokenAmount,
        method: fvm_shared::MethodNum,
        params: Option<fvm_ipld_encoding::ipld_block::IpldBlock>,
    ) -> Result<crate::MessageResult, crate::TestVMError> {
        let params = params.map_or(RawBytes::default(), |b| RawBytes::from(b.data));
        let res = self.execute(from, to, method, params, value);
    }

    fn resolve_address(
        &self,
        addr: &fvm_shared::address::Address,
    ) -> Option<fvm_shared::address::Address> {
        todo!();
    }
}
