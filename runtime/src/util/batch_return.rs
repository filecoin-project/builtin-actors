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
    pub const fn empty() -> Self {
        Self { success_count: 0, fail_codes: Vec::new() }
    }

    pub const fn ok(n: u32) -> Self {
        Self { success_count: n, fail_codes: Vec::new() }
    }

    pub fn size(&self) -> usize {
        self.success_count as usize + self.fail_codes.len()
    }

    // Returns a vector of exit codes for each item (including successes).
    pub fn codes(&self) -> Vec<ExitCode> {
        let mut ret = Vec::new();

        for fail in &self.fail_codes {
            for _ in ret.len()..fail.idx as usize {
                ret.push(ExitCode::OK)
            }
            ret.push(fail.code)
        }
        for _ in ret.len()..self.size() {
            ret.push(ExitCode::OK)
        }
        ret
    }

    // Returns a subset of items corresponding to the successful indices.
    pub fn successes<T: Copy>(&self, items: &Vec<T>) -> Vec<T> {
        let mut ret = Vec::new();
        let mut fail_idx = 0;
        for idx in 0..self.size() {
            if fail_idx < self.fail_codes.len() && idx == self.fail_codes[fail_idx].idx as usize {
                fail_idx += 1;
            } else {
                ret.push(items[idx])
            }
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
