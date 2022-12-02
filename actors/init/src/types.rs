// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{Cbor, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::error::ExitCode;

/// Init actor Constructor parameters
#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    pub network_name: String,
}

/// Init actor Exec Params
#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ExecParams {
    pub code_cid: Cid,
    pub constructor_params: RawBytes,
}

/// Init actor Exec Return value
#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ExecReturn {
    /// ID based address for created actor
    pub id_address: Address,
    /// Reorg safe address for actor
    pub robust_address: Address,
    // /// Constructor failed
    // pub constructor_failure: RawBytes
}

/// Init actor Exec4 Params
#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct Exec4Params {
    pub code_cid: Cid,
    pub constructor_params: RawBytes,
    pub subaddress: RawBytes,
}

/// Init actor Exec4 Return value
pub type Exec4Return = ExecReturn;

impl Cbor for ExecReturn {}
impl Cbor for ExecParams {}
impl Cbor for Exec4Params {}

/// Init actor Install Params
#[cfg(feature = "m2-native")]
#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct InstallParams {
    pub code: RawBytes,
}

/// Init actor Install Return value
#[cfg(feature = "m2-native")]
#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct InstallReturn {
    pub code_cid: Cid,
    pub installed: bool,
}

#[cfg(feature = "m2-native")]
impl Cbor for InstallParams {}
#[cfg(feature = "m2-native")]
impl Cbor for InstallReturn {}
