use cid::Cid;
use frc46_token::token::types::{
    BurnFromParams, BurnFromReturn, BurnParams, BurnReturn, DecreaseAllowanceParams,
    GetAllowanceParams, IncreaseAllowanceParams, MintReturn, RevokeAllowanceParams,
    TransferFromParams, TransferFromReturn, TransferParams, TransferReturn,
};
use frc46_token::token::{Token, TokenError, TOKEN_PRECISION};
use fvm_actor_utils::receiver::ReceiverHookError;
use fvm_actor_utils::syscalls::{NoStateError, Syscalls};
use fvm_actor_utils::util::ActorRuntime;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::Response;
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR};
use lazy_static::lazy_static;
use log::info;
use num_derive::FromPrimitive;

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_dispatch, actor_error, extract_send_result, ActorContext, ActorError, AsActorError,
    SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;

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
    pub static ref INFINITE_ALLOWANCE: TokenAmount = TokenAmount::from_atto(
        BigInt::from(TOKEN_PRECISION)
            * BigInt::from(1_000_000_000_000_000_000_000_i128)
    );
}

/// Datacap actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    // Deprecated in v10
    // Mint = 2,
    // Destroy = 3,
    // Name = 10,
    // Symbol = 11,
    // TotalSupply = 12,
    // BalanceOf = 13,
    // Transfer = 14,
    // TransferFrom = 15,
    // IncreaseAllowance = 16,
    // DecreaseAllowance = 17,
    // RevokeAllowance = 18,
    // Burn = 19,
    // BurnFrom = 20,
    // Allowance = 21,
    // Method numbers derived from FRC-0042 standards
    MintExported = frc42_dispatch::method_hash!("Mint"),
    DestroyExported = frc42_dispatch::method_hash!("Destroy"),
    NameExported = frc42_dispatch::method_hash!("Name"),
    SymbolExported = frc42_dispatch::method_hash!("Symbol"),
    GranularityExported = frc42_dispatch::method_hash!("Granularity"),
    TotalSupplyExported = frc42_dispatch::method_hash!("TotalSupply"),
    BalanceExported = frc42_dispatch::method_hash!("Balance"),
    TransferExported = frc42_dispatch::method_hash!("Transfer"),
    TransferFromExported = frc42_dispatch::method_hash!("TransferFrom"),
    IncreaseAllowanceExported = frc42_dispatch::method_hash!("IncreaseAllowance"),
    DecreaseAllowanceExported = frc42_dispatch::method_hash!("DecreaseAllowance"),
    RevokeAllowanceExported = frc42_dispatch::method_hash!("RevokeAllowance"),
    BurnExported = frc42_dispatch::method_hash!("Burn"),
    BurnFromExported = frc42_dispatch::method_hash!("BurnFrom"),
    AllowanceExported = frc42_dispatch::method_hash!("Allowance"),
}

pub struct Actor;

impl Actor {
    /// Constructor for DataCap Actor
    pub fn constructor(rt: &impl Runtime, params: ConstructorParams) -> Result<(), ActorError> {
        let governor = params.governor;
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        // Confirm the governor address is an ID.
        rt.resolve_address(&governor)
            .ok_or_else(|| actor_error!(illegal_argument, "failed to resolve governor address"))?;

        let st = State::new(rt.store(), governor).context("failed to create datacap state")?;
        rt.create(&st)?;
        Ok(())
    }

