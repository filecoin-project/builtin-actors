use fil_actor_market::Method as MarketMethod;
use fil_actor_market::WithdrawBalanceParams as MarketWithdrawBalanceParams;
use fil_actor_miner::Method as MinerMethod;
use fil_actor_miner::WithdrawBalanceParams as MinerWithdrawBalanceParams;
use fil_actors_runtime::test_utils::{MARKET_ACTOR_CODE_ID, MINER_ACTOR_CODE_ID};
use fil_actors_runtime::STORAGE_MARKET_ACTOR_ADDR;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::METHOD_SEND;
use test_vm::util::{apply_ok, create_accounts};
use test_vm::Actor;
use test_vm::VM;

#[test]
fn withdraw_all_funds() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);
    let initial_balance = BigInt::from(6) * BigInt::from(1e18 as i128);
    let addrs = create_accounts(&v, 1, initial_balance);
    let caller = addrs[0];

    let three_fil = TokenAmount::from(3);
    assert_add_collateral_and_withdraw(
        &v,
        three_fil.clone(),
        three_fil.clone(),
        three_fil,
        *STORAGE_MARKET_ACTOR_ADDR,
        caller,
    );
}

#[test]
fn withdraw_as_much_as_possible() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);
    let initial_balance = BigInt::from(6) * BigInt::from(1e18 as i128);
    let addrs = create_accounts(&v, 1, initial_balance);
    let caller = addrs[0];

    // Add 2 FIL of collateral and attempt to withdraw 3
    let two_fil = TokenAmount::from(2);
    let three_fil = TokenAmount::from(3);
    assert_add_collateral_and_withdraw(
        &v,
        two_fil.clone(),
        two_fil,
        three_fil,
        *STORAGE_MARKET_ACTOR_ADDR,
        caller,
    );
}

#[test]
fn withdraw_0() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);
    let initial_balance = BigInt::from(6) * BigInt::from(1e18 as i128);
    let addrs = create_accounts(&v, 1, initial_balance);
    let caller = addrs[0];

    // Add 0 FIL of collateral and attempt to withdraw 3
    let three_fil = TokenAmount::from(3);
    assert_add_collateral_and_withdraw(
        &v,
        TokenAmount::zero(),
        TokenAmount::zero(),
        three_fil,
        *STORAGE_MARKET_ACTOR_ADDR,
        caller,
    );
}

// Precondition: escrow is a market or miner addr.  If miner address caller must be the owner address.
// 1. Add collateral to escrow address
// 2. Send a withdraw message attempting to remove `requested` funds
// 3. Assert correct return value and actor balance transfer
fn assert_add_collateral_and_withdraw(
    v: &VM,
    collateral: TokenAmount,
    expected_withdrawn: TokenAmount,
    requested: TokenAmount,
    escrow: Address,
    caller: Address,
) {
    // get code cid
    let e = require_actor(v, escrow);
    let a_type = e.code;
    if a_type != *MINER_ACTOR_CODE_ID && a_type != *MARKET_ACTOR_CODE_ID {
        panic!("unexepcted escrow address actor type: {}", a_type);
    }

    // caller initial balance
    let mut c = require_actor(v, caller);
    let caller_initial_balance = c.balance;

    // add collateral
    if collateral > BigInt::zero() {
        match a_type {
            x if x == *MINER_ACTOR_CODE_ID => {
                apply_ok(v, caller, escrow, collateral.clone(), METHOD_SEND, RawBytes::default())
            }
            x if x == *MARKET_ACTOR_CODE_ID => apply_ok(
                v,
                caller,
                escrow,
                collateral.clone(),
                MarketMethod::AddBalance as u64,
                caller,
            ),
            _ => panic!("unreachable"),
        };
    }

    c = require_actor(v, caller);
    assert_eq!(&caller_initial_balance - &collateral, c.balance);

    // attempt to withdraw withdrawal
    let ret = match a_type {
        x if x == *MINER_ACTOR_CODE_ID => {
            let params = MinerWithdrawBalanceParams { amount_requested: requested };
            apply_ok(v, caller, escrow, BigInt::zero(), MinerMethod::WithdrawBalance as u64, params)
        }
        x if x == *MARKET_ACTOR_CODE_ID => {
            let params =
                MarketWithdrawBalanceParams { provider_or_client: caller, amount: collateral };
            apply_ok(
                v,
                caller,
                escrow,
                BigInt::zero(),
                MarketMethod::WithdrawBalance as u64,
                params,
            )
        }
        _ => panic!("unreachable"),
    };
    println!("{:?}", ret);
    todo!();
    // let idx = ret.bytes().len()-1;
    // let withdrawn = TokenAmount::from(ret.bytes()[idx as usize]);
    // assert_eq!(expected_withdrawn, withdrawn);

    // c = require_actor(v, caller);
    // assert_eq!(caller_initial_balance, c.balance);
}

fn require_actor(v: &VM, addr: Address) -> Actor {
    v.get_actor(addr).unwrap()
}
