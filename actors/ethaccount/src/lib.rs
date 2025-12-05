pub mod state;
pub mod types;

use crate::state::State;
use cid::Cid;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::eip7702::{ApplyAndCallParams, ApplyAndCallReturn};
use fil_actors_runtime::WithCodec;
use fil_actors_runtime::runtime::EMPTY_ARR_CID;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    ActorError, EAM_ACTOR_ID, FIRST_EXPORTED_METHOD_NUMBER, SYSTEM_ACTOR_ADDR, actor_dispatch,
    actor_error,
};
use fvm_ipld_encoding::DAG_CBOR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::strict_bytes;
use fvm_ipld_encoding::tuple::{Deserialize_tuple, Serialize_tuple, serde_tuple};
use fvm_shared::address::Address;
use fvm_shared::address::Payload;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sys::SendFlags;
use fvm_shared::{METHOD_CONSTRUCTOR, MethodNum};
use num_derive::FromPrimitive;
use std::collections::HashSet;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(EthAccountActor);

/// Ethereum Account actor methods.
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    ApplyAndCall = frc42_dispatch::method_hash!("ApplyAndCall"),
}

/// Ethereum Account actor.
pub struct EthAccountActor;

// Minimal local copy of the EVM InvokeContract params shape, to avoid a hard
// dependency on the EVM actor crate while still routing outer calls through
// the EVM entrypoint when the target is an EVM contract.
#[derive(Serialize_tuple, Deserialize_tuple)]
struct InvokeContractParams {
    #[serde(with = "strict_bytes")]
    pub input_data: Vec<u8>,
}

impl EthAccountActor {
    fn ensure_initialized(rt: &impl Runtime) -> Result<(), ActorError> {
        // If state root is empty, create initial state.
        let root = rt.get_state_root()?;
        if root == EMPTY_ARR_CID {
            rt.create(&State {
                delegate_to: None,
                auth_nonce: 0,
                evm_storage_root: EMPTY_ARR_CID,
            })?;
        }
        Ok(())
    }

    fn validate_tuple(
        rt: &impl Runtime,
        t: &fil_actors_evm_shared::eip7702::DelegationParam,
    ) -> Result<(), ActorError> {
        // chain id 0 or local
        if t.chain_id != 0 && fvm_shared::chainid::ChainID::from(t.chain_id) != rt.chain_id() {
            return Err(ActorError::illegal_argument("invalid chain id".into()));
        }
        // Length checks first: r,s must be <= 32 bytes.
        if t.r.len() > 32 {
            return Err(ActorError::illegal_argument("r length exceeds 32".into()));
        }
        if t.s.len() > 32 {
            return Err(ActorError::illegal_argument("s length exceeds 32".into()));
        }
        // r/s non-zero
        if t.r.iter().all(|&b| b == 0) || t.s.iter().all(|&b| b == 0) {
            return Err(ActorError::illegal_argument("zero r/s".into()));
        }
        // y_parity 0 or 1
        if t.y_parity != 0 && t.y_parity != 1 {
            return Err(ActorError::illegal_argument("invalid y_parity".into()));
        }
        // low-s on 32-byte left-padded S
        let mut s_padded = [0u8; 32];
        let start = 32 - t.s.len();
        s_padded[start..].copy_from_slice(&t.s);
        if Self::is_high_s(&s_padded) {
            return Err(ActorError::illegal_argument("high-s not allowed".into()));
        }
        Ok(())
    }

    fn is_high_s(sv: &[u8; 32]) -> bool {
        // n/2 for secp256k1
        const N: [u8; 32] = [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C,
            0xD0, 0x36, 0x41, 0x41,
        ];
        let mut n2 = [0u8; 32];
        let mut carry = 0u16;
        for i in (0..32).rev() {
            let v = (carry << 8) | N[i] as u16;
            n2[i] = (v / 2) as u8;
            carry = v % 2;
        }
        sv > &n2
    }

