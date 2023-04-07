use fil_actor_market::Method as MarketMethod;
use fil_actor_market::WithdrawBalanceParams as MarketWithdrawBalanceParams;
use fil_actor_miner::Method as MinerMethod;
use fil_actor_miner::WithdrawBalanceParams as MinerWithdrawBalanceParams;
use fil_actors_runtime::test_utils::{MARKET_ACTOR_CODE_ID, MINER_ACTOR_CODE_ID};
use fil_actors_runtime::STORAGE_MARKET_ACTOR_ADDR;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;

use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredPoStProof;
use fvm_shared::METHOD_SEND;
use test_vm::util::{apply_code, apply_ok, create_accounts, create_miner};
use test_vm::Actor;
use test_vm::TestVM;

#[cfg(test)]
mod market_tests {
    use super::*;

    #[test]
    fn withdraw_all_funds() {
        let store = MemoryBlockstore::new();
        let (v, caller) = market_setup(&store);

        let three_fil = TokenAmount::from_whole(3);
        assert_add_collateral_and_withdraw(
            &v,
            three_fil.clone(),
            three_fil.clone(),
            three_fil,
            STORAGE_MARKET_ACTOR_ADDR,
            caller,
        );
    }

    #[test]
    fn withdraw_as_much_as_possible() {
        let store = MemoryBlockstore::new();
        let (v, caller) = market_setup(&store);

        // Add 2 FIL of collateral and attempt to withdraw 3
        let two_fil = TokenAmount::from_whole(2);
        let three_fil = TokenAmount::from_whole(3);
        assert_add_collateral_and_withdraw(
            &v,
            two_fil.clone(),
            two_fil,
            three_fil,
            STORAGE_MARKET_ACTOR_ADDR,
            caller,
        );
    }

    #[test]
    fn withdraw_0() {
        let store = MemoryBlockstore::new();
        let (v, caller) = market_setup(&store);

        // Add 0 FIL of collateral and attempt to withdraw 3
        let three_fil = TokenAmount::from_whole(3);
        assert_add_collateral_and_withdraw(
            &v,
            TokenAmount::zero(),
            TokenAmount::zero(),
            three_fil,
            STORAGE_MARKET_ACTOR_ADDR,
            caller,
        );
    }
}

#[cfg(test)]
mod miner_tests {
    use super::*;

    #[test]
    fn withdraw_all_funds() {
        let store = MemoryBlockstore::new();
        let (v, _, owner, m_addr) = miner_setup(&store);

        let three_fil = TokenAmount::from_whole(3);
        assert_add_collateral_and_withdraw(
            &v,
            three_fil.clone(),
            three_fil.clone(),
            three_fil,
            m_addr,
            owner,
        );
    }

    #[test]
    fn withdraw_as_much_as_possible() {
        let store = MemoryBlockstore::new();
        let (v, _, owner, m_addr) = miner_setup(&store);

        let two_fil = TokenAmount::from_whole(2);
        let three_fil = TokenAmount::from_whole(3);
        assert_add_collateral_and_withdraw(&v, two_fil.clone(), two_fil, three_fil, m_addr, owner);
    }

    #[test]
    fn withdraw_0() {
        let store = MemoryBlockstore::new();
        let (v, _, owner, m_addr) = miner_setup(&store);

        let three_fil = TokenAmount::from_whole(3);
        assert_add_collateral_and_withdraw(
            &v,
            TokenAmount::zero(),
            TokenAmount::zero(),
            three_fil,
            m_addr,
            owner,
        );
    }

    #[test]
    fn withdraw_from_non_owner_address_fails() {
        let store = MemoryBlockstore::new();
        let (v, worker, _, m_addr) = miner_setup(&store);

        let one_fil = TokenAmount::from_whole(1);
        let params = MinerWithdrawBalanceParams { amount_requested: one_fil };
        apply_code(
            &v,
            &worker,
            &m_addr,
            &TokenAmount::zero(),
            MinerMethod::WithdrawBalance as u64,
            Some(params),
            ExitCode::USR_FORBIDDEN,
        );
    }
}

// Precondition: escrow is a market or miner addr.  If miner address caller must be the owner address.
// 1. Add collateral to escrow address
// 2. Send a withdraw message attempting to remove `requested` funds
// 3. Assert correct return value and actor balance transfer
fn assert_add_collateral_and_withdraw<BS: Blockstore>(
    v: &TestVM<BS>,
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
    if collateral.is_positive() {
        match a_type {
            x if x == *MINER_ACTOR_CODE_ID => {
                apply_ok(v, &caller, &escrow, &collateral, METHOD_SEND, None::<RawBytes>)
            }
            x if x == *MARKET_ACTOR_CODE_ID => apply_ok(
                v,
                &caller,
                &escrow,
                &collateral,
                MarketMethod::AddBalance as u64,
                Some(caller),
            ),
            _ => panic!("unreachable"),
        };
    }

    c = require_actor(v, caller);
    assert_eq!(&caller_initial_balance - &collateral, c.balance);

    // attempt to withdraw withdrawal
    let withdrawn: TokenAmount = match a_type {
        x if x == *MINER_ACTOR_CODE_ID => {
            let params = MinerWithdrawBalanceParams { amount_requested: requested };
            apply_ok(
                v,
                &caller,
                &escrow,
                &TokenAmount::zero(),
                MinerMethod::WithdrawBalance as u64,
                Some(params),
            )
            .deserialize()
            .unwrap()
        }
        x if x == *MARKET_ACTOR_CODE_ID => {
            let params =
                MarketWithdrawBalanceParams { provider_or_client: caller, amount: requested };
            apply_ok(
                v,
                &caller,
                &escrow,
                &TokenAmount::zero(),
                MarketMethod::WithdrawBalance as u64,
                Some(params),
            )
            .deserialize()
            .unwrap()
        }
        _ => panic!("unreachable"),
    };
    assert_eq!(expected_withdrawn, withdrawn);

    c = require_actor(v, caller);
    assert_eq!(caller_initial_balance, c.balance);
}

fn require_actor<BS: Blockstore>(v: &TestVM<BS>, addr: Address) -> Actor {
    v.get_actor(&addr).unwrap()
}

fn market_setup(store: &'_ MemoryBlockstore) -> (TestVM<MemoryBlockstore>, Address) {
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(store);
    let initial_balance = TokenAmount::from_whole(6);
    let addrs = create_accounts(&v, 1, &initial_balance);
    let caller = addrs[0];
    (v, caller)
}

fn miner_setup(
    store: &'_ MemoryBlockstore,
) -> (TestVM<MemoryBlockstore>, Address, Address, Address) {
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(store);
    let initial_balance = TokenAmount::from_whole(10_000);
    let addrs = create_accounts(&v, 2, &initial_balance);
    let (worker, owner) = (addrs[0], addrs[1]);

    // create miner
    let (m_addr, _) = create_miner(
        &v,
        &owner,
        &worker,
        RegisteredPoStProof::StackedDRGWindow32GiBV1P1,
        &TokenAmount::zero(),
    );

    (v, worker, owner, m_addr)
}
