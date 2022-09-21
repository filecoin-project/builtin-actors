use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::error::ExitCode;

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq, Eq)]
pub struct FailCode {
    pub idx: u32,
    pub code: ExitCode,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, PartialEq, Eq, Debug)]
pub struct BatchReturn {
    // Total successes in batch
    pub success_count: u32,
    // Failure code and index for each failure in batch
    pub fail_codes: Vec<FailCode>,
}

impl BatchReturn {
    pub fn codes(&self) -> Vec<ExitCode> {
        let mut ret = Vec::new();

        for fail in &self.fail_codes {
            for _ in ret.len()..fail.idx as usize {
                ret.push(ExitCode::OK)
            }
            ret.push(fail.code)
        }
        let batch_size = self.success_count as usize + self.fail_codes.len();
        for _ in ret.len()..batch_size {
            ret.push(ExitCode::OK)
        }
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

    pub fn add_success(&mut self) -> &mut Self {
        self.success_count += 1;
        self
    }

    pub fn add_fail(&mut self, code: ExitCode) -> &mut Self {
        self.fail_codes
            .push(FailCode { idx: (self.success_count + self.fail_codes.len()) as u32, code });
        self
    }

    pub fn gen(&self) -> BatchReturn {
        assert_eq!(self.expect_count, self.success_count + self.fail_codes.len(), "programmer error, mismatched batch size {} and processed count {} batch return must include success/fail for all inputs", self.expect_count, self.success_count + self.fail_codes.len());
        BatchReturn {
            success_count: self.success_count as u32,
            fail_codes: self.fail_codes.clone(),
        }
    }
}