    pub fn name(rt: &impl Runtime) -> Result<NameReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        Ok(NameReturn { name: "DataCap".to_string() })
    }

    pub fn symbol(rt: &impl Runtime) -> Result<SymbolReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        Ok(SymbolReturn { symbol: "DCAP".to_string() })
    }

    pub fn granularity(rt: &impl Runtime) -> Result<GranularityReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        Ok(GranularityReturn { granularity: DATACAP_GRANULARITY })
    }

    pub fn total_supply(rt: &impl Runtime) -> Result<TotalSupplyReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let mut st: State = rt.state()?;
        let syscalls = SyscallProvider { rt };
        let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
        let token = as_token(&mut st, &runtime);
        Ok(TotalSupplyReturn { supply: token.total_supply() })
    }

    pub fn balance(rt: &impl Runtime, params: BalanceParams) -> Result<BalanceReturn, ActorError> {
        // NOTE: mutability and method caller here are awkward for a read-only call
        rt.validate_immediate_caller_accept_any()?;
        let mut st: State = rt.state()?;
        let syscalls = SyscallProvider { rt };
        let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
        let token = as_token(&mut st, &runtime);
        token.balance_of(&params.address).map(|balance| BalanceReturn { balance }).actor_result()
    }

    pub fn allowance(
        rt: &impl Runtime,
        params: GetAllowanceParams,
    ) -> Result<GetAllowanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let mut st: State = rt.state()?;
        let syscalls = SyscallProvider { rt };
        let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
        let token = as_token(&mut st, &runtime);
        token
            .allowance(&params.owner, &params.operator)
            .map(|allowance| GetAllowanceReturn { allowance })
            .actor_result()
    }

    /// Mints new data cap tokens for an address (a verified client).
    /// Simultaneously sets the allowance for any specified operators to effectively infinite.
    /// Only the governor can call this method.
    /// This method is not part of the fungible token standard.
    pub fn mint(rt: &impl Runtime, params: MintParams) -> Result<MintReturn, ActorError> {
        let mut hook = rt
            .transaction(|st: &mut State, rt| {
                // Only the governor can mint datacap tokens.
                rt.validate_immediate_caller_is(std::iter::once(&st.governor))?;
                let operator = st.governor;

                let syscalls = SyscallProvider { rt };
                let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
                let mut token = as_token(st, &runtime);
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
        let syscalls = SyscallProvider { rt };
        let intermediate = hook.call(&as_actor_runtime(&syscalls)).actor_result()?;
        let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
        as_token(&mut st, &runtime).mint_return(intermediate).actor_result()
    }

    /// Destroys data cap tokens for an address (a verified client).
    /// Only the governor can call this method.
    /// This method is not part of the fungible token standard, and is named distinctly from
    /// "burn" to reflect that distinction.
    pub fn destroy(rt: &impl Runtime, params: DestroyParams) -> Result<BurnReturn, ActorError> {
        rt.transaction(|st: &mut State, rt| {
            // Only the governor can destroy datacap tokens on behalf of a holder.
            rt.validate_immediate_caller_is(std::iter::once(&st.governor))?;

            let syscalls = SyscallProvider { rt };
            let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
            let mut token = as_token(st, &runtime);
            // Burn tokens as if the holder had invoked burn() themselves.
            // The governor doesn't need an allowance.
            token.burn(&params.owner, &params.amount).actor_result()
        })
        .context("state transaction failed")
    }

    /// Transfers data cap tokens to an address.
    /// Data cap tokens are not generally transferable.
    /// Succeeds if the to or from address is the governor, otherwise always fails.
    pub fn transfer(
        rt: &impl Runtime,
        params: TransferParams,
    ) -> Result<TransferReturn, ActorError> {
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

                let syscalls = SyscallProvider { rt };
                let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
                let mut token = as_token(st, &runtime);
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
        let syscalls = SyscallProvider { rt };
        let intermediate = hook.call(&as_actor_runtime(&syscalls)).actor_result()?;
        let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
        as_token(&mut st, &runtime).transfer_return(intermediate).actor_result()
    }

    /// Transfers data cap tokens between addresses.
    /// Data cap tokens are not generally transferable between addresses.
    /// Succeeds if the to address is the governor, otherwise always fails.
    pub fn transfer_from(
        rt: &impl Runtime,
        params: TransferFromParams,
    ) -> Result<TransferFromReturn, ActorError> {
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

                let syscalls = SyscallProvider { rt };
                let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
                let mut token = as_token(st, &runtime);
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
        let syscalls = SyscallProvider { rt };
        let intermediate = hook.call(&as_actor_runtime(&syscalls)).actor_result()?;
        let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
        as_token(&mut st, &runtime).transfer_from_return(intermediate).actor_result()
    }

    pub fn increase_allowance(
        rt: &impl Runtime,
        params: IncreaseAllowanceParams,
    ) -> Result<IncreaseAllowanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let owner = rt.message().caller();
        let operator = params.operator;

        rt.transaction(|st: &mut State, rt| {
            let syscalls = SyscallProvider { rt };
            let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
            let mut token = as_token(st, &runtime);
            token
                .increase_allowance(&owner, &operator, &params.increase)
                .map(|new_allowance| IncreaseAllowanceReturn { new_allowance })
                .actor_result()
        })
        .context("state transaction failed")
    }

    pub fn decrease_allowance(
        rt: &impl Runtime,
        params: DecreaseAllowanceParams,
    ) -> Result<DecreaseAllowanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let owner = &rt.message().caller();
        let operator = &params.operator;

        rt.transaction(|st: &mut State, rt| {
            let syscalls = SyscallProvider { rt };
            let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
            let mut token = as_token(st, &runtime);
            token
                .decrease_allowance(owner, operator, &params.decrease)
                .map(|new_allowance| DecreaseAllowanceReturn { new_allowance })
                .actor_result()
        })
        .context("state transaction failed")
    }

    pub fn revoke_allowance(
        rt: &impl Runtime,
        params: RevokeAllowanceParams,
    ) -> Result<RevokeAllowanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let owner = &rt.message().caller();
        let operator = &params.operator;

        rt.transaction(|st: &mut State, rt| {
            let syscalls = SyscallProvider { rt };
            let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
            let mut token = as_token(st, &runtime);
            token
                .revoke_allowance(owner, operator)
                .map(|old_allowance| RevokeAllowanceReturn { old_allowance })
                .actor_result()
        })
        .context("state transaction failed")
    }

    pub fn burn(rt: &impl Runtime, params: BurnParams) -> Result<BurnReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let owner = &rt.message().caller();

        rt.transaction(|st: &mut State, rt| {
            let syscalls = SyscallProvider { rt };
            let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
            let mut token = as_token(st, &runtime);
            token.burn(owner, &params.amount).actor_result()
        })
        .context("state transaction failed")
    }

    pub fn burn_from(
        rt: &impl Runtime,
        params: BurnFromParams,
    ) -> Result<BurnFromReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let operator = &rt.message().caller();
        let owner = &params.owner;

        rt.transaction(|st: &mut State, rt| {
            let syscalls = SyscallProvider { rt };
            let runtime = ActorRuntime::new(&syscalls, syscalls.rt.store());
            let mut token = as_token(st, &runtime);
            token.burn_from(operator, owner, &params.amount).actor_result()
        })
        .context("state transaction failed")
    }
}

