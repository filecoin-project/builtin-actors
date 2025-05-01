// Copyright 2024 Curio Storage Inc.
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;

/// State for the Sealer actor
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone)]
pub struct State {
    /// The ID address of this sealer actor
    pub id_addr: Address,
    // TODO: Add sector bitfield, ACL/proxy address, etc.
} 