// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_shared::ActorID;
use fvm_shared::address::Address;

/// Singleton Actor IDs
pub const SYSTEM_ACTOR_ID: ActorID = 0;
pub const INIT_ACTOR_ID: ActorID = 1;
pub const REWARD_ACTOR_ID: ActorID = 2;
pub const CRON_ACTOR_ID: ActorID = 3;
pub const STORAGE_POWER_ACTOR_ID: ActorID = 4;
pub const STORAGE_MARKET_ACTOR_ID: ActorID = 5;
pub const VERIFIED_REGISTRY_ACTOR_ID: ActorID = 6;
pub const DATACAP_TOKEN_ACTOR_ID: ActorID = 7;
pub const EAM_ACTOR_ID: ActorID = 10;
pub const BURNT_FUNDS_ACTOR_ID: ActorID = 99;

/// Singleton Actor Addresses
pub const SYSTEM_ACTOR_ADDR: Address = Address::new_id(SYSTEM_ACTOR_ID);
pub const INIT_ACTOR_ADDR: Address = Address::new_id(INIT_ACTOR_ID);
pub const REWARD_ACTOR_ADDR: Address = Address::new_id(REWARD_ACTOR_ID);
pub const CRON_ACTOR_ADDR: Address = Address::new_id(CRON_ACTOR_ID);
pub const STORAGE_POWER_ACTOR_ADDR: Address = Address::new_id(STORAGE_POWER_ACTOR_ID);
pub const STORAGE_MARKET_ACTOR_ADDR: Address = Address::new_id(STORAGE_MARKET_ACTOR_ID);
pub const VERIFIED_REGISTRY_ACTOR_ADDR: Address = Address::new_id(VERIFIED_REGISTRY_ACTOR_ID);
pub const DATACAP_TOKEN_ACTOR_ADDR: Address = Address::new_id(DATACAP_TOKEN_ACTOR_ID);
pub const EAM_ACTOR_ADDR: Address = Address::new_id(EAM_ACTOR_ID);
pub const BURNT_FUNDS_ACTOR_ADDR: Address = Address::new_id(BURNT_FUNDS_ACTOR_ID);

/// Defines first available ID address after builtin actors
pub const FIRST_NON_SINGLETON_ADDR: ActorID = 100;
