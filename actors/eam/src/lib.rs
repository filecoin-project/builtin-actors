use std::iter;

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
    fvm_ipld_encoding::{serde_bytes, tuple::*, RawBytes},
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

#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    Create = 2,
    Create2 = 3,
    CreateAccount = 4,
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
    #[serde(with = "serde_bytes")]
    pub initcode: Vec<u8>,
    pub nonce: u64,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct Create2Ret {
    #[serde(with = "serde_bytes")]
    pub f4_address: Vec<u8>,
    pub id_address: ActorID,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct Create2Params {
    #[serde(with = "serde_bytes")]
    pub initcode: Vec<u8>,
    /// TODO are we hashing with Little Endian bytes
    #[serde(with = "serde_bytes")]
    pub salt: [u8; 32],
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct Create2Ret {
    #[serde(with = "serde_bytes")]
    pub f4_address: Vec<u8>,
    pub id_address: ActorID,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct InitAccountParams {
    #[serde(with = "serde_bytes")]
    pub pubkey: [u8; SECP_PUB_LEN],
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct InitAccountReturn {
    pub actor_id: ActorID,
    pub robust_address: Address,
    #[serde(with = "serde_bytes")]
    pub eth_address: [u8; 20],
}

fn eth2f4(addr: &[u8]) -> Result<Address, ActorError> {
    Address::new_delegated(EAM_ACTOR_ID, addr)
        .map_err(|e| ActorError::illegal_argument(e.to_string()))
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
        // TODO: Implement CREATE logic.
        Self::assert_code_size(&params.initcode)?;

        let rlp = RlpCreateAddress {
            address: Self::get_eth_address(rt)?,
            nonce: params.nonce,
        };
        // rlp encoded bytes
        let mut addr = rt.hash_arr::<20>(SupportedHashes::Keccak256, &rlp.rlp_bytes().to_vec());

        eth2f4(&addr[12..32]);

        // TODO
        Ok((&addr[12..32]).to_vec().into())
    }

    pub fn create2<BS, RT>(rt: &mut RT, params: Create2Params) -> Result<Create2Ret, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(iter::once(&Type::EVM))?;
        Self::assert_code_size(&params.initcode)?;

        // hash the initial code bytes
        let inithash = rt.hash(SupportedHashes::Keccak256, &params.initcode);
        
        let eth_address = Self::get_eth_address(rt)?;

        let address_hash = rt.hash_arr::<20>(
            SupportedHashes::Keccak256,
            &[&[0xff], eth_address.as_slice(), &params.salt, &inithash].concat(),
        );

        // TODO
        Ok(Create2Ret { f4_address: Address::new_delegated(ETH, &address_hash), id_address: 0})
    }

    fn get_eth_address<BS, RT>(rt: &RT) -> Result<[u8; 20], ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        let addr = rt.lookup_address(rt.message().caller().id().unwrap());

        match addr.map(|a| a.payload()) {
            Some(Payload::Delegated(eth)) => Ok(eth.subaddress().try_into().unwrap()),
            _ => Err(ActorError::assertion_failed(
                "All FEVM actors should have a predictable address".to_string(),
            )),
        }
    }

    fn assert_code_size(code: &[u8]) -> Result<(), ActorError> {
        (code.len() == MAX_CODE_SIZE).then(|| ()).ok_or(ActorError::illegal_argument("EVM bytecode larger than 24kB".to_string()))
    }

    pub fn init_account<BS, RT>(
        rt: &mut RT,
        params: InitAccountParams,
    ) -> Result<InitAccountReturn, ActorError>
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
        let eth_address = rt.hash_arr(SupportedHashes::Keccak256, &params.pubkey[1..]);

        // TODO: Check reserved ranges (id, precompile, etc.).

        // Attempt to deploy an account there.
        let init_params = ext::init::Exec4Params {
            code_cid: todo!(),
            constructor_params: todo!(),
            subaddress: eth_address.to_vec().into(),
        };

        let ret: ext::init::Exec4Return = rt
            .send(
                &INIT_ACTOR_ADDR,
                ext::init::EXEC4_METHOD,
                RawBytes::serialize(&init_params)?,
                rt.message().value_received(),
            )?
            .deserialize()?;

        Ok(InitAccountReturn {
            actor_id: ret.id_address.id().unwrap(),
            robust_address: ret.robust_address,
            eth_address,
        })
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
            Some(Method::Create2) => Ok(RawBytes::serialize(Self::create2(rt, cbor::deserialize_params(params)?)?)?),
            Some(Method::InitAccount) => {
                RawBytes::serialize(Self::init_account(rt, cbor::deserialize_params(params)?))
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
