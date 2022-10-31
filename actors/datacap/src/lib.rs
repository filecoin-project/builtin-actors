use std::marker::PhantomData;

use frc46_token::token::types::{
    BurnFromParams, BurnFromReturn, BurnParams, BurnReturn, DecreaseAllowanceParams,
    GetAllowanceParams, IncreaseAllowanceParams, MintReturn, RevokeAllowanceParams,
    TransferFromParams, TransferFromReturn, TransferParams, TransferReturn,
};
use frc46_token::token::{Token, TokenError, TOKEN_PRECISION};
use fvm_actor_utils::messaging::{Messaging, MessagingError};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::receipt::Receipt;
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR, METHOD_SEND};
use lazy_static::lazy_static;
use log::info;
use num_derive::FromPrimitive;
use num_traits::{FromPrimitive, Zero};

use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_error, cbor, ActorContext, ActorError, AsActorError, SYSTEM_ACTOR_ADDR,
};

pub use self::state::State;
pub use self::types::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

mod state;
pub mod testing;
mod types;

pub const DATACAP_GRANULARITY: u64 = TOKEN_PRECISION as u64;

lazy_static! {
    // > 800 EiB
    static ref INFINITE_ALLOWANCE: TokenAmount = TokenAmount::from_atto(
        BigInt::from(TOKEN_PRECISION)
            * BigInt::from(1_000_000_000_000_000_000_000_i128)
    );
}
/// Static method numbers for builtin-actor private dispatch.
/// The methods are also expected to be exposed via FRC-XXXX standard calling convention,
/// with numbers determined by name.
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    // Non-standard.
    Mint = 2,
    Destroy = 3,
    // Static method numbers for token standard methods, for private use.
    Name = 10,
    Symbol = 11,
    TotalSupply = 12,
    BalanceOf = 13,
    Transfer = 14,
    TransferFrom = 15,
    IncreaseAllowance = 16,
    DecreaseAllowance = 17,
    RevokeAllowance = 18,
    Burn = 19,
    BurnFrom = 20,
    Allowance = 21,
}

pub struct Actor;

impl Actor {
    /// Constructor for DataCap Actor
    pub fn constructor<BS, RT>(rt: &mut RT, governor: Address) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        // Confirm the governor address is an ID.
        rt.resolve_address(&governor)
            .ok_or_else(|| actor_error!(illegal_argument, "failed to resolve governor address"))?;

