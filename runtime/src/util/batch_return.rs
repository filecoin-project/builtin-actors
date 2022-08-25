use fvm_shared::error::ExitCode;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug)]
pub struct FailCode {
    pub idx: usize,
    pub code: ExitCode,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct BatchReturn {
    pub batch_size: usize,
    pub fail_codes: Vec<FailCode>,
}

impl BatchReturn {
    pub fn codes(&self) -> Vec<ExitCode> {
        let mut ret = Vec::new();

        for fail in &self.fail_codes {
            for _ in ret.len()..fail.idx {
                ret.push(ExitCode::OK)
            }
            ret.push(fail.code)
        }
        for _ in ret.len()..self.batch_size {
            ret.push(ExitCode::OK)
        };
        ret
    }
}

impl Cbor for BatchReturn {}


pub struct BatchReturnGen {
    idx: usize,
    fail_codes: Vec<FailCode>,
}

impl BatchReturnGen {
    pub fn new() -> Self {
        BatchReturnGen { idx: 0, fail_codes: Vec::new() }
    }

    pub fn add_success(&mut self) {
        self.idx+=1;
    }

    pub fn add_fail(&mut self, code: ExitCode) {
        self.fail_codes.push(FailCode{idx: self.idx, code});
        self.idx += 1;
    }

    pub fn gen(&self) -> BatchReturn {
        BatchReturn {
            batch_size: self.idx,
            fail_codes: self.fail_codes.clone(),
        }
    }
}