use castaway::cast;
use fvm_ipld_encoding::CBOR;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use fvm_ipld_encoding::ipld_block::IpldBlock;
use serde::{de::DeserializeOwned, Serialize};

use crate::ActorError;

pub struct WithCodec<T, const CODEC: u64>(pub T);

impl<T, const CODEC: u64> Deref for WithCodec<T, CODEC> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, const CODEC: u64> DerefMut for WithCodec<T, CODEC> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T, const CODEC: u64> From<T> for WithCodec<T, CODEC> {
    fn from(value: T) -> Self {
        WithCodec(value)
    }
}

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
    ($($(#[$m:meta])* $(_)? $($method:ident)|* => $func:ident $([$tag:ident])?,)*) => {
        fn invoke_method<RT>(
            rt: &RT,
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
                  $crate::actor_dispatch!(@pattern $($method)|*) =>
                  $crate::actor_dispatch!(@target rt args method $func $($tag)?),)*
                None => Err(actor_error!(unhandled_message; "invalid method: {}", method)),
            }
        }
    };
    (@pattern) => {
        None
    };
    (@pattern $($method:ident)|+) => {
        Some($(Self::Methods::$method)|+)
    };
    (@target $rt:ident $args:ident $method:ident $func:ident default_params) => {{
        $crate::dispatch_default($rt, Self::$func, $args)
    }};
    (@target $rt:ident $args:ident $method:ident $func:ident) => {
        $crate::dispatch($rt, $method, Self::$func, $args)
    };
}

