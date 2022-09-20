// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_shared::address::Address;
use fvm_shared::ActorID;

pub const SYSTEM_ACTOR_ID: ActorID = 0;
pub const SYSTEM_ACTOR_ADDR: Address = Address::new_id(SYSTEM_ACTOR_ID);

pub const INIT_ACTOR_ID: ActorID = 1;
pub const INIT_ACTOR_ADDR: Address = Address::new_id(INIT_ACTOR_ID);

pub const REWARD_ACTOR_ID: ActorID = 2;
pub const REWARD_ACTOR_ADDR: Address = Address::new_id(REWARD_ACTOR_ID);

pub const CRON_ACTOR_ID: ActorID = 3;
pub const CRON_ACTOR_ADDR: Address = Address::new_id(CRON_ACTOR_ID);

pub const STORAGE_POWER_ACTOR_ID: ActorID = 4;
pub const STORAGE_POWER_ACTOR_ADDR: Address = Address::new_id(STORAGE_POWER_ACTOR_ID);

pub const STORAGE_MARKET_ACTOR_ID: ActorID = 5;
pub const STORAGE_MARKET_ACTOR_ADDR: Address = Address::new_id(STORAGE_MARKET_ACTOR_ID);

pub const VERIFIED_REGISTRY_ACTOR_ID: ActorID = 6;
pub const VERIFIED_REGISTRY_ACTOR_ADDR: Address = Address::new_id(VERIFIED_REGISTRY_ACTOR_ID);

pub const CHAOS_ACTOR_ID: ActorID = 98;
pub const CHAOS_ACTOR_ADDR: Address = Address::new_id(CHAOS_ACTOR_ID);

pub const BURNT_FUNDS_ACTOR_ID: ActorID = 99;
pub const BURNT_FUNDS_ACTOR_ADDR: Address = Address::new_id(BURNT_FUNDS_ACTOR_ID);

/// Defines first available ID address after builtin actors
pub const FIRST_NON_SINGLETON_ADDR: ActorID = 100;
