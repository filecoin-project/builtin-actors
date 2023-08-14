use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::{ActorID, MethodNum};

/// The result of an actor method invocation.
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum InvocationResult {
    CallReturn { return_value: Option<IpldBlock>, exit_code: ExitCode },
    CallError { reason: String, errno: ErrorNumber },
}

/// A trace of an actor method invocation.
#[derive(Clone, Debug)]
pub struct InvocationTrace {
    pub from: ActorID,
    pub to: Address,
    pub value: TokenAmount,
    pub method: MethodNum,
    pub params: Option<IpldBlock>,
    pub result: InvocationResult,
    pub subinvocations: Vec<InvocationTrace>,
}

/// An expectation for a method invocation trace.
/// Non-optional fields must always be specified, and are always checked against any trace.
/// Optional fields are ignored when checking the expectation against a trace.
// Future work:
// - Add mutator or factory methods to allow builder-style customisation of expectations.
// - Add a capture() option on value, params, ret etc to enable extraction of internal values
//   while matching with an invocation trace.
// - Make value mandatory (requires specifying the currently unknown ones).
// - Return a top-level ExpectInvocation from helpers like util::apply_ok to save caller
//   constructing it.
#[derive(Clone, Debug)]
pub struct ExpectInvocation {
    pub from: ActorID,
    pub to: Address,
    pub method: MethodNum,
    pub value: Option<TokenAmount>,
    pub params: Option<Option<IpldBlock>>,
    pub result: ExpectResult,
    pub subinvocs: Option<Vec<ExpectInvocation>>,
}

#[derive(Clone, Debug)]
pub enum ExpectResult {
    /// Separate from CallReturn so that matching on return_value is optional
    ExpectReturn {
        return_value: Option<Option<IpldBlock>>,
        exit_code: ExitCode,
    },
    ExpectError {
        reason: String,
        errno: ErrorNumber,
    },
}

impl ExpectResult {
    pub fn ok(return_value: Option<Option<IpldBlock>>) -> ExpectResult {
        Self::ExpectReturn { return_value, exit_code: ExitCode::OK }
    }
}

impl ExpectInvocation {
    /// Asserts that a trace matches this expectation, including subinvocations.
    pub fn matches(&self, invoc: &InvocationTrace) {
        let id = format!("[{}→{}:{}]", invoc.from, invoc.to, invoc.method);
        self.quick_match(invoc, String::new());

        match &self.result {
            ExpectResult::ExpectReturn {
                return_value: expected_return,
                exit_code: expected_code,
            } => {
                if let InvocationResult::CallReturn { return_value, exit_code } = &invoc.result {
                    if let Some(expected_return) = expected_return {
                        assert_eq!(
                            return_value, expected_return,
                            "{} unexpected return value: got: {:?}, expected: {:?}",
                            id, return_value, expected_return
                        );
                    }
                    assert_eq!(
                        exit_code, expected_code,
                        "{} unexpected exit code: got: {:?}, expected: {:?}",
                        id, exit_code, expected_code
                    );
                } else {
                    panic!(
                        "{} expected CallReturn but got CallError instead: {:?}",
                        id, invoc.result
                    );
                }
            }
            ExpectResult::ExpectError { reason: expected_reason, errno: expected_errno } => {
                if let InvocationResult::CallError { reason, errno } = &invoc.result {
                    assert_eq!(
                        reason, expected_reason,
                        "{} unexpected error reason: got: {:?}, expected: {:?}",
                        id, reason, expected_reason
                    );
                    assert_eq!(
                        errno, expected_errno,
                        "{} unexpected error code: got: {:?}, expected: {:?}",
                        id, errno, expected_errno
                    );
                } else {
                    panic!(
                        "{} expected CallError but got CallReturn instead: {:?}",
                        id, invoc.result
                    );
                }
            }
        };

        if let Some(v) = &self.value {
            assert_eq!(
                v, &invoc.value,
                "{} unexpected value: expected: {}, was: {} ",
                id, v, invoc.value
            );
        }
        if let Some(p) = &self.params {
            assert_eq!(
                p, &invoc.params,
                "{} unexpected params: expected: {:x?}, was: {:x?}",
                id, p, invoc.params
            );
        }
        if let Some(expect_subinvocs) = &self.subinvocs {
            let subinvocs = &invoc.subinvocations;

            let panic_str = format!(
                "unexpected subinvocs:\n expected: \n[\n{}]\n was:\n[\n{}]\n",
                self.fmt_expect_invocs(expect_subinvocs),
                self.fmt_invocs(subinvocs)
            );
            assert_eq!(subinvocs.len(), expect_subinvocs.len(), "{} {}", id, panic_str);

            for (i, invoc) in subinvocs.iter().enumerate() {
                let expect_invoc = expect_subinvocs.get(i).unwrap();
                // only try to match if required fields match
                expect_invoc.quick_match(invoc, panic_str.clone());
                expect_invoc.matches(invoc);
            }
        }
    }

    pub fn fmt_invocs(&self, invocs: &[InvocationTrace]) -> String {
        invocs
            .iter()
            .enumerate()
            .map(|(i, invoc)| format!("{}: [{}:{}],\n", i, invoc.to, invoc.method))
            .collect()
    }

    pub fn fmt_expect_invocs(&self, exs: &[ExpectInvocation]) -> String {
        exs.iter()
            .enumerate()
            .map(|(i, ex)| format!("{}: [{}:{}],\n", i, ex.to, ex.method))
            .collect()
    }

    pub fn quick_match(&self, invoc: &InvocationTrace, extra_msg: String) {
        let id = format!("[{}→{}:{}]", invoc.from, invoc.to, invoc.method);
        assert_eq!(
            self.from, invoc.from,
            "{} unexpected from addr: expected: {}, was: {} \n{}",
            id, self.from, invoc.from, extra_msg
        );
        assert_eq!(
            self.to, invoc.to,
            "{} unexpected to addr: expected: {}, was: {} \n{}",
            id, self.to, invoc.to, extra_msg
        );
        assert_eq!(
            self.method, invoc.method,
            "{} unexpected method: expected: {}, was: {} \n{}",
            id, self.method, invoc.from, extra_msg
        );
    }
}

impl Default for ExpectInvocation {
    // Defaults are mainly useful for ignoring optional fields with a ..Default::default() clause.
    // The addresses must generally be provided explicitly.
    // Defaults include successful exit code.
    fn default() -> Self {
        Self {
            from: 0,
            to: Address::new_id(0),
            method: 0,
            value: None,
            params: None,
            // by-default we expect a successful invocation
            result: ExpectResult::ExpectReturn { return_value: None, exit_code: ExitCode::OK },
            subinvocs: None,
        }
    }
}