    fn recover_authority(
        rt: &impl Runtime,
        t: &fil_actors_evm_shared::eip7702::DelegationParam,
    ) -> Result<EthAddress, ActorError> {
        // message = keccak256(0x05 || rlp([chain_id, address(20), nonce]))
        let mut s = rlp::RlpStream::new_list(3);
        s.append(&t.chain_id);
        s.append(&t.address.as_ref());
        s.append(&t.nonce);
        let rlp_bytes = s.out().to_vec();
        let mut preimage = Vec::with_capacity(1 + rlp_bytes.len());
        preimage.push(0x05u8);
        preimage.extend_from_slice(&rlp_bytes);
        let mut hash32 = [0u8; 32];
        let h = rt.hash(SupportedHashes::Keccak256, &preimage);
        hash32.copy_from_slice(&h);

        // build 65-byte signature r||s||v (accept <=32-byte r/s; left-pad to 32)
        let mut sig = [0u8; 65];
        let r_start = 32 - t.r.len();
        sig[r_start..32].copy_from_slice(&t.r);
        let s_start = 64 - t.s.len();
        sig[s_start..64].copy_from_slice(&t.s);
        sig[64] = t.y_parity;
        let pubkey = rt
            .recover_secp_public_key(&hash32, &sig)
            .map_err(|e| ActorError::illegal_argument(format!("signature recovery failed: {e}")))?;
        let (keccak64, _len) = rt.hash_64(SupportedHashes::Keccak256, &pubkey[1..]);
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&keccak64[12..32]);
        Ok(EthAddress(addr))
    }

    /// Ethereum Account actor constructor.
    /// NOTE: This method is NOT currently called from anywhere, instead the FVM just deploys EthAccounts.
    pub fn constructor(rt: &impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        match rt
            .lookup_delegated_address(rt.message().receiver().id().unwrap())
            .map(|a| *a.payload())
        {
            Some(Payload::Delegated(da)) if da.namespace() == EAM_ACTOR_ID => {}
            Some(_) => {
                return Err(ActorError::illegal_argument(
                    "invalid target for EthAccount creation".to_string(),
                ));
            }
            None => {
                return Err(ActorError::illegal_argument(
                    "receiver must have a predictable address".to_string(),
                ));
            }
        }

        Ok(())
    }

    // Always succeeds, accepting any transfers.
    pub fn fallback(
        rt: &impl Runtime,
        method: MethodNum,
        _: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        if method >= FIRST_EXPORTED_METHOD_NUMBER {
            Ok(None)
        } else {
            Err(actor_error!(unhandled_message; "invalid method: {}", method))
        }
    }

    /// Validate EIP-7702 params and invoke the outer call.
    /// This is a scaffold; full validation and persistence are implemented in follow-ups.
    pub fn apply_and_call<RT>(
        rt: &RT,
        params: WithCodec<ApplyAndCallParams, DAG_CBOR>,
    ) -> Result<ApplyAndCallReturn, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_accept_any()?;
        Self::ensure_initialized(rt)?;

        let tuples = &params.0.list;
        if tuples.is_empty() {
            return Err(ActorError::illegal_argument("authorizationList must be non-empty".into()));
        }
        if tuples.len() > 64 {
            return Err(ActorError::illegal_argument("authorizationList exceeds tuple cap".into()));
        }
        // Duplicate authority rejection within a single message.
        let mut seen: HashSet<[u8; 20]> = HashSet::new();

        // Determine this actor's Ethereum address.
        let receiver_id = rt
            .message()
            .receiver()
            .id()
            .ok_or_else(|| ActorError::illegal_state("receiver not an id address".into()))?;
        let receiver_eth20 = match rt.lookup_delegated_address(receiver_id) {
            Some(Address { .. }) => {
                // Extract last 20 bytes from f4 address payload; assuming EAM namespace.
                match rt.lookup_delegated_address(receiver_id).map(|a| *a.payload()) {
                    Some(Payload::Delegated(d)) if d.namespace() == EAM_ACTOR_ID => {
                        let mut a = [0u8; 20];
                        let daddr = d.subaddress();
                        match daddr.len().cmp(&20) {
                            std::cmp::Ordering::Equal => a.copy_from_slice(daddr),
                            std::cmp::Ordering::Greater => {
                                a.copy_from_slice(&daddr[daddr.len() - 20..])
                            }
                            std::cmp::Ordering::Less => {
                                return Err(ActorError::illegal_state(
                                    "EthAccount has non-20B f4".into(),
                                ));
                            }
                        }
                        a
                    }
                    _ => {
                        return Err(ActorError::illegal_state(
                            "receiver has no EAM f4 address".into(),
                        ));
                    }
                }
            }
            None => return Err(ActorError::illegal_state("receiver not resolvable to f4".into())),
        };

        // Apply tuples that target this receiver only (WIP: single-authority per actor). Others are rejected.
        rt.transaction::<State, _, _>(|st, rt: &_| {
            for t in tuples.iter() {
                Self::validate_tuple(rt, t)?;
                let authority = Self::recover_authority(rt, t)?;
                let mut key = [0u8; 20];
                key.copy_from_slice(authority.as_ref());
                if !seen.insert(key) {
                    return Err(ActorError::illegal_argument(
                        "duplicate authority in authorizationList".into(),
                    ));
                }
                // Pre-existence policy: reject if authority resolves to EVM contract.
                if let Some(id) = rt.resolve_address(&Address::from(authority)) {
                    if let Some(code) = rt.get_actor_code_cid(&id) {
                        if matches!(
                            rt.resolve_builtin_actor_type(&code),
                            Some(fil_actors_runtime::runtime::builtins::Type::EVM)
                        ) {
                            return Err(ActorError::illegal_argument(
                                "authority is an EVM contract".into(),
                            ));
                        }
                    }
                }
                // Only support updating self for now (WIP behavior).
                if authority.as_ref() != receiver_eth20 {
                    return Err(ActorError::illegal_argument(
                        "authorization authority must equal receiver (WIP)".into(),
                    ));
                }
                // Nonce equality; absent treated as 0.
                if st.auth_nonce != t.nonce {
                    return Err(ActorError::illegal_argument(format!(
                        "nonce mismatch for receiver: expected {}, got {}",
                        st.auth_nonce, t.nonce
                    )));
                }
                // Update mapping: zero clears
                let is_zero_delegate = t.address.as_ref().iter().all(|&b| b == 0);
                st.delegate_to = if is_zero_delegate { None } else { Some(t.address) };
                // Bump nonce
                st.auth_nonce = st.auth_nonce.saturating_add(1);
                // Initialize storage root if absent
                if st.evm_storage_root == Cid::default() || st.evm_storage_root == EMPTY_ARR_CID {
                    st.evm_storage_root = EMPTY_ARR_CID;
                }
            }
            Ok(())
        })?;

        // Outer call: when the target resolves to an EVM contract, route via the
        // EVM InvokeContract entrypoint so the callee executes under the EVM
        // interpreter and can benefit from delegated CALL/EXTCODE* semantics.
        // For non-EVM targets or unresolved addresses, fall back to a plain
        // value transfer (METHOD_SEND) with no parameters.
        let call = &params.0.call;
        let to_fil: Address = call.to.into();
        // value is encoded as bytes; parse as big-endian U256 then into TokenAmount
        let value = {
            use fil_actors_evm_shared::uints::U256;
            let v = U256::from_big_endian(&call.value);
            TokenAmount::from(&v)
        };

        // Detect whether the target is an EVM builtin actor.
        let is_evm_target = match rt.resolve_address(&to_fil) {
            Some(id) => match rt.get_actor_code_cid(&id) {
                Some(code) => matches!(
                    rt.resolve_builtin_actor_type(&code),
                    Some(fil_actors_runtime::runtime::builtins::Type::EVM)
                ),
                None => false,
            },
            None => false,
        };

        // Route via InvokeEVM when the target is an EVM contract; otherwise use
        // a plain send (METHOD_SEND). In all cases, map the callee exit code
        // into the embedded status while keeping this actor's exit code OK.
        let res = if is_evm_target {
            let params_blk = IpldBlock::serialize_dag_cbor(&InvokeContractParams {
                input_data: call.input.clone(),
            })
            .map_err(|e| {
                ActorError::illegal_argument(format!("failed to encode outer EVM call params: {e}"))
            })?;
            let method_invoke_evm = frc42_dispatch::method_hash!("InvokeEVM");
            rt.send(&to_fil, method_invoke_evm, params_blk, value, None, SendFlags::default())
        } else {
            rt.send(&to_fil, 0, None, value, None, SendFlags::default())
        };

        use fvm_shared::error::ExitCode;
        match res {
            Ok(resp) => Ok(ApplyAndCallReturn {
                status: if resp.exit_code == ExitCode::OK { 1 } else { 0 },
                output_data: resp.return_data.map(|b| b.data).unwrap_or_default(),
            }),
            Err(_) => Ok(ApplyAndCallReturn { status: 0, output_data: Vec::new() }),
        }
    }
}

impl ActorCode for EthAccountActor {
    type Methods = Method;

    fn name() -> &'static str {
        "EVMAccount"
    }

    actor_dispatch! {
        Constructor => constructor,
        ApplyAndCall => apply_and_call,
        _ => fallback,
    }
}
