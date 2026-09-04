use cid::Cid;
use frc46_token::token::types::{
    BurnFromParams, BurnFromReturn, BurnParams, BurnReturn, DecreaseAllowanceParams,
    GetAllowanceParams, IncreaseAllowanceParams, MintReturn, RevokeAllowanceParams,
    TransferFromParams, TransferFromReturn, TransferParams, TransferReturn,
};
use frc46_token::token::{TOKEN_PRECISION, Token, TokenError};
use fvm_actor_utils::receiver::ReceiverHookError;
use fvm_actor_utils::syscalls::{NoStateError, Syscalls};
use fvm_actor_utils::util::ActorRuntime;
use fvm_shared::Response;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::{ActorID, METHOD_CONSTRUCTOR, MethodNum};
use log::info;
use num_derive::FromPrimitive;

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    ActorContext, ActorError, SYSTEM_ACTOR_ADDR, actor_dispatch, actor_error, extract_send_result,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;

pub use self::state::State;
pub use self::types::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

mod state;
pub mod testing;
mod types;

pub const DATACAP_GRANULARITY: u64 = TOKEN_PRECISION;

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

// Callers apply their own caller validation before calling this.
fn datacap_deprecated<T>(reason: &str) -> Result<T, ActorError> {
    Err(actor_error!(
        forbidden,
        "FIP-0118: datacap is deprecated, {} is no longer supported",
        reason
    ))
}

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

    pub fn mint(rt: &impl Runtime, _params: MintParams) -> Result<MintReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("minting")
    }

    /// Destroys data cap tokens for an address (a verified client).
    /// Only the governor can call this method.
    /// This method is not part of the fungible token standard, and is named distinctly from
    /// "burn" to reflect that distinction.
    pub fn destroy(rt: &impl Runtime, _params: DestroyParams) -> Result<BurnReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("destroying datacap")
    }

    pub fn transfer(
        rt: &impl Runtime,
        _params: TransferParams,
    ) -> Result<TransferReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("transfer")
    }

    pub fn transfer_from(
        rt: &impl Runtime,
        _params: TransferFromParams,
    ) -> Result<TransferFromReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("transfer")
    }

    pub fn increase_allowance(
        rt: &impl Runtime,
        _params: IncreaseAllowanceParams,
    ) -> Result<IncreaseAllowanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("increasing allowance")
    }

    pub fn decrease_allowance(
        rt: &impl Runtime,
        _params: DecreaseAllowanceParams,
    ) -> Result<DecreaseAllowanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("decreasing allowance")
    }

    pub fn revoke_allowance(
        rt: &impl Runtime,
        _params: RevokeAllowanceParams,
    ) -> Result<RevokeAllowanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("revoking allowance")
    }

    pub fn burn(rt: &impl Runtime, _params: BurnParams) -> Result<BurnReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("burn")
    }

    pub fn burn_from(
        rt: &impl Runtime,
        _params: BurnFromParams,
    ) -> Result<BurnFromReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("burn")
    }
}

/// Implementation of the token library's messenger trait in terms of the built-in actors'
/// runtime library.
struct SyscallProvider<'a, RT> {
    rt: &'a RT,
}

impl<RT> Syscalls for &SyscallProvider<'_, RT>
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