/// Implementation of the token library's messenger trait in terms of the built-in actors'
/// runtime library.
struct SyscallProvider<'a, RT> {
    rt: &'a RT,
}

impl<'a, RT> Syscalls for &SyscallProvider<'a, RT>
where
    RT: Runtime,
{
    fn root(&self) -> Result<Cid, NoStateError> {
        self.rt.get_state_root().map_err(|_| NoStateError)
    }

    fn receiver(&self) -> ActorID {
        self.rt.message().receiver().id().unwrap()
    }

    // This never returns an Err.  However we could return an error if the
    // Runtime send method passed through the underlying syscall error
    // instead of hiding it behind a client-side chosen exit code.
    fn send(
        &self,
        to: &Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
    ) -> Result<Response, ErrorNumber> {
        // The Runtime discards some of the information from the syscall :-(
        let res = extract_send_result(self.rt.send_simple(to, method, params, value));

        let rec = match res {
            Ok(ret) => Response { exit_code: ExitCode::OK, return_data: ret },
            Err(ae) => {
                info!("datacap messenger failed: {}", ae.msg());
                Response { exit_code: ae.exit_code(), return_data: None }
            }
        };
        Ok(rec)
    }

    fn resolve_address(&self, addr: &Address) -> Option<ActorID> {
        self.rt.resolve_address(addr)
    }

    fn set_root(&self, cid: &Cid) -> Result<(), NoStateError> {
        self.rt.set_state_root(cid).map_err(|_| NoStateError)
    }

    fn caller(&self) -> ActorID {
        self.rt.message().caller().id().unwrap()
    }
}

// Returns a token instance wrapping the token state.
fn as_token<'st, RT>(
    st: &'st mut State,
    token_runtime: &'st ActorRuntime<&'st SyscallProvider<'st, RT>, &'st RT::Blockstore>,
) -> Token<'st, &'st SyscallProvider<'st, RT>, &'st RT::Blockstore>
where
    RT: Runtime,
{
    Token::wrap(token_runtime, DATACAP_GRANULARITY, &mut st.token)
}

// Returns an ActorRuntime wrapping the Runtime and Blockstore
fn as_actor_runtime<'st, RT>(
    sys_provider: &'st SyscallProvider<'st, RT>,
) -> ActorRuntime<&'st SyscallProvider<'st, RT>, &'st RT::Blockstore>
where
    RT: Runtime,
{
    ActorRuntime::new(sys_provider, sys_provider.rt.store())
}

trait AsActorResult<T> {
    fn actor_result(self) -> Result<T, ActorError>;
}

impl<T> AsActorResult<T> for Result<T, TokenError> {
    fn actor_result(self) -> Result<T, ActorError> {
        self.map_err(|e| ActorError::unchecked(ExitCode::from(&e), e.to_string()))
    }
}

impl<T> AsActorResult<T> for Result<T, ReceiverHookError> {
    fn actor_result(self) -> Result<T, ActorError> {
        self.map_err(|e| ActorError::unchecked(ExitCode::from(&e), e.to_string()))
    }
}

impl ActorCode for Actor {
    type Methods = Method;

    fn name() -> &'static str {
        "DataCap"
    }

    actor_dispatch! {
        Constructor => constructor,
        MintExported => mint,
        DestroyExported => destroy,
        NameExported => name,
        SymbolExported => symbol,
        GranularityExported => granularity,
        TotalSupplyExported => total_supply,
        BalanceExported => balance,
        TransferExported => transfer,
        TransferFromExported => transfer_from,
        IncreaseAllowanceExported => increase_allowance,
        DecreaseAllowanceExported => decrease_allowance,
        RevokeAllowanceExported => revoke_allowance,
        BurnExported => burn,
        BurnFromExported => burn_from,
        AllowanceExported => allowance,
    }
}
