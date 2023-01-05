use std::iter;

use ext::init::{Exec4Params, Exec4Return};
use fil_actors_runtime::{actor_dispatch_unrestricted, AsActorError};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;

pub mod ext;

use {
    fil_actors_runtime::{
        actor_error,
        runtime::builtins::Type,
        runtime::{ActorCode, Runtime},
        ActorError, EAM_ACTOR_ID, INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    },
    fvm_ipld_encoding::{strict_bytes, tuple::*, RawBytes},
    fvm_shared::{
        address::{Address, Payload},
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
    // TODO: Do we want to use ExportedNums for all of these, per FRC-42?
    Create = 2,
    Create2 = 3,
}

/// Compute the a new actor address using the EVM's CREATE rules.
pub fn compute_address_create(rt: &impl Runtime, from: &EthAddress, nonce: u64) -> EthAddress {
    let mut stream = rlp::RlpStream::new();
    stream.begin_list(2).append(&&from.0[..]).append(&nonce);
    EthAddress(hash_20(rt, &stream.out()))
}

/// Compute the a new actor address using the EVM's CREATE2 rules.
pub fn compute_address_create2(
    rt: &impl Runtime,
    from: &EthAddress,
    salt: &[u8; 32],
    initcode: &[u8],
) -> EthAddress {
    let inithash = rt.hash(SupportedHashes::Keccak256, initcode);
    EthAddress(hash_20(rt, &[&[0xff], &from.0[..], salt, &inithash].concat()))
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EthAddress(#[serde(with = "strict_bytes")] pub [u8; 20]);

impl EthAddress {
    /// Returns true if the EthAddress refers to an address in the precompile range.
    /// [reference](https://github.com/filecoin-project/ref-fvm/issues/1164#issuecomment-1371304676)
    #[inline]
    fn is_precompile(&self) -> bool {
        // Exact index is not checked since it is unknown to the EAM what precompiles exist in the EVM actor.
        // 0 indexes of both ranges are not assignable as well but are _not_ precompile address.
        let [prefix, middle @ .., _index] = self.0;
        (prefix == 0xfe || prefix == 0x00) && middle == [0u8; 18]
    }

    /// Returns true if the EthAddress is an actor ID embedded in an eth address.
    #[inline]
    fn is_id(&self) -> bool {
        self.0[0] == 0xff && self.0[1..12].iter().all(|&i| i == 0)
    }

    #[inline]
    fn is_null(&self) -> bool {
        self.0 == [0; 20]
    }

    /// Returns true if the EthAddress is "reserved" (cannot be assigned by the EAM).
    #[inline]
    fn is_reserved(&self) -> bool {
        self.is_precompile() || self.is_id() || self.is_null()
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
    #[serde(with = "strict_bytes")]
    pub salt: [u8; 32],
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, PartialEq, Eq)]
pub struct Return {
    pub actor_id: ActorID,
    pub robust_address: Address,
    pub eth_address: EthAddress,
}

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
    // If the new address is reserved (an ID address, or a precompile), reject it. An attacker would
    // need to brute-force 96bits of a cryptographic hash and convince the target to use an attacker
    // chosen salt, but we might as well be safe.
    if new_addr.is_reserved() {
        return Err(ActorError::forbidden("cannot create address with a reserved prefix".into()));
    }
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
            IpldBlock::serialize_cbor(&init_params)?,
            rt.message().value_received(),
        )?
        .deserialize()?;

    Ok(Return::from_exec4(ret, new_addr))
}

fn resolve_caller(rt: &mut impl Runtime) -> Result<EthAddress, ActorError> {
    let caller_id = rt.message().caller().id().unwrap();
    Ok(match rt.lookup_delegated_address(caller_id).map(|a| *a.payload()) {
        Some(Payload::Delegated(addr)) if addr.namespace() == EAM_ACTOR_ID => EthAddress(
            addr.subaddress()
                .try_into()
                .context_code(ExitCode::USR_FORBIDDEN, "caller's eth address isn't valid")?,
        ),
        _ => {
            let mut bytes = [0u8; 20];
            bytes[0] = 0xff;
            bytes[12..].copy_from_slice(&caller_id.to_be_bytes());
            EthAddress(bytes)
        }
    })
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
        rt.validate_immediate_caller_is(iter::once(&SYSTEM_ACTOR_ADDR))
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
        let caller_addr = resolve_caller(rt)?;

        // CREATE logic
        let eth_addr = compute_address_create(rt, &caller_addr, params.nonce);

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

        // Try to lookup the caller's EVM address, but otherwise derive one from the ID address.
        let caller_addr = resolve_caller(rt)?;

        // Compute the CREATE2 address
        let eth_addr = compute_address_create2(rt, &caller_addr, &params.salt, &params.initcode);

        // send to init actor
        create_actor(rt, caller_addr, eth_addr, params.initcode)
    }
}

