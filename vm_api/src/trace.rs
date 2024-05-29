use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::event::ActorEvent;
use fvm_shared::{ActorID, MethodNum};

type ReturnValue = Option<IpldBlock>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmittedEvent {
    pub emitter: ActorID,
    pub event: ActorEvent,
}

/// A trace of an actor method invocation.
#[derive(Clone, Debug)]
pub struct InvocationTrace {
    pub from: ActorID,
    pub to: Address,
    pub value: TokenAmount,
    pub method: MethodNum,
    pub params: Option<IpldBlock>,
    /// error_number is set when an unexpected syscall error occurs
    pub error_number: Option<ErrorNumber>,
    // no need to check return_value or exit_code if error_number is set
    pub exit_code: ExitCode,
    pub return_value: ReturnValue,
    pub subinvocations: Vec<InvocationTrace>,
    pub events: Vec<EmittedEvent>,
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
    /// If error_number is set, exit_code and return_value are not checked
    pub error_number: Option<ErrorNumber>,
    pub exit_code: ExitCode,
    pub return_value: Option<ReturnValue>,
    pub subinvocs: Option<Vec<ExpectInvocation>>,
    pub events: Option<Vec<EmittedEvent>>,
}

impl ExpectInvocation {
    /// Asserts that a trace matches this expectation, including subinvocations.
    pub fn matches(&self, invoc: &InvocationTrace) {
        let id = format!("[{}→{}:{}]", invoc.from, invoc.to, invoc.method);
        self.quick_match(invoc, String::new());

        if self.error_number.is_some() && self.return_value.is_some() {
            panic!(
                "{} malformed expectation: expected error_number {} but also expected return_value",
                id,
                self.error_number.unwrap()
            );
        }

        if let Some(error_number) = &self.error_number {
            assert!(
                invoc.error_number.is_some(),
                "{} expected error_number: {}, was: None",
                id,
                error_number
            );
            assert_eq!(
                error_number,
                &invoc.error_number.unwrap(),
                "{} unexpected error_number: expected: {}, was: {}",
                id,
                error_number,
                invoc.error_number.unwrap()
            );
        } else {
            assert_eq!(
                self.exit_code, invoc.exit_code,
                "{} unexpected exit_code: expected: {}, was: {}",
                id, self.exit_code, invoc.exit_code
            );

            if let Some(v) = &self.return_value {
                assert_eq!(
                    v, &invoc.return_value,
                    "{} unexpected return_value: expected: {:?}, was: {:?}",
                    id, v, invoc.return_value
                );
            }
        }

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

        // match emitted events
        if let Some(expected_events) = &self.events {
            let emitted_events = &invoc.events;
            assert_eq!(
                emitted_events.len(),
                expected_events.len(),
                "{} {} emitted={}, expected={}, {:?}, {:?}",
                id,
                "length of expected and emitted events do not match",
                emitted_events.len(),
                expected_events.len(),
                emitted_events,
                expected_events
            );

            // use the zip method to iterate over the emitted events and expected_events
            // vectors at the same time
            for (emitted, expected) in emitted_events.iter().zip(expected_events.iter()) {
                // only try to match if required fields match
                assert_eq!(*emitted, *expected);
            }
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
        invocs.iter().enumerate().fold(String::new(), |mut s, (i, invoc)| {
            use std::fmt::Write;
            let _ = writeln!(s, "{}: [{}:{}],", i, invoc.to, invoc.method);
            s
        })
    }

    pub fn fmt_expect_invocs(&self, exs: &[ExpectInvocation]) -> String {
        exs.iter().enumerate().fold(String::new(), |mut s, (i, ex)| {
            use std::fmt::Write;
            let _ = writeln!(s, "{}: [{}:{}],", i, ex.to, ex.method);
            s
        })
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
            id, self.method, invoc.method, extra_msg
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
            error_number: None,
            exit_code: ExitCode::OK,
            return_value: None,
            subinvocs: None,
            events: None,
        }
    }
}
