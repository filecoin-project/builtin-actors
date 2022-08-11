#![allow(dead_code)]

use {
    crate::interp::{Message, Output, SignedTransaction, StatusCode, H160, U256},
    bytes::Bytes,
    cid::Cid,
    fil_actors_runtime::{runtime::Runtime, ActorError},
    fvm_ipld_blockstore::Blockstore,
    fvm_ipld_hamt::Hamt,
    fvm_shared::address::Address,
    std::{cell::RefCell, collections::HashSet},
};

/// Info sourced from the current transaction and block
#[derive(Clone, Debug)]
pub struct TransactionContext {
    /// The transaction gas price.
    pub tx_gas_price: U256,
    /// The transaction origin account.
    pub tx_origin: H160,
    /// The miner of the block.
    pub block_coinbase: H160,
    /// The block number.
    pub block_number: u64,
    /// The block timestamp.
    pub block_timestamp: u64,
    /// The block gas limit.
    pub block_gas_limit: u64,
    /// The block difficulty.
    pub block_difficulty: U256,
    /// The blockchain's ChainID.
    pub chain_id: U256,
    /// The block base fee per gas (EIP-1559, EIP-3198).
    pub block_base_fee: U256,
}

/// State access status (EIP-2929).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessStatus {
    Cold,
    Warm,
}

impl Default for AccessStatus {
    fn default() -> Self {
        Self::Cold
    }
}

#[derive(Clone, Copy, Debug)]
pub enum StorageStatus {
    /// The value of a storage item has been left unchanged: 0 -> 0 and X -> X.
    Unchanged,
    /// The value of a storage item has been modified: X -> Y.
    Modified,
    /// A storage item has been modified after being modified before: X -> Y -> Z.
    ModifiedAgain,
    /// A new storage item has been added: 0 -> X.
    Added,
    /// A storage item has been deleted: X -> 0.
    Deleted,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Call<'a> {
    Call(&'a Message),
    Create(&'a Message),
}

/// Platform Abstraction Layer
/// that bridges the FVM world to EVM world
pub struct System<'r, BS: Blockstore> {
    state: RefCell<Hamt<&'r BS, U256, U256>>,
    access_list: RefCell<HashSet<U256>>,
    _bridge: Address,
    self_address: H160,
    context: TransactionContext,
}

impl<'r, BS: Blockstore> System<'r, BS> {
    pub fn new<RT: Runtime<BS>>(
        state_cid: Cid,
        runtime: &'r RT,
        bridge: Address,
        self_address: H160,
        tx: &SignedTransaction,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            context: TransactionContext {
                tx_gas_price: tx.gas_price(),
                tx_origin: tx.sender_address()?,
                block_coinbase: H160::zero(),   // todo
                block_number: 0,                // todo
                block_timestamp: 0,             // todo
                block_gas_limit: 30000000,      // todo
                block_difficulty: U256::zero(), // todo
                chain_id: tx.chain_id().unwrap_or_default().into(),
                block_base_fee: U256::zero(), // todo
            },
            _bridge: bridge,
            self_address,
            access_list: RefCell::new(HashSet::new()),
            state: RefCell::new(Hamt::load(&state_cid, runtime.store())?),
        })
    }
}

impl<'r, BS: Blockstore> System<'r, BS> {
    pub fn flush_state(&self) -> Result<Cid, ActorError> {
        self.state.borrow_mut().flush().map_err(|e| ActorError::illegal_state(e.to_string()))
    }

    /// Check if an account exists.
    pub fn account_exists(&self, _address: H160) -> bool {
        todo!()
    }

    /// Get value of a storage key.
    ///
    /// Returns `Ok(U256::zero())` if does not exist.
    pub fn get_storage(&self, _address: H160, _key: U256) -> U256 {
        todo!();
    }

    /// Set value of a storage key.
    pub fn set_storage(
        &self,
        address: H160,
        key: U256,
        value: U256,
    ) -> Result<StorageStatus, StatusCode> {
        fvm_sdk::debug::log(format!("setting storage for {address:?} @ {key} to {value}"));
        if address == self.self_address {
            let mut storage_status = StorageStatus::Added;
            let prev_value = self
                .state
                .borrow()
                .get(&key)
                .map_err(|e| StatusCode::InternalError(e.to_string()))?
                .cloned();

            if let Some(v) = prev_value {
                if v == value {
                    storage_status = StorageStatus::Unchanged;
                } else {
                    storage_status = StorageStatus::Modified;
                }
            }

            if value == U256::zero() {
                self.state
                    .borrow_mut()
                    .delete(&key)
                    .map_err(|e| StatusCode::InternalError(e.to_string()))?;
                storage_status = StorageStatus::Deleted;
            } else {
                self.state
                    .borrow_mut()
                    .set(key, value)
                    .map_err(|e| StatusCode::InternalError(e.to_string()))?;
            }

            Ok(storage_status)
        } else {
            unimplemented!("setting storage across contracts is not supported yet")
        }
    }

    /// Get balance of an account.
    ///
    /// Returns `Ok(0)` if account does not exist.
    pub fn get_balance(&self, _address: H160) -> U256 {
        todo!()
    }

    /// Get code size of an account.
    ///
    /// Returns `Ok(0)` if account does not exist.
    pub fn get_code_size(&self, _address: H160) -> U256 {
        todo!()
    }

    /// Get code hash of an account.
    ///
    /// Returns `Ok(0)` if account does not exist.
    pub fn get_code_hash(&self, _address: H160) -> U256 {
        todo!();
    }

    /// Copy code of an account.
    ///
    /// Returns `Ok(0)` if offset is invalid.
    pub fn copy_code(&self, _address: H160, _offset: usize, _buffer: &mut [u8]) -> usize {
        todo!()
    }

    /// Self-destruct account.
    pub fn selfdestruct(&self, _address: H160, _beneficiary: H160) {
        todo!()
    }

    /// Call to another account.
    pub fn call(&self, _msg: Call) -> Output {
        todo!();
    }

    /// Get block hash.
    ///
    /// Returns `Ok(U256::zero())` if block does not exist.
    pub fn get_block_hash(&self, _block_number: u64) -> U256 {
        todo!();
    }

    /// Emit a log.
    pub fn emit_log(&self, _address: H160, _data: Bytes, _topics: &[U256]) {
        todo!();
    }

    /// Mark account as warm, return previous access status.
    ///
    /// Returns `Ok(AccessStatus::Cold)` if account does not exist.
    pub fn access_account(&self, _address: H160) -> AccessStatus {
        todo!();
    }

    /// Mark storage key as warm, return previous access status.
    ///
    /// Returns `Ok(AccessStatus::Cold)` if account does not exist.
    pub fn access_storage(&self, address: H160, key: U256) -> AccessStatus {
        if address == self.self_address {
            if self.access_list.borrow().contains(&key) {
                AccessStatus::Warm
            } else {
                self.access_list.borrow_mut().insert(key);
                AccessStatus::Cold
            }
        } else {
            unimplemented!("cross-contract storage access is not supported yet");
        }
    }

    /// Return context information about the current transaction and current block
    pub fn transaction_context(&self) -> &TransactionContext {
        &self.context
    }
}
