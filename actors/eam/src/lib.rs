use std::iter;

use ext::init::{Exec4Params, Exec4Return};
use fvm_ipld_encoding::Cbor;
use rlp::Encodable;

pub mod ext;

use {
    fil_actors_runtime::{
        actor_error, cbor,
        runtime::builtins::Type,
        runtime::{ActorCode, Runtime},
        ActorError, EAM_ACTOR_ID, INIT_ACTOR_ADDR,
    },
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

#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    Create = 2,
    Create2 = 3,
    // CreateAccount = 4,
}

#[derive(Debug)]
/// Intermediate type between RLP encoding for CREATE
pub struct RlpCreateAddress {
    pub address: EthAddress,
    pub nonce: u64,
}

impl rlp::Encodable for RlpCreateAddress {
    fn rlp_append(&self, s: &mut rlp::RlpStream) {
        s.encoder().encode_value(&self.address.0);
        s.append(&self.nonce);
    }
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EthAddress(#[serde(with = "strict_bytes")] pub [u8; 20]);

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
    #[serde(with = "strict_bytes")]
    pub salt: [u8; 32],
}

impl Cbor for Create2Params {}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct InitAccountParams {
    #[serde(with = "strict_bytes")]
    pub pubkey: [u8; SECP_PUB_LEN],
}
impl Cbor for InitAccountParams {}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, PartialEq, Eq)]
pub struct Return {
    pub actor_id: ActorID,
    pub robust_address: Address,
    pub eth_address: EthAddress,
}

impl Cbor for Return {}

pub type CreateReturn = Return;
pub type Create2Return = Return;

impl Return {
    fn from_exec4(exec4: Exec4Return, eth_address: EthAddress) -> Self {
        Self {
            actor_id: exec4.id_address.id().unwrap(),
            robust_address: exec4.robust_address,
            eth_address,
        }
    }
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone)]
pub struct EvmConstructorParams {
    /// The actor's "creator" (specified by the EAM).
    pub creator: EthAddress,
    /// The initcode that will construct the new EVM actor.
    pub initcode: RawBytes,
}

/// hash of data with Keccack256, with first 12 bytes cropped
fn hash_20(rt: &impl Runtime, data: &[u8]) -> [u8; 20] {
    rt.hash(SupportedHashes::Keccak256, data)[12..32].try_into().unwrap()
}

fn create_actor(
    rt: &mut impl Runtime,
    creator: EthAddress,
    new_addr: EthAddress,
    initcode: Vec<u8>,
) -> Result<Return, ActorError> {
    let constructor_params =
        RawBytes::serialize(EvmConstructorParams { creator, initcode: initcode.into() })?;

    let init_params = Exec4Params {
        code_cid: rt.get_code_cid_for_type(Type::EVM),
        constructor_params,
        subaddress: new_addr.0.to_vec().into(),
    };

    let ret: ext::init::Exec4Return = rt
        .send(
            &INIT_ACTOR_ADDR,
            ext::init::EXEC4_METHOD,
            RawBytes::serialize(&init_params)?,
            rt.message().value_received(),
        )?
        .deserialize()?;

    Ok(Return::from_exec4(ret, new_addr))
}

pub struct EamActor;
impl EamActor {
    pub fn constructor(rt: &mut impl Runtime) -> Result<(), ActorError> {
        let actor_id = rt.resolve_address(&rt.message().receiver()).unwrap();
        if actor_id != EAM_ACTOR_ID {
            return Err(ActorError::forbidden(format!(
                "The Ethereum Address Manager must be deployed at {EAM_ACTOR_ID}, was deployed at {actor_id}"
            )));
        }
        rt.validate_immediate_caller_type(std::iter::once(&Type::Init))
    }

    /// Create a new contract per the EVM's CREATE rules.
    ///
    /// Permissions: May be called by any actor.
    pub fn create(rt: &mut impl Runtime, params: CreateParams) -> Result<CreateReturn, ActorError> {
        // TODO: this accepts a nonce from the user, so we _may_ want to limit it to specific
        // actors. However, we won't deploy over another actor anyways (those constraints are
        // enforced by the init actor and the FVM itself), so it shouldn't really be an issue in
        // practice.
        //
        // This allows _any_ actor to behave like an Ethereum account, so we'd prefer to keep it
        // open.
        rt.validate_immediate_caller_accept_any()?;

        let caller_id = rt.message().caller().id().unwrap();
        let caller_addr = match rt.lookup_address(caller_id).unwrap().payload() {
            Payload::Delegated(addr) => EthAddress(addr.subaddress().try_into().unwrap()),
            _ => unreachable!(),
        };

        // CREATE logic
        let rlp = RlpCreateAddress { address: caller_addr, nonce: params.nonce };
        let eth_addr = EthAddress(hash_20(rt, &rlp.rlp_bytes()));

        // send to init actor
        create_actor(rt, caller_addr, eth_addr, params.initcode)
    }

    /// Create a new contract per the EVM's CREATE2 rules.
    ///
    /// Permissions: May be called by any actor.
    pub fn create2(
        rt: &mut impl Runtime,
        params: Create2Params,
    ) -> Result<Create2Return, ActorError> {
        rt.validate_immediate_caller_accept_any()?;

        // CREATE2 logic
        let inithash = rt.hash(SupportedHashes::Keccak256, &params.initcode);

        // Try to lookup the caller's EVM address, but otherwise derive one from the ID address.
        let caller_id = rt.message().caller().id().unwrap();
        let caller_addr = match rt.lookup_address(caller_id).map(|a| *a.payload()) {
            Some(Payload::Delegated(addr)) if addr.namespace() == EAM_ACTOR_ID => {
                EthAddress(addr.subaddress().try_into().expect("eth address has an invalid size"))
            }
            _ => {
                let mut bytes = [0u8; 20];
                bytes[0] = 0xff;
                bytes[12..].copy_from_slice(&caller_id.to_be_bytes());
                EthAddress(bytes)
            }
        };

        let eth_addr = EthAddress(hash_20(
            rt,
            &[&[0xff], &caller_addr.0[..], &params.salt, &inithash].concat(),
        ));

        // send to init actor
        create_actor(rt, caller_addr, eth_addr, params.initcode)
    }

    pub fn create_account(
        rt: &mut impl Runtime,
        params: InitAccountParams,
    ) -> Result<Return, ActorError> {
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
        let eth_address = EthAddress(hash_20(rt, &params.pubkey[1..]));

        // TODO: Check reserved ranges (id, precompile, etc.).

        // Attempt to deploy an account there.
        // TODO
        create_actor(rt, EthAddress([0u8; 20]), eth_address, Vec::new()).ok();
        todo!()
    }
}

impl ActorCode for EamActor {
    fn invoke_method<RT>(
        rt: &mut RT,
        method: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        RT: Runtime,
    {
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt)?;
                Ok(RawBytes::default())
            }
            Some(Method::Create) => {
                Ok(RawBytes::serialize(Self::create(rt, cbor::deserialize_params(params)?)?)?)
            }
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