        let st = State::new(rt.store(), governor).context("failed to create datacap state")?;
        rt.create(&st)?;
        Ok(())
    }

    pub fn name<BS, RT>(rt: &mut RT) -> Result<String, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        Ok("DataCap".to_string())
    }

    pub fn symbol<BS, RT>(rt: &mut RT) -> Result<String, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        Ok("DCAP".to_string())
    }

    pub fn total_supply<BS, RT>(rt: &mut RT, _: ()) -> Result<TokenAmount, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let mut st: State = rt.state()?;
        let msg = Messenger { rt, dummy: Default::default() };
        let token = as_token(&mut st, &msg);
        Ok(token.total_supply())
    }

    pub fn balance_of<BS, RT>(rt: &mut RT, address: Address) -> Result<TokenAmount, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        // NOTE: mutability and method caller here are awkward for a read-only call
        rt.validate_immediate_caller_accept_any()?;
        let mut st: State = rt.state()?;
        let msg = Messenger { rt, dummy: Default::default() };
        let token = as_token(&mut st, &msg);
        token.balance_of(&address).actor_result()
    }

    pub fn allowance<BS, RT>(
        rt: &mut RT,
        params: GetAllowanceParams,
    ) -> Result<TokenAmount, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let mut st: State = rt.state()?;
        let msg = Messenger { rt, dummy: Default::default() };
        let token = as_token(&mut st, &msg);
        token.allowance(&params.owner, &params.operator).actor_result()
    }

    /// Mints new data cap tokens for an address (a verified client).
    /// Simultaneously sets the allowance for any specified operators to effectively infinite.
    /// Only the governor can call this method.
    /// This method is not part of the fungible token standard.
    pub fn mint<BS, RT>(rt: &mut RT, params: MintParams) -> Result<MintReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        let mut hook = rt
            .transaction(|st: &mut State, rt| {
                // Only the governor can mint datacap tokens.
                rt.validate_immediate_caller_is(std::iter::once(&st.governor))?;
                let operator = st.governor;

                let msg = Messenger { rt, dummy: Default::default() };
                let mut token = as_token(st, &msg);
                // Mint tokens "from" the operator to the beneficiary.
                let ret = token
                    .mint(
                        &operator,
                        &params.to,
                        &params.amount,
                        RawBytes::default(),
                        RawBytes::default(),
                    )
                    .actor_result();

                // Set allowance for any specified operators.
                for delegate in &params.operators {
                    token
                        .set_allowance(&params.to, delegate, &INFINITE_ALLOWANCE)
                        .actor_result()?;
                }

                ret
            })
            .context("state transaction failed")?;

        let mut st: State = rt.state()?;
        let msg = Messenger { rt, dummy: Default::default() };
        let intermediate = hook.call(&&msg).actor_result()?;
        as_token(&mut st, &msg).mint_return(intermediate).actor_result()
    }

    /// Destroys data cap tokens for an address (a verified client).
    /// Only the governor can call this method.
    /// This method is not part of the fungible token standard, and is named distinctly from
    /// "burn" to reflect that distinction.
    pub fn destroy<BS, RT>(rt: &mut RT, params: DestroyParams) -> Result<BurnReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.transaction(|st: &mut State, rt| {
            // Only the governor can destroy datacap tokens on behalf of a holder.
            rt.validate_immediate_caller_is(std::iter::once(&st.governor))?;

            let msg = Messenger { rt, dummy: Default::default() };
            let mut token = as_token(st, &msg);
            // Burn tokens as if the holder had invoked burn() themselves.
            // The governor doesn't need an allowance.
            token.burn(&params.owner, &params.amount).actor_result()
        })
        .context("state transaction failed")
    }

    /// Transfers data cap tokens to an address.
    /// Data cap tokens are not generally transferable.
    /// Succeeds if the to or from address is the governor, otherwise always fails.
    pub fn transfer<BS, RT>(
        rt: &mut RT,
        params: TransferParams,
    ) -> Result<TransferReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let operator = &rt.message().caller();
        let from = operator;
        // Resolve to address for comparison with governor address.
        let to = rt
            .resolve_address(&params.to)
            .context_code(ExitCode::USR_ILLEGAL_ARGUMENT, "to must be ID address")?;
        let to_address = Address::new_id(to);

        let mut hook = rt
            .transaction(|st: &mut State, rt| {
                let allowed = to_address == st.governor || *from == st.governor;
                if !allowed {
                    return Err(actor_error!(
                        forbidden,
                        "transfer not allowed from {} to {} (governor is {})",
                        from,
                        to_address,
                        st.governor
                    ));
                }

                let msg = Messenger { rt, dummy: Default::default() };
                let mut token = as_token(st, &msg);
                token
                    .transfer(
                        from,
                        &to_address,
                        &params.amount,
                        params.operator_data.clone(),
                        RawBytes::default(),
                    )
                    .actor_result()
            })
            .context("state transaction failed")?;

        let mut st: State = rt.state()?;
        let msg = Messenger { rt, dummy: Default::default() };
        let intermediate = hook.call(&&msg).actor_result()?;
        as_token(&mut st, &msg).transfer_return(intermediate).actor_result()
    }

    /// Transfers data cap tokens between addresses.
    /// Data cap tokens are not generally transferable between addresses.
    /// Succeeds if the to address is the governor, otherwise always fails.
    pub fn transfer_from<BS, RT>(
        rt: &mut RT,
        params: TransferFromParams,
    ) -> Result<TransferFromReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let operator = rt.message().caller();
        let from = params.from;
        // Resolve to address for comparison with governor.
        let to = rt
            .resolve_address(&params.to)
            .context_code(ExitCode::USR_ILLEGAL_ARGUMENT, "to must be an ID address")?;
        let to_address = Address::new_id(to);

        let mut hook = rt
            .transaction(|st: &mut State, rt| {
                let allowed = to_address == st.governor;
                if !allowed {
                    return Err(actor_error!(
                        forbidden,
                        "transfer not allowed from {} to {} (governor is {})",
                        from,
                        to_address,
                        st.governor
                    ));
                }

                let msg = Messenger { rt, dummy: Default::default() };
                let mut token = as_token(st, &msg);
                token
                    .transfer_from(
                        &operator,
                        &from,
                        &to_address,
                        &params.amount,
                        params.operator_data.clone(),
                        RawBytes::default(),
                    )
                    .actor_result()
            })
            .context("state transaction failed")?;

        let mut st: State = rt.state()?;
        let msg = Messenger { rt, dummy: Default::default() };
        let intermediate = hook.call(&&msg).actor_result()?;
        as_token(&mut st, &msg).transfer_from_return(intermediate).actor_result()
    }

    pub fn increase_allowance<BS, RT>(
        rt: &mut RT,
        params: IncreaseAllowanceParams,
    ) -> Result<TokenAmount, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let owner = rt.message().caller();
        let operator = params.operator;

        rt.transaction(|st: &mut State, rt| {
            let msg = Messenger { rt, dummy: Default::default() };
            let mut token = as_token(st, &msg);
            token.increase_allowance(&owner, &operator, &params.increase).actor_result()
        })
        .context("state transaction failed")
    }

    pub fn decrease_allowance<BS, RT>(
        rt: &mut RT,
        params: DecreaseAllowanceParams,
    ) -> Result<TokenAmount, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let owner = &rt.message().caller();
        let operator = &params.operator;

        rt.transaction(|st: &mut State, rt| {
            let msg = Messenger { rt, dummy: Default::default() };
            let mut token = as_token(st, &msg);
            token.decrease_allowance(owner, operator, &params.decrease).actor_result()
        })
        .context("state transaction failed")
    }

    pub fn revoke_allowance<BS, RT>(
        rt: &mut RT,
        params: RevokeAllowanceParams,
    ) -> Result<TokenAmount, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let owner = &rt.message().caller();
        let operator = &params.operator;

        rt.transaction(|st: &mut State, rt| {
            let msg = Messenger { rt, dummy: Default::default() };
            let mut token = as_token(st, &msg);
            token.revoke_allowance(owner, operator).actor_result()
        })
        .context("state transaction failed")
    }

    pub fn burn<BS, RT>(rt: &mut RT, params: BurnParams) -> Result<BurnReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let owner = &rt.message().caller();

        rt.transaction(|st: &mut State, rt| {
            let msg = Messenger { rt, dummy: Default::default() };
            let mut token = as_token(st, &msg);
            token.burn(owner, &params.amount).actor_result()
        })
        .context("state transaction failed")
    }

    pub fn burn_from<BS, RT>(
        rt: &mut RT,
        params: BurnFromParams,
    ) -> Result<BurnFromReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let operator = &rt.message().caller();
        let owner = &params.owner;

        rt.transaction(|st: &mut State, rt| {
            let msg = Messenger { rt, dummy: Default::default() };
            let mut token = as_token(st, &msg);
            token.burn_from(operator, owner, &params.amount).actor_result()
        })
        .context("state transaction failed")
    }
}

