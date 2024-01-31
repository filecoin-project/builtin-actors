use fvm_ipld_encoding::tuple::*;
use fvm_shared::error::ExitCode;
use std::fmt;

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

    pub fn of(codes: &[ExitCode]) -> Self {
        let mut gen = BatchReturnGen::new(codes.len());
        for code in codes {
            gen.add(*code);
        }
        gen.gen()
    }

    pub fn size(&self) -> usize {
        self.success_count as usize + self.fail_codes.len()
    }

    pub fn all_ok(&self) -> bool {
        self.fail_codes.is_empty()
    }

    /// Returns a vector of exit codes for each item (including successes).
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

    /// Returns a subset of items corresponding to the successful indices.
    /// Panics if `items` is not the same length as this batch return.
    pub fn successes<'i, T>(&self, items: &'i [T]) -> Vec<&'i T> {
        if items.len() != self.size() {
            panic!("items length {} does not match batch size {}", items.len(), self.size());
        }
        let mut ret = Vec::new();
        let mut fail_idx = 0;
        for (idx, item) in items.iter().enumerate() {
            if fail_idx < self.fail_codes.len() && idx == self.fail_codes[fail_idx].idx as usize {
                fail_idx += 1;
            } else {
                ret.push(item)
            }
        }
        ret
    }
}

impl fmt::Display for BatchReturn {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let succ_str = format!("Batch successes {} / {}", self.success_count, self.size());
        if self.all_ok() {
            return f.write_str(&succ_str);
        }
        let mut ret = format!("{}, Batch failing: [", succ_str);
        let mut strs = Vec::new();
        for fail in &self.fail_codes {
            strs.push(format!("code={} at idx={}", fail.code, fail.idx))
        }
        let fail_str = strs.join(", ");
        ret.push_str(&fail_str);
        ret.push(']');
        f.write_str(&ret)
    }
}

/// Computes a batch return that is the result of a sequence of batch returns
/// applied to the previous successful results.
/// Each batch's size() must be equal to the previous batch's success_count.
/// Any fail codes then override the prior stack's successful items,
/// indexed against only those successful items.
/// E.g. stack([OK, E1, OK, E2], [OK, E3], [E4]) => [E4, E1, E3, E2]
pub fn stack(batch_returns: &[BatchReturn]) -> BatchReturn {
    if batch_returns.is_empty() {
        return BatchReturn::empty();
    }
    let mut base = batch_returns[0].clone();
    for nxt in &batch_returns[1..] {
        assert_eq!(
            base.success_count as usize,
            nxt.size(),
            "can't stack batch of {} on batch with {} successes",
            nxt.size(),
            base.success_count
        );
        let mut base_fail = base.fail_codes.iter().peekable();
        let mut offset = 0;
        let new_fail_codes: Vec<_> = nxt
            .fail_codes
            .iter()
            .map(|nxt_fail| {
                while base_fail.next_if(|f| f.idx <= nxt_fail.idx + offset).is_some() {
                    offset += 1;
                }
                FailCode { idx: nxt_fail.idx + offset, code: nxt_fail.code }
            })
            .collect();
        base.fail_codes.extend(new_fail_codes);
        base.fail_codes.sort_by(|a, b| a.idx.cmp(&b.idx));
        base.success_count = nxt.success_count;
    }
    assert_eq!(base.size(), batch_returns[0].size());
    assert_eq!(base.success_count, batch_returns[batch_returns.len() - 1].success_count);
    base
}

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
        self.add_successes(1)
    }

    pub fn add_successes(&mut self, count: usize) -> &mut Self {
        self.success_count += count;
        self
    }

    pub fn add_fail(&mut self, code: ExitCode) -> &mut Self {
        self.fail_codes
            .push(FailCode { idx: (self.success_count + self.fail_codes.len()) as u32, code });
        self
    }

    pub fn add(&mut self, code: ExitCode) -> &mut Self {
        if code.is_success() {
            self.add_success()
        } else {
            self.add_fail(code)
        }
    }

    pub fn gen(self) -> BatchReturn {
        assert_eq!(self.expect_count, self.success_count + self.fail_codes.len(), "programmer error, mismatched batch size {} and processed count {} batch return must include success/fail for all inputs", self.expect_count, self.success_count + self.fail_codes.len());
        BatchReturn { success_count: self.success_count as u32, fail_codes: self.fail_codes }
    }
}

