use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    ActorError, Map, WithCodec, actor_dispatch, ActorDowncast, actor_error,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::{Deserialize_tuple, Serialize_tuple};
use fvm_ipld_encoding::{CBOR, DAG_CBOR};
use fvm_ipld_hamt::BytesKey;
use fvm_shared::chainid::ChainID;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_CONSTRUCTOR;
// NOTE: signature recovery TBD; placeholder returns error for now.
use fvm_shared::crypto::hash::SupportedHashes;
use num_derive::FromPrimitive;
use rlp::RlpStream;

// EIP-7702 authorization domain: inner tuple signatures must be over
// keccak256(0x05 || rlp([chain_id, address, nonce])).
const AUTH_MAGIC: u8 = 0x05;

mod state;
mod types;

pub use state::*;
pub use types::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(DelegatorActor);

#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    ApplyDelegations = 2,
    // FRC-42 exported methods (callable by EVM):
    LookupDelegate = frc42_dispatch::method_hash!("LookupDelegate"),
    GetStorageRoot = frc42_dispatch::method_hash!("GetStorageRoot"),
    PutStorageRoot = frc42_dispatch::method_hash!("PutStorageRoot"),
}

pub struct DelegatorActor;

impl DelegatorActor {
    pub fn constructor<RT: Runtime>(rt: &RT) -> Result<(), ActorError>
    where
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_accept_any()?;
        let st = State::new(rt.store())?;
        rt.create(&st)?;
        Ok(())
    }

    pub fn apply_delegations<RT: Runtime>(
        rt: &RT,
        params: WithCodec<ApplyDelegationsParams, DAG_CBOR>,
    ) -> Result<(), ActorError>
    where
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_accept_any()?;

        let list = params.0.list;
        if list.is_empty() {
            return Err(ActorError::illegal_argument("empty delegation list".into()));
        }
        let chain_id = rt.chain_id();
        rt.transaction::<State, _, _>(|st, rt| {
            let mut mapping = st.load_mapping(rt.store())?;
            let mut nonces = st.load_nonces(rt.store())?;
            for auth in list.into_iter() {
                validate_tuple(&auth, chain_id)?;
                let authority = recover_authority(rt, &auth)?;
                let key = BytesKey(authority.as_ref().to_vec());
                let current = nonces.get(&key).map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "hamt get nonce"))?;
                let current = current.unwrap_or(&0u64);
                if *current != auth.nonce {
                    return Err(ActorError::illegal_argument(format!(
                        "nonce mismatch for {}: expected {}, got {}",
                        authority, current, auth.nonce
                    )));
                }
                mapping.set(key.clone(), auth.address).map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "hamt set mapping"))?;
                nonces.set(key.clone(), auth.nonce.saturating_add(1)).map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "hamt set nonce"))?;
            }
            let new_mappings = mapping.flush().map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "flush mapping"))?;
            let new_nonces = nonces.flush().map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "flush nonces"))?;
            st.mappings = new_mappings;
            st.nonces = new_nonces;
            Ok(())
        })?;
        Ok(())
    }

    pub fn lookup_delegate<RT: Runtime>(
        rt: &RT,
        authority: WithCodec<LookupDelegateParams, CBOR>,
    ) -> Result<WithCodec<LookupDelegateReturn, CBOR>, ActorError>
    where
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        let mapping = st.load_mapping(rt.store())?;
        let key = BytesKey(authority.0.authority.as_ref().to_vec());
        let res = mapping.get(&key).map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "hamt get mapping"))?;
        Ok(LookupDelegateReturn { delegate: res.copied() }.into())
    }

    pub fn get_storage_root<RT: Runtime>(
        rt: &RT,
        params: WithCodec<GetStorageRootParams, CBOR>,
    ) -> Result<WithCodec<GetStorageRootReturn, CBOR>, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        let roots = st.load_storage_roots(rt.store())?;
        let key = BytesKey(params.0.authority.as_ref().to_vec());
        let res = roots.get(&key).map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "hamt get root"))?;
        Ok(GetStorageRootReturn { root: res.cloned() }.into())
    }

    pub fn put_storage_root<RT: Runtime>(
        rt: &RT,
        params: WithCodec<PutStorageRootParams, DAG_CBOR>,
    ) -> Result<(), ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        // Validate caller and restrict to EVM actor type.
        rt.validate_immediate_caller_accept_any()?;
        // Only EVM actors may write EOA storage roots.
        let caller = rt.message().caller();
        let caller_id = rt
            .resolve_address(&caller)
            .ok_or_else(|| ActorError::forbidden("cannot resolve caller".into()))?;
        let code = rt
            .get_actor_code_cid(&caller_id)
            .ok_or_else(|| ActorError::forbidden("caller has no code".into()))?;
        match rt.resolve_builtin_actor_type(&code) {
            Some(fil_actors_runtime::runtime::builtins::Type::EVM) => {}
            _ => return Err(ActorError::forbidden("only EVM actor may put storage root".into())),
        }
        let p = params.0;
        rt.transaction::<State, _, _>(|st, rt| {
            let mut roots = st.load_storage_roots(rt.store())?;
            let key = BytesKey(p.authority.as_ref().to_vec());
            roots.set(key, p.root).map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "hamt set root"))?;
            st.storage_roots = roots.flush().map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "flush roots"))?;
            Ok(())
        })
    }
}