/// Implementation of the token library's messenger trait in terms of the built-in actors'
/// runtime library.
struct Messenger<'a, BS, RT>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    rt: &'a mut RT,
    // Without this, Rust complains the BS parameter is unused.
    // This might be solved better by having BS as an associated type of the Runtime trait.
    dummy: PhantomData<BS>,
}

// The trait is implemented for Messenger _reference_ since the mutable ref to rt has been
// moved into it and we can't move the messenger instance since callers need to get at the
// rt that's now in there.
impl<'a, BS, RT> Messaging for &Messenger<'a, BS, RT>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    fn actor_id(&self) -> ActorID {
        // The Runtime unhelpfully wraps receiver in an address, while the Messaging trait
        // is closer to the syscall interface.
        self.rt.message().receiver().id().unwrap()
    }

    // This never returns an Err.  However we could return an error if the
    // Runtime send method passed through the underlying syscall error
    // instead of hiding it behind a client-side chosen exit code.
    fn send(
        &self,
        to: &Address,
        method: MethodNum,
        params: &RawBytes,
        value: &TokenAmount,
    ) -> fvm_actor_utils::messaging::Result<Receipt> {
        // The Runtime discards some of the information from the syscall :-(
        let fake_gas_used = 0;
        let res = self.rt.send(to, method, params.clone(), value.clone());

        let rec = match res {
            Ok(bytes) => {
                Receipt { exit_code: ExitCode::OK, return_data: bytes, gas_used: fake_gas_used }
            }
            Err(ae) => {
                info!("datacap messenger failed: {}", ae.msg());
                Receipt {
                    exit_code: ae.exit_code(),
                    return_data: RawBytes::default(),
                    gas_used: fake_gas_used,
                }
            }
        };
        Ok(rec)
    }

    fn resolve_id(&self, address: &Address) -> fvm_actor_utils::messaging::Result<ActorID> {
        self.rt.resolve_address(address).ok_or(MessagingError::AddressNotInitialized(*address))
    }

    fn initialize_account(&self, address: &Address) -> fvm_actor_utils::messaging::Result<ActorID> {
        let fake_syscall_error_number = ErrorNumber::NotFound;
        if self.rt.send(address, METHOD_SEND, Default::default(), TokenAmount::zero()).is_err() {
            return Err(MessagingError::Syscall(fake_syscall_error_number));
        }
        self.resolve_id(address)
    }
}

