use std::iter;

use ext::init::{Exec4Params, Exec4Return};
use rlp::Encodable;

mod ext;
mod state;

use {
    fil_actors_runtime::{
        actor_error, cbor,
        runtime::builtins::Type,
        runtime::{ActorCode, Runtime},
        ActorError, EAM_ACTOR_ID, INIT_ACTOR_ADDR,
    },
    fvm_ipld_blockstore::Blockstore,
    fvm_ipld_encoding::{strict_bytes, tuple::*, RawBytes},
    fvm_shared::{
        address::{Address, Payload, SECP_PUB_LEN},
        crypto::hash::SupportedHashes,
        ActorID, MethodNum, METHOD_CONSTRUCTOR,
    },
    num_derive::FromPrimitive,
    num_traits::FromPrimitive,
};

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(EamActor);

/// Maximum allowed EVM bytecode size.
/// The contract code size limit is 24kB.
const MAX_CODE_SIZE: usize = 24 << 10;

/// TODO double check this
const Keccack256_ZERO_INPUT_HASH: [u8; 32] =
    hex_literal::hex!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");

#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    Create = 2,
    Create2 = 3,
    // CreateAccount = 4,
}

/// Intermediate type between RLP encoding for CREATE
struct RlpCreateAddress {
    address: [u8; 20],
    nonce: u64,
}