impl ActorCode for DelegatorActor {
    type Methods = Method;
    fn name() -> &'static str { "Delegator" }
    actor_dispatch! {
        Constructor => constructor,
        ApplyDelegations => apply_delegations,
        LookupDelegate => lookup_delegate,
        GetStorageRoot => get_storage_root,
        PutStorageRoot => put_storage_root,
    }
}

fn validate_tuple(t: &DelegationParam, local_chain: ChainID) -> Result<(), ActorError> {
    // chain id 0 or local
    if t.chain_id != 0 && ChainID::from(t.chain_id) != local_chain {
        return Err(ActorError::illegal_argument("invalid chain id".into()));
    }
    // y_parity 0 or 1
    if t.y_parity != 0 && t.y_parity != 1 { return Err(ActorError::illegal_argument("invalid y_parity".into())); }
    // r/s non-zero
    if t.r.iter().all(|&b| b == 0) || t.s.iter().all(|&b| b == 0) {
        return Err(ActorError::illegal_argument("zero r/s".into()));
    }
    // low-s: s <= n/2
    if is_high_s(&t.s) { return Err(ActorError::illegal_argument("high-s not allowed".into())); }
    Ok(())
}

fn is_high_s(s: &[u8; 32]) -> bool {
    // secp256k1 curve order n
    // n = FFFFFFFF FFFFFFFF FFFFFFFF FFFFFFFE BAAEDCE6 AF48A03B BFD25E8C D0364141
    const N: [u8; 32] = [
        0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,
        0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFE,
        0xBA,0xAE,0xDC,0xE6,0xAF,0x48,0xA0,0x3B,
        0xBF,0xD2,0x5E,0x8C,0xD0,0x36,0x41,0x41
    ];
    // n/2
    let mut n2 = [0u8; 32];
    let mut carry = 0u16;
    for i in (0..32).rev() {
        let v = (carry << 8) | N[i] as u16;
        n2[i] = (v / 2) as u8;
        carry = v % 2;
    }
    s.gt(&n2)
}

fn recover_authority<RT: Runtime>(rt: &RT, t: &DelegationParam) -> Result<EthAddress, ActorError> {
    // message = keccak256(0x05 || rlp([chain_id, address(20), nonce]))
    let mut s = RlpStream::new_list(3);
    s.append(&t.chain_id);
    s.append(&t.address.as_ref());
    s.append(&t.nonce);
    let rlp_bytes = s.out().to_vec();
    let mut preimage = Vec::with_capacity(1 + rlp_bytes.len());
    preimage.push(AUTH_MAGIC);
    preimage.extend_from_slice(&rlp_bytes);

    let mut hash32 = [0u8; 32];
    let h = rt.hash(SupportedHashes::Keccak256, &preimage);
    hash32.copy_from_slice(&h);

    // build 65-byte signature r||s||v
    let mut sig = [0u8; 65];
    sig[..32].copy_from_slice(&t.r);
    sig[32..64].copy_from_slice(&t.s);
    sig[64] = t.y_parity;

    // recover uncompressed pubkey (65 bytes)
    let pubkey = rt
        .recover_secp_public_key(&hash32, &sig)
        .map_err(|e| ActorError::illegal_argument(format!("signature recovery failed: {e}")))?;

    // Compute address: keccak(pubkey[1..])[12..]
    let (mut keccak64, _len) = rt.hash_64(SupportedHashes::Keccak256, &pubkey[1..]);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&keccak64[12..32]);
    Ok(EthAddress(addr))
}