// Returns a token instance wrapping the token state.
fn as_token<'st, BS, RT>(
    st: &'st mut State,
    msg: &'st Messenger<'st, BS, RT>,
) -> Token<'st, &'st BS, &'st Messenger<'st, BS, RT>>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    Token::wrap(msg.rt.store(), msg, DATACAP_GRANULARITY, &mut st.token)
}

trait AsActorResult<T> {
    fn actor_result(self) -> Result<T, ActorError>;
}

impl<T> AsActorResult<T> for Result<T, TokenError> {
    fn actor_result(self) -> Result<T, ActorError> {
        self.map_err(|e| ActorError::unchecked(ExitCode::from(&e), e.to_string()))
    }
}

impl ActorCode for Actor {
    fn invoke_method<BS, RT>(
        rt: &mut RT,
        method: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        // I'm trying to find a fixed template for these blocks so we can macro it.
        // Current blockers:
        // - the serialize method maps () to CBOR null (we want no bytes instead)
        // - the serialize method can't do BigInts
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::Mint) => {
                let ret = Self::mint(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "mint result")
            }
            Some(Method::Destroy) => {
                let ret = Self::destroy(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "destroy result")
            }
            Some(Method::Name) => {
                let ret = Self::name(rt)?;
                serialize(&ret, "name result")
            }
            Some(Method::Symbol) => {
                let ret = Self::symbol(rt)?;
                serialize(&ret, "symbol result")
            }
            Some(Method::TotalSupply) => {
                let ret = Self::total_supply(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "total_supply result")
            }
            Some(Method::BalanceOf) => {
                let ret = Self::balance_of(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "balance_of result")
            }
            Some(Method::Transfer) => {
                let ret = Self::transfer(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "transfer result")
            }
            Some(Method::TransferFrom) => {
                let ret = Self::transfer_from(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "transfer_from result")
            }
            Some(Method::IncreaseAllowance) => {
                let ret = Self::increase_allowance(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "increase_allowance result")
            }
            Some(Method::DecreaseAllowance) => {
                let ret = Self::decrease_allowance(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "decrease_allowance result")
            }
            Some(Method::RevokeAllowance) => {
                Self::revoke_allowance(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::Burn) => {
                let ret = Self::burn(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "burn result")
            }
            Some(Method::BurnFrom) => {
                let ret = Self::burn_from(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "burn_from result")
            }
            Some(Method::Allowance) => {
                let ret = Self::allowance(rt, cbor::deserialize_params(params)?)?;
                serialize(&ret, "allowance result")
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