impl rlp::Encodable for RlpCreateAddress {
    fn rlp_append(&self, s: &mut rlp::RlpStream) {
        // TODO check if this is correct... I cant read go code well enough to tell
        s.encoder().encode_value(&self.address);
        s.append(&self.nonce);
    }
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct CreateParams {
    #[serde(with = "strict_bytes")]
    pub initcode: Vec<u8>,
    pub nonce: u64,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct Create2Params {
    #[serde(with = "strict_bytes")]
    pub initcode: Vec<u8>,
    /// TODO are we hashing with Little Endian bytes
    #[serde(with = "strict_bytes")]
    pub salt: [u8; 32],
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct InitAccountParams {
    #[serde(with = "strict_bytes")]
    pub pubkey: [u8; SECP_PUB_LEN],
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct EamReturn {
    pub actor_id: ActorID,
    pub robust_address: Address,
    #[serde(with = "strict_bytes")]
    pub eth_address: [u8; 20],
}

impl EamReturn {
    fn from_exec4(exec4: Exec4Return, eth_address: [u8; 20]) -> Self {
        Self {
            actor_id: exec4.id_address.id().unwrap(),
            robust_address: exec4.robust_address,
            eth_address,
        }
    }
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct EvmConstructorParams {
    /// The actor's "creator" (specified by the EAM).
    pub creator: [u8; 20],
    /// The initcode that will construct the new EVM actor.
    pub initcode: RawBytes,
}

fn assert_code_size(code: &[u8]) -> Result<(), ActorError> {
    (code.len() == MAX_CODE_SIZE).then(|| ()).ok_or(ActorError::illegal_argument(
        "Supplied EVM bytecode is larger than 24kB.".to_string(),
    ))
}

/// hash of data with Keccack256, with first 12 bytes cropped
fn hash_20<BS, RT>(rt: &RT, data: &[u8]) -> [u8; 20]
where
    BS: Blockstore + Clone,
    RT: Runtime<BS>,
{
    let buf = rt.hash(SupportedHashes::Keccak256, data);
    buf[12..32].try_into().unwrap()
}

fn create_actor<BS, RT>(
    rt: &mut RT,
    creator: [u8; 20],
    new_addr: [u8; 20],
    initcode: Vec<u8>,
) -> Result<RawBytes, ActorError>
where
    BS: Blockstore + Clone,
    RT: Runtime<BS>,
{
    let constructor_params =
        RawBytes::serialize(EvmConstructorParams { creator, initcode: initcode.into() })?;

    let init_params = Exec4Params {
        code_cid: rt.get_code_cid_for_type(Type::EVM),
        constructor_params,
        subaddress: new_addr.to_vec().into(),
    };

    let ret: ext::init::Exec4Return = rt
        .send(
            &INIT_ACTOR_ADDR,
            ext::init::EXEC4_METHOD,
            RawBytes::serialize(&init_params)?,
            rt.message().value_received(),
        )?
        .deserialize()?;

    Ok(RawBytes::serialize(EamReturn::from_exec4(ret, new_addr))?)
}

/// lookup caller's raw ETH address
fn get_caller_address<BS, RT>(rt: &RT) -> Result<[u8; 20], ActorError>
where
    BS: Blockstore + Clone,
    RT: Runtime<BS>,
{
    let caller_id = rt.message().caller().id().unwrap();

    let addr = rt.lookup_address(caller_id);

    match addr.map(|a| *a.payload()) {
        Some(Payload::Delegated(eth)) => Ok(eth.subaddress().try_into().unwrap()),
        _ => Err(ActorError::assertion_failed(
            "All FEVM actors should have a delegated address".to_string(),
        )),
    }
}

pub struct EamActor;
impl EamActor {
    pub fn constructor<BS, RT>(rt: &mut RT) -> Result<(), ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        let actor_id = rt.resolve_address(&rt.message().receiver()).unwrap();
        if actor_id != EAM_ACTOR_ID {
            return Err(ActorError::forbidden(format!(
                "The Ethereum Address Manager must be deployed at {EAM_ACTOR_ID}, was deployed at {actor_id}"
            )));
        }
        rt.validate_immediate_caller_accept_any()
    }

    pub fn create<BS, RT>(rt: &mut RT, params: CreateParams) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(iter::once(&Type::EVM))?;
        assert_code_size(&params.initcode)?;

        let caller_addr = get_caller_address(rt)?;
        // CREATE logic
        let rlp = RlpCreateAddress { address: caller_addr, nonce: params.nonce };
        let eth_addr = hash_20(rt, &rlp.rlp_bytes().to_vec());

        // send to init actor
        create_actor(rt, caller_addr, eth_addr, params.initcode)
    }

    pub fn create2<BS, RT>(rt: &mut RT, params: Create2Params) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(iter::once(&Type::EVM))?;
        assert_code_size(&params.initcode)?;

        // CREATE2 logic
        let inithash = rt.hash(SupportedHashes::Keccak256, &params.initcode);

        let caller_addr = get_caller_address(rt)?;

        let eth_addr =
            hash_20(rt, &[&[0xff], caller_addr.as_slice(), &params.salt, &inithash].concat());

        // send to init actor
        create_actor(rt, caller_addr, eth_addr, params.initcode)
    }

    pub fn create_account<BS, RT>(
        rt: &mut RT,
        params: InitAccountParams,
    ) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        // First, validate that we're receiving this message from the filecoin account that maps to
        // this ethereum account.
        //
        // We don't need to validate that the _key_ is well formed or anything, because the fact
        // that we're receiving a message from the account proves that to be the case anyways.
        //
        // TODO: allow off-chain deployment!
        let key_addr = Address::new_secp256k1(&params.pubkey)
            .map_err(|e| ActorError::illegal_argument(format!("not a valid public key: {e}")))?;

        rt.validate_immediate_caller_is(iter::once(&key_addr))?;

        // Compute the equivalent eth address
        let eth_address = hash_20(rt, &params.pubkey[1..]);

        // TODO: Check reserved ranges (id, precompile, etc.).

        // Attempt to deploy an account there.
        // TODO
        create_actor(rt, [0u8; 20], eth_address, Vec::new()).ok();
        todo!()
    }
}

impl ActorCode for EamActor {
    fn invoke_method<BS, RT>(
        rt: &mut RT,
        method: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt)?;
                Ok(RawBytes::default())
            }
            Some(Method::Create) => Self::create(rt, cbor::deserialize_params(params)?),
            Some(Method::Create2) => {
                Ok(RawBytes::serialize(Self::create2(rt, cbor::deserialize_params(params)?)?)?)
            }
            // Some(Method::CreateAccount) => {
            //     Self::create_account(rt, cbor::deserialize_params(params)?)
            // }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
