use castaway::cast;
use std::marker::PhantomData;

use fvm_ipld_encoding::ipld_block::IpldBlock;
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
    ($($(#[$m:meta])* $(_)? $($method:ident)? => $func:ident $([$tag:ident])?,)*) => {
        fn invoke_method<RT>(
            rt: &mut RT,
            method: fvm_shared::MethodNum,
            args: Option<fvm_ipld_encoding::ipld_block::IpldBlock>,
        ) -> Result<Option<fvm_ipld_encoding::ipld_block::IpldBlock>, $crate::ActorError>
        where
            RT: $crate::runtime::Runtime,
            RT::Blockstore: Clone,
        {
            $crate::builtin::shared::restrict_internal_api(rt, method)?;
            match <Self::Methods as num_traits::FromPrimitive>::from_u64(method) {
                $($(#[$m])*
                  $crate::actor_dispatch!(@pattern $($method)?) =>
                  $crate::actor_dispatch!(@target rt args method $func $($tag)?),)*
                None => Err(actor_error!(unhandled_message; "invalid method: {}", method)),
            }
        }
    };
    (@pattern) => {
        None
    };
    (@pattern $method:ident) => {
        Some(Self::Methods::$method)
    };
    (@target $rt:ident $args:ident $method:ident $func:ident raw) => {
        Self::$func($rt, $method, $args)
    };
    (@target $rt:ident $args:ident $method:ident $func:ident) => {
        $crate::dispatch($rt, Self::$func, &$args)
    };
}

#[macro_export]
macro_rules! actor_dispatch_unrestricted {
    ($($(#[$m:meta])* $(_)? $($method:ident)? => $func:ident $([$tag:ident])?,)*) => {
        fn invoke_method<RT>(
            rt: &mut RT,
            method: fvm_shared::MethodNum,
            args: Option<fvm_ipld_encoding::ipld_block::IpldBlock>,
        ) -> Result<Option<fvm_ipld_encoding::ipld_block::IpldBlock>, $crate::ActorError>
        where
            RT: $crate::runtime::Runtime,
            RT::Blockstore: Clone,
        {
            match <Self::Methods as num_traits::FromPrimitive>::from_u64(method) {
                $($(#[$m])*
                  $crate::actor_dispatch!(@pattern $($method)?) =>
                  $crate::actor_dispatch!(@target rt args method $func $($tag)?),)*
                None => Err(actor_error!(unhandled_message; "invalid method: {}", method)),
            }
        }
    };
    (@pattern) => {
        None
    };
    (@pattern $method:ident) => {
        Some(Self::Methods::$method)
    };
    (@target $rt:ident $args:ident $method:ident $func:ident raw) => {
        Self::$func($rt, $method, $args)
    };
    (@target $rt:ident $args:ident $method:ident $func:ident) => {
        $crate::dispatch($rt, Self::$func, &$args)
    };
}

pub trait Dispatch<'de, RT> {
    fn call(
        self,
        rt: &mut RT,
        args: &'de Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError>;
}

pub struct Dispatcher<F, A> {
    func: F,
    _marker: PhantomData<fn(A)>,
}

impl<F, A> Dispatcher<F, A> {
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
pub fn dispatch<'de, F, A, RT>(
    rt: &mut RT,
    func: F,
    arg: &'de Option<IpldBlock>,
) -> Result<Option<IpldBlock>, ActorError>
where
    Dispatcher<F, A>: Dispatch<'de, RT>,
{
    Dispatcher::new(func).call(rt, arg)
}

/// Convert the passed value into an IPLD Block, or None if it's `()`.
fn maybe_into_block<T: Serialize>(v: T) -> Result<Option<IpldBlock>, ActorError> {
    if cast!(&v, &()).is_ok() {
        Ok(None)
    } else {
        Ok(IpldBlock::serialize_cbor(&v)?)
    }
}

impl<'de, F, R, RT> Dispatch<'de, RT> for Dispatcher<F, ()>
where
    F: FnOnce(&mut RT) -> Result<R, ActorError>,
    R: Serialize,
{
    fn call(
        self,
        rt: &mut RT,
        args: &'de Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
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
    fn call(
        self,
        rt: &mut RT,
        args: &'de Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
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
    assert!(dispatch(&mut rt, with_arg, &arg).expect("failed to dispatch").is_none());
    assert!(dispatch(&mut rt, without_arg, &None).expect("failed to dispatch").is_none());
    assert_eq!(dispatch(&mut rt, with_arg_ret, &arg).expect("failed to dispatch"), arg);

    // Incorrect dispatch
    let _ = dispatch(&mut rt, with_arg, &None).expect_err("should have required an argument");
    let _ = dispatch(&mut rt, without_arg, &arg).expect_err("should have required an argument");
}