#[macro_export]
macro_rules! actor_dispatch_unrestricted {
    ($($(#[$m:meta])* $(_)? $($method:ident)|* => $func:ident $([$tag:ident])?,)*) => {
        fn invoke_method<RT>(
            rt: &RT,
            method: fvm_shared::MethodNum,
            args: Option<fvm_ipld_encoding::ipld_block::IpldBlock>,
        ) -> Result<Option<fvm_ipld_encoding::ipld_block::IpldBlock>, $crate::ActorError>
        where
            RT: $crate::runtime::Runtime,
            RT::Blockstore: Clone,
        {
            match <Self::Methods as num_traits::FromPrimitive>::from_u64(method) {
                $($(#[$m])*
                  $crate::actor_dispatch!(@pattern $($method)|*) =>
                  $crate::actor_dispatch!(@target rt args method $func $($tag)?),)*
                None => Err(actor_error!(unhandled_message; "invalid method: {}", method)),
            }
        }
    };
    (@pattern) => {
        None
    };
    (@pattern $($method:ident)|+) => {
        Some($(Self::Methods::$method)|+)
    };
    (@target $rt:ident $args:ident $method:ident $func:ident raw) => {
        Self::$func($rt, $method, $args)
    };
    (@target $rt:ident $args:ident $method:ident $func:ident default_params) => {{
        $crate::dispatch_default($rt, Self::$func, &$args)
    }};
    (@target $rt:ident $args:ident $method:ident $func:ident) => {
        $crate::dispatch($rt, Self::$func, &$args)
    };
}

pub trait Dispatch<RT> {
    fn call(
        self,
        rt: &RT,
        method: u64,
        args: Option<IpldBlock>,
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
#[doc(hidden)]
pub fn dispatch<F, A, RT>(
    rt: &RT,
    method: u64,
    func: F,
    arg: Option<IpldBlock>,
) -> Result<Option<IpldBlock>, ActorError>
where
    Dispatcher<F, A>: Dispatch<RT>,
{
    Dispatcher::new(func).call(rt, method, arg)
}

/// Like [`dispatch`], but pass the default value to if there are no parameters.
#[doc(hidden)]
pub fn dispatch_default<F, A, R, RT>(
    rt: &RT,
    func: F,
    arg: Option<IpldBlock>,
) -> Result<Option<IpldBlock>, ActorError>
where
    F: FnOnce(&RT, A) -> Result<R, ActorError>,
    A: DeserializeOwned + Default,
    R: Serialize,
{
    let arg = arg.as_ref().map(|b| b.deserialize()).transpose()?.unwrap_or_default();
    maybe_into_block((func)(rt, arg)?, CBOR) // TOOD
}

/// Convert the passed value into an IPLD Block, or None if it's `()`.
fn maybe_into_block<T: Serialize>(v: T, codec: u64) -> Result<Option<IpldBlock>, ActorError> {
    if cast!(&v, &()).is_ok() {
        Ok(None)
    } else {
        Ok(Some(IpldBlock::serialize(codec, &v)?))
    }
}

impl<F, RT> Dispatch<RT> for Dispatcher<F, ()>
where
    F: FnOnce(&RT, u64, Option<IpldBlock>) -> Result<Option<IpldBlock>, ActorError>,
{
    fn call(
        self,
        rt: &RT,
        method: u64,
        args: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        (self.func)(rt, method, args)
    }
}

impl<F, R, RT> Dispatch<RT> for Dispatcher<F, (R,)>
where
    F: FnOnce(&RT) -> Result<R, ActorError>,
    R: Serialize,
{
    fn call(
        self,
        rt: &RT,
        _method: u64,
        args: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        match args {
            None => maybe_into_block((self.func)(rt)?, CBOR),
            Some(_) => Err(ActorError::illegal_argument("method expects no arguments".into())),
        }
    }
}

impl<F, R, RT, const CODEC: u64> Dispatch<RT> for Dispatcher<F, (WithCodec<R, CODEC>,)>
where
    F: FnOnce(&RT) -> Result<WithCodec<R, CODEC>, ActorError>,
    R: Serialize,
{
    fn call(
        self,
        rt: &RT,
        _method: u64,
        args: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        match args {
            None => maybe_into_block((self.func)(rt)?.0, CODEC),
            Some(_) => Err(ActorError::illegal_argument("method expects arguments".into())),
        }
    }
}

impl<F, A, R, RT> Dispatch<RT> for Dispatcher<F, (A, R)>
where
    F: FnOnce(&RT, A) -> Result<R, ActorError>,
    A: DeserializeOwned,
    R: Serialize,
{
    fn call(
        self,
        rt: &RT,
        _method: u64,
        args: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        match args {
            None => Err(ActorError::illegal_argument("method expects arguments".into())),
            Some(arg) => maybe_into_block((self.func)(rt, arg.deserialize()?)?, CBOR),
        }
    }
}

impl<F, A, R, RT, const CODEC: u64> Dispatch<RT> for Dispatcher<F, (WithCodec<A, CODEC>, R)>
where
    F: FnOnce(&RT, WithCodec<A, CODEC>) -> Result<R, ActorError>,
    A: DeserializeOwned,
    R: Serialize,
{
    fn call(
        self,
        rt: &RT,
        _method: u64,
        args: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        match args {
            None => Err(ActorError::illegal_argument("method expects arguments".into())),
            Some(arg) if arg.codec != CODEC => {
                Err(ActorError::illegal_argument("method expects arguments".into()))
            }
            Some(arg) => maybe_into_block((self.func)(rt, WithCodec(arg.deserialize()?))?, CBOR),
        }
    }
}

impl<F, A, R, RT, const CODEC: u64> Dispatch<RT> for Dispatcher<F, (A, WithCodec<R, CODEC>)>
where
    F: FnOnce(&RT, A) -> Result<WithCodec<R, CODEC>, ActorError>,
    A: DeserializeOwned,
    R: Serialize,
{
    fn call(
        self,
        rt: &RT,
        _method: u64,
        args: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        match args {
            None => Err(ActorError::illegal_argument("method expects arguments".into())),
            Some(arg) => maybe_into_block((self.func)(rt, arg.deserialize()?)?.0, CODEC),
        }
    }
}

impl<F, A, R, RT, const CODEC: u64> Dispatch<RT>
    for Dispatcher<F, (WithCodec<A, CODEC>, WithCodec<R, CODEC>)>
where
    F: FnOnce(&RT, WithCodec<A, CODEC>) -> Result<WithCodec<R, CODEC>, ActorError>,
    A: DeserializeOwned,
    R: Serialize,
{
    fn call(
        self,
        rt: &RT,
        _method: u64,
        args: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        match args {
            None => Err(ActorError::illegal_argument("method expects arguments".into())),
            Some(arg) if arg.codec != CODEC => {
                Err(ActorError::illegal_argument("method expects arguments".into()))
            }
            Some(arg) => maybe_into_block((self.func)(rt, WithCodec(arg.deserialize()?))?.0, CODEC),
        }
    }
}

#[test]
fn test_dispatch() {
    use crate::ActorError;
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Serialize, Deserialize)]
    struct SomeArgs {
        foo: String,
    }

    trait Runtime {}
    struct MockRuntime;
    impl Runtime for MockRuntime {}

    fn with_arg(_: &impl Runtime, foo: SomeArgs) -> Result<(), ActorError> {
        assert_eq!(foo.foo, "foo");
        Ok(())
    }

    fn with_arg_ret(_: &impl Runtime, foo: SomeArgs) -> Result<SomeArgs, ActorError> {
        Ok(foo)
    }

    fn without_arg(_: &impl Runtime) -> Result<(), ActorError> {
        Ok(())
    }

    let rt = MockRuntime;
    let arg = IpldBlock::serialize_cbor(&SomeArgs { foo: "foo".into() })
        .expect("failed to serialize arguments");

    // Correct dispatch
    assert!(dispatch(&rt, 1, with_arg, arg.clone()).expect("failed to dispatch").is_none());
    assert!(dispatch(&rt, 1, without_arg, None).expect("failed to dispatch").is_none());
    assert_eq!(dispatch(&rt, 1, with_arg_ret, arg.clone()).expect("failed to dispatch"), arg);

    // Incorrect dispatch
    let _ = dispatch(&rt, 1, with_arg, None).expect_err("should have required an argument");
    let _ = dispatch(&rt, 1, without_arg, arg).expect_err("should have required an argument");
}