// Unit tests
#[cfg(test)]
mod test {
    use crate::util::batch_return::stack;
    use crate::{BatchReturn, FailCode};
    use fvm_shared::error::ExitCode;

    const OK: ExitCode = ExitCode::OK;
    const ERR1: ExitCode = ExitCode::USR_ILLEGAL_ARGUMENT;
    const ERR2: ExitCode = ExitCode::USR_NOT_FOUND;
    const ERR3: ExitCode = ExitCode::USR_FORBIDDEN;

    ///// Tests for stacking batch returns. /////

    #[test]
    fn test_stack_empty() {
        let batch_returns = vec![];
        let stacked = stack(&batch_returns);
        assert_eq!(stacked.success_count, 0);
        assert_eq!(Vec::<FailCode>::new(), stacked.fail_codes);
    }

    #[test]
    fn test_stack_single() {
        assert_stack(&[], &[]);
        assert_stack(&[OK], &[&[OK]]);
        assert_stack(&[ERR1], &[&[ERR1]]);
        assert_stack(&[ERR1, OK, ERR2], &[&[ERR1, OK, ERR2]]);
    }

    #[test]
    fn test_stack_overwrites() {
        assert_stack(&[OK], &[&[OK], &[OK]]);
        assert_stack(&[ERR1], &[&[OK], &[ERR1]]);

        assert_stack(&[OK, ERR1], &[&[OK, OK], &[OK, ERR1]]);
        assert_stack(&[ERR1, ERR2], &[&[OK, OK], &[ERR1, ERR2]]);
    }

    #[test]
    fn test_stack_offsets() {
        assert_stack(&[ERR1], &[&[ERR1], &[]]);
        assert_stack(&[ERR1, ERR2], &[&[ERR1, ERR2], &[]]);

        assert_stack(&[ERR2, ERR1], &[&[OK, ERR1], &[ERR2]]);
        assert_stack(&[ERR1, ERR2], &[&[ERR1, OK], &[ERR2]]);

        assert_stack(&[ERR2, ERR1], &[&[OK, OK], &[OK, ERR1], &[ERR2]]);
        assert_stack(&[ERR1, ERR2], &[&[OK, OK], &[ERR1, OK], &[ERR2]]);

        assert_stack(&[OK, ERR1, OK], &[&[OK, ERR1, OK], &[OK, OK]]);
        assert_stack(&[ERR2, ERR1, OK], &[&[OK, ERR1, OK], &[ERR2, OK]]);
        assert_stack(&[OK, ERR1, ERR2], &[&[OK, ERR1, OK], &[OK, ERR2]]);
        assert_stack(&[ERR1, ERR2, OK], &[&[ERR1, OK, OK], &[ERR2, OK]]);
        assert_stack(&[ERR1, OK, ERR2], &[&[ERR1, OK, OK], &[OK, ERR2]]);
        assert_stack(&[ERR3, ERR1, ERR2], &[&[OK, ERR1, OK], &[ERR3, ERR2]]);

        assert_stack(
            &[ERR1, ERR1, ERR1, ERR3, ERR2, ERR3],
            &[&[ERR1, ERR1, ERR1, OK, ERR2, OK], &[ERR3, ERR3]],
        );

        assert_stack(
            &[ERR1, ERR3, ERR2, OK, ERR1, ERR3],
            &[&[ERR1, OK, ERR2, OK, ERR1, OK], &[ERR3, OK, ERR3]],
        );

        assert_stack(
            &[ERR2, ERR1, OK, ERR3, ERR2],
            &[&[OK; 5], &[OK, ERR1, OK, OK, OK], &[ERR2, OK, OK, ERR2], &[OK, ERR3]],
        );
    }

    fn assert_stack(expected: &[ExitCode], stacked: &[&[ExitCode]]) {
        let expected = BatchReturn::of(expected);
        let batches: Vec<BatchReturn> = stacked.iter().map(|b| BatchReturn::of(b)).collect();
        let stacked = stack(&batches);
        assert_eq!(expected, stacked);
    }
}
