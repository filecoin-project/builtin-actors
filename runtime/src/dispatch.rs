use castaway::cast;
use std::marker::PhantomData;

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use serde::{Deserialize, Serialize};

use crate::ActorError;

/// Implement actor method dispatch:
///
/// ```ignore
/// type Actor;
/// #[derive(FromPrimitive)]
/// #[repr(u64)]
/// enum Method {
///     Constructor = 1,
/// }
/// impl ActorCode for Actor {
///     type Methods = Method;
///     actor_dispatch! {
///         Constructor => constructor,
///     }
/// }
/// ```
#[macro_export]
macro_rules! actor_dispatch {
    ($($method:ident => $func:ident,)*) => {
        fn invoke_method<RT>(
            rt: &mut RT,
            method: MethodNum,
            args: Option<fvm_ipld_encoding::ipld_block::IpldBlock>,
        ) -> Result<RawBytes, ActorError>
        where
            RT: Runtime,
        {
            match FromPrimitive::from_u64(method) {
                $(Some(Self::Methods::$method) => $crate::dispatch(rt, Self::$func, &args),)*
                None => Err(actor_error!(unhandled_message; "invalid method: {}", method)),
            }
        }
    };
}

pub trait Dispatch<'de, RT> {
    fn call(self, rt: &mut RT, args: &'de Option<IpldBlock>) -> Result<RawBytes, ActorError>;
}

pub struct Dispatcher<F, A> {
    func: F,
    _marker: PhantomData<fn(A)>,
}

impl<F, A> Dispatcher<F, A> {
    // TODO: drop this allow
    #[allow(dead_code)]
    const fn new(f: F) -> Self {
        Dispatcher { func: f, _marker: PhantomData }
    }
}

/// Dispatch an actor method, deserializing the input and re-serializing the output.
///
/// This method automatically handles:
///
/// - Dispatching None/Some based on the number of parameters (0/1).
/// - Returning None if the return type is `Result<(), ActorError>`.
// TODO: drop this allow
#[allow(dead_code)]
pub fn dispatch<'de, F, A, RT>(
    rt: &mut RT,
    func: F,
    arg: &'de Option<IpldBlock>,
) -> Result<RawBytes, ActorError>
where
    Dispatcher<F, A>: Dispatch<'de, RT>,
{
    Dispatcher::new(func).call(rt, arg)
}

fn maybe_into_block<T: Serialize>(v: T) -> Result<RawBytes, ActorError> {
    if cast!(&v, &()).is_ok() {
        Ok(RawBytes::default())
    } else {
        Ok(RawBytes::serialize(&v)?)
    }
}

impl<'de, F, R, RT> Dispatch<'de, RT> for Dispatcher<F, ()>
where
    F: FnOnce(&mut RT) -> Result<R, ActorError>,
    R: Serialize,
{
    fn call(self, rt: &mut RT, args: &'de Option<IpldBlock>) -> Result<RawBytes, ActorError> {
        match args {
            None => maybe_into_block((self.func)(rt)?),
            Some(_) => Err(ActorError::illegal_argument("method expects no arguments".into())),
        }
    }
}

impl<'de, F, A, R, RT> Dispatch<'de, RT> for Dispatcher<F, (A,)>
where
    F: FnOnce(&mut RT, A) -> Result<R, ActorError>,
    A: Deserialize<'de>,
    R: Serialize,
{
    fn call(self, rt: &mut RT, args: &'de Option<IpldBlock>) -> Result<RawBytes, ActorError> {
        match args {
            None => Err(ActorError::illegal_argument("method expects arguments".into())),
            Some(arg) => maybe_into_block((self.func)(rt, arg.deserialize()?)?),
        }
    }
}

#[test]
fn test_dispatch() {
    use crate::ActorError;
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct SomeArgs {
        foo: String,
    }

    trait Runtime {}
    struct MockRuntime;
    impl Runtime for MockRuntime {}

    fn with_arg(_: &mut impl Runtime, foo: SomeArgs) -> Result<(), ActorError> {
        assert_eq!(foo.foo, "foo");
        Ok(())
    }

    fn with_arg_ret(_: &mut impl Runtime, foo: SomeArgs) -> Result<SomeArgs, ActorError> {
        Ok(foo)
    }

    fn without_arg(_: &mut impl Runtime) -> Result<(), ActorError> {
        Ok(())
    }

    let mut rt = MockRuntime;
    let arg = IpldBlock::serialize_cbor(&SomeArgs { foo: "foo".into() })
        .expect("failed to serialize arguments");

    // Correct dispatch
    assert!(dispatch(&mut rt, with_arg, &arg).expect("failed to dispatch").is_empty());
    assert!(dispatch(&mut rt, without_arg, &None).expect("failed to dispatch").is_empty());
    assert_eq!(
        dispatch(&mut rt, with_arg_ret, &arg).expect("failed to dispatch").to_vec(),
        arg.clone().unwrap().data
    );

    // Incorrect dispatch
    let _ = dispatch(&mut rt, with_arg, &None).expect_err("should have required an argument");
    let _ = dispatch(&mut rt, without_arg, &arg).expect_err("should have required an argument");
}
