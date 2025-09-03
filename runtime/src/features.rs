// Centralized feature activation points for actors/runtime behavior.
// TODO: Update NV_EIP_7702 to the network version chosen for activation at upgrade time.

use fvm_shared::version::NetworkVersion;

pub const NV_EIP_7702: NetworkVersion = NetworkVersion::V16;

