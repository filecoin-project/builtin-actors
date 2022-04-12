// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_hamt::Error as HamtError;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::econ::TokenAmount;
use num_traits::{Signed, Zero};

use fil_actors_runtime::{
    actor_error, make_empty_map, make_map_with_root_and_bitwidth, ActorError, Map,
};

pub const BALANCE_TABLE_BITWIDTH: u32 = 6;

/// Balance table which handles getting and updating token balances specifically
pub struct BalanceTable<'a, BS>(Map<'a, BS, BigIntDe>);
impl<'a, BS> BalanceTable<'a, BS>
where
    BS: Blockstore,
{
    /// Initializes a new empty balance table
    pub fn new(bs: &'a BS) -> Self {
        Self(make_empty_map(bs, BALANCE_TABLE_BITWIDTH))
    }

    /// Initializes a balance table from a root Cid
    pub fn from_root(bs: &'a BS, cid: &Cid) -> Result<Self, HamtError<BS::Error>> {
        Ok(Self(make_map_with_root_and_bitwidth(cid, bs, BALANCE_TABLE_BITWIDTH)?))
    }

    /// Retrieve root from balance table
    pub fn root(&mut self) -> Result<Cid, HamtError<BS::Error>> {
        self.0.flush()
    }

    /// Gets token amount for given address in balance table
    pub fn get(&self, key: &Address) -> Result<TokenAmount, HamtError<BS::Error>> {
        if let Some(v) = self.0.get(&key.to_bytes())? {
            Ok(v.0.clone())
        } else {
            Ok(0.into())
        }
    }

    /// Adds token amount to previously initialized account.
    pub fn add(&mut self, key: &Address, value: &TokenAmount) -> Result<(), ActorError> {
        let prev = self.get(key)?;
        let sum = &prev + value;
        if sum.is_negative() {
            return Err(actor_error!(
                illegal_argument,
                "new balance in table cannot be negative: {}",
                sum
            ));
        }
        if sum.is_zero() && !prev.is_zero() {
            self.0.delete(&key.to_bytes())?;
            Ok(())
        } else {
            self.0.set(key.to_bytes().into(), BigIntDe(sum))?;
            Ok(())
        }
    }

    /// Subtracts up to the specified amount from a balance, without reducing the balance
    /// below some minimum.
    /// Returns the amount subtracted (always positive or zero).
    pub fn subtract_with_minimum(
        &mut self,
        key: &Address,
        req: &TokenAmount,
        floor: &TokenAmount,
    ) -> Result<TokenAmount, ActorError> {
        let prev = self.get(key)?;
        let available = std::cmp::max(TokenAmount::zero(), prev - floor);
        let sub: TokenAmount = std::cmp::min(&available, req).clone();

        if sub.is_positive() {
            self.add(key, &-sub.clone())?;
        }

        Ok(sub)
    }

    /// Subtracts value from a balance, and errors if full amount was not substracted.
    pub fn must_subtract(&mut self, key: &Address, req: &TokenAmount) -> Result<(), ActorError> {
        let prev = self.get(key)?;

        if req > &prev {
            return Err(actor_error!(illegal_argument, "couldn't subtract the requested amount"));
        }
        self.add(key, &-req)?;

        Ok(())
    }

    /// Returns total balance held by this balance table
    #[allow(dead_code)]
    pub fn total(&self) -> Result<TokenAmount, HamtError<BS::Error>> {
        let mut total = TokenAmount::default();

        self.0.for_each(|_, v: &BigIntDe| {
            total += &v.0;
        })?;

        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use fvm_ipld_blockstore::MemoryBlockstore;
    use fvm_shared::address::Address;
    use fvm_shared::econ::TokenAmount;

    use crate::balance_table::BalanceTable;

    #[test]
    fn total() {
        let addr1 = Address::new_id(100);
        let addr2 = Address::new_id(101);
        let store = MemoryBlockstore::default();
        let mut bt = BalanceTable::new(&store);

        assert_eq!(bt.total().unwrap(), TokenAmount::from(0u8));

        struct TotalTestCase<'a> {
            amount: u64,
            addr: &'a Address,
            total: u64,
        }
        let cases = [
            TotalTestCase { amount: 10, addr: &addr1, total: 10 },
            TotalTestCase { amount: 20, addr: &addr1, total: 30 },
            TotalTestCase { amount: 40, addr: &addr2, total: 70 },
            TotalTestCase { amount: 50, addr: &addr2, total: 120 },
        ];

        for t in cases.iter() {
            bt.add(t.addr, &TokenAmount::from(t.amount)).unwrap();

            assert_eq!(bt.total().unwrap(), TokenAmount::from(t.total));
        }
    }

    #[test]
    fn balance_subtracts() {
        let addr = Address::new_id(100);
        let store = MemoryBlockstore::default();
        let mut bt = BalanceTable::new(&store);

        bt.add(&addr, &TokenAmount::from(80u8)).unwrap();
        assert_eq!(bt.get(&addr).unwrap(), TokenAmount::from(80u8));
        // Test subtracting past minimum only subtracts correct amount
        assert_eq!(
            bt.subtract_with_minimum(&addr, &TokenAmount::from(20u8), &TokenAmount::from(70u8))
                .unwrap(),
            TokenAmount::from(10u8)
        );
        assert_eq!(bt.get(&addr).unwrap(), TokenAmount::from(70u8));

        // Test subtracting to limit
        assert_eq!(
            bt.subtract_with_minimum(&addr, &TokenAmount::from(10u8), &TokenAmount::from(60u8))
                .unwrap(),
            TokenAmount::from(10u8)
        );
        assert_eq!(bt.get(&addr).unwrap(), TokenAmount::from(60u8));

        // Test must subtract success
        bt.must_subtract(&addr, &TokenAmount::from(10u8)).unwrap();
        assert_eq!(bt.get(&addr).unwrap(), TokenAmount::from(50u8));

        // Test subtracting more than available
        assert!(bt.must_subtract(&addr, &TokenAmount::from(100u8)).is_err());
    }
}
