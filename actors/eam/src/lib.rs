use std::iter;

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
        address::{Address, SECP_PUB_LEN},
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

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct CreateParams {
    #[serde(with = "serde_bytes")]
    pub initcode: Vec<u8>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct Create2Params {
    #[serde(with = "serde_bytes")]
    pub initcode: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub salt: [u8; 32],
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
    }

    pub fn create2<BS, RT>(rt: &mut RT, params: Create2Params) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(iter::once(&Type::EVM))?;
        // TODO: Implement CREATE2 logic.
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
        let eth_address = rt.hash(SupportedHashes::Keccak256, &params.pubkey[1..])[12..].to_owned();

        // TODO: Check reserved ranges (id, precompile, etc.).

        // Attempt to deploy an account there.
        let init_params = ext::init::Exec4Params {
            code_cid: todo!(),
            constructor_params: todo!(),
            subaddress: eth_address.into(),
        };

        let ret: ext::init::Exec4Return = rt
            .send(
                &INIT_ACTOR_ADDR,
                ext::init::EXEC4_METHOD,
                RawBytes::serialize(&init_params),
                rt.message().value_received(),
            )?
            .deserialize()?;

        Ok(InitAccountReturn {
            actor_id: ret.id_address.id().unwrap(),
            robust_address: ret.robust_address,
            eth_address: init_params.subaddress,
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
            Some(Method::Create) => Self::create_actor(rt),
            Some(Method::Create2) => Self::create_actor2(rt),
            Some(Method::InitAccount) => {
                RawBytes::serialize(Self::init_account(rt, cbor::deserialize_params(params)?))
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
