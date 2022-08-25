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
    pub success_count: usize,
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
        let batch_size = self.success_count + self.fail_codes.len();
        for _ in ret.len()..batch_size {
            ret.push(ExitCode::OK)
        };
        ret
    }
}

impl Cbor for BatchReturn {}


pub struct BatchReturnGen {
    success_count: usize,
    fail_codes: Vec<FailCode>,

    // gen will only work if it has processed all of the expected batch
    expect_count: usize,
}

impl BatchReturnGen {
    pub fn new(expect_count: usize) -> Self {
        BatchReturnGen { success_count: 0, fail_codes: Vec::new(), expect_count }
    }

    pub fn add_success(&mut self) {
        self.success_count+=1;
    }

    pub fn add_fail(&mut self, code: ExitCode) {
        self.fail_codes.push(FailCode{idx: self.success_count + self.fail_codes.len(), code});
    }

    pub fn gen(&self) -> BatchReturn {
        assert_eq!(self.expect_count, self.success_count + self.fail_codes.len(), "programmer error, mismatched batch size {} and processed coutn {} batch return must include success/fail for all inputs", self.expect_count, self.success_count + self.fail_codes.len());
        BatchReturn {
            success_count: self.success_count,
            fail_codes: self.fail_codes.clone(),
        }
    }
}