impl ActorCode for EamActor {
    type Methods = Method;
    actor_dispatch_unrestricted! {
        Constructor => constructor,
        Create => create,
        Create2 => create2,
    }
}

#[cfg(test)]
mod test {
    use fil_actors_runtime::test_utils::MockRuntime;
    use fvm_shared::error::ExitCode;

    use crate::compute_address_create2;

    use super::{compute_address_create, create_actor, EthAddress};

    #[test]
    fn test_create_actor_rejects() {
        let mut rt = MockRuntime::default();
        let mut creator = EthAddress([0; 20]);
        creator.0[0] = 0xff;
        creator.0[19] = 0x1;

        // Reject ID.
        let mut new_addr = EthAddress([0; 20]);
        new_addr.0[0] = 0xff;
        new_addr.0[18] = 0x20;
        new_addr.0[19] = 0x20;
        assert_eq!(
            ExitCode::USR_FORBIDDEN,
            create_actor(&mut rt, creator, new_addr, Vec::new()).unwrap_err().exit_code()
        );

        // Reject EVM Precompile.
        let mut new_addr = EthAddress([0; 20]);
        new_addr.0[19] = 0x20;
        assert_eq!(
            ExitCode::USR_FORBIDDEN,
            create_actor(&mut rt, creator, new_addr, Vec::new()).unwrap_err().exit_code()
        );

        // Reject Native Precompile.
        new_addr.0[0] = 0xfe;
        assert_eq!(
            ExitCode::USR_FORBIDDEN,
            create_actor(&mut rt, creator, new_addr, Vec::new()).unwrap_err().exit_code()
        );

        // Reject Null.
        let new_addr = EthAddress([0; 20]);
        assert_eq!(
            ExitCode::USR_FORBIDDEN,
            create_actor(&mut rt, creator, new_addr, Vec::new()).unwrap_err().exit_code()
        );
    }

    #[test]
    fn test_create_address() {
        let rt = MockRuntime::default();
        // check addresses against externally generated cases
        for (from, nonce, expected) in &[
            ([0u8; 20], 0u64, hex_literal::hex!("bd770416a3345f91e4b34576cb804a576fa48eb1")),
            ([0; 20], 200, hex_literal::hex!("a6b14387c1356b443061155e9c3e17f72c1777e5")),
            ([123; 20], 12345, hex_literal::hex!("809a9ab0471e78ee5100e96ca4d0828d1b97e2ba")),
        ] {
            let result = compute_address_create(&rt, &EthAddress(*from), *nonce);
            assert_eq!(result.0[..], expected[..]);
        }
    }

    #[test]
    fn test_create_address2() {
        let rt = MockRuntime::default();
        // check addresses against externally generated cases
        for (from, salt, initcode, expected) in &[
            (
                [0u8; 20],
                [0u8; 32],
                &b""[..],
                hex_literal::hex!("e33c0c7f7df4809055c3eba6c09cfe4baf1bd9e0"),
            ),
            (
                [0x99u8; 20],
                [0x42; 32],
                &b"foobar"[..],
                hex_literal::hex!("64425c93a90901271fa355c2bc462190803b97d4"),
            ),
        ] {
            let result = compute_address_create2(&rt, &EthAddress(*from), salt, initcode);
            assert_eq!(result.0[..], expected[..]);
        }
    }
}
