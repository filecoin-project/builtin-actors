use anyhow::{anyhow, Error};
use cid::multihash::Code;
use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::CborStore;
#[cfg(feature = "fake-proofs")]
use fvm_ipld_encoding::RawBytes;
use fvm_sdk as fvm;
use fvm_sdk::NO_DATA_BLOCK_ID;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::crypto::signature::{
    Signature, SECP_PUB_LEN, SECP_SIG_LEN, SECP_SIG_MESSAGE_HASH_SIZE,
};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::piece::PieceInfo;
use fvm_shared::randomness::RANDOMNESS_LENGTH;
use fvm_shared::sector::{
    AggregateSealVerifyProofAndInfos, RegisteredSealProof, ReplicaUpdateInfo, SealVerifyInfo,
    WindowPoStVerifyInfo,
};
use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum, Response};
use num_traits::FromPrimitive;
use serde::de::DeserializeOwned;
use serde::Serialize;
#[cfg(feature = "fake-proofs")]
use sha2::{Digest, Sha256};

use crate::runtime::actor_blockstore::ActorBlockstore;
use crate::runtime::builtins::Type;
use crate::runtime::{
    ActorCode, ConsensusFault, DomainSeparationTag, MessageInfo, Policy, Primitives, RuntimePolicy,
    Verifier,
};
use crate::{actor_error, ActorError, Runtime, SendError};

/// A runtime that bridges to the FVM environment through the FVM SDK.
pub struct FvmRuntime<B = ActorBlockstore> {
    blockstore: B,
    /// Indicates whether we are in a state transaction. During such, sending
    /// messages is prohibited.
    in_transaction: bool,
    /// Indicates that the caller has been validated.
    caller_validated: bool,
    /// The runtime policy
    policy: Policy,
}

impl Default for FvmRuntime {
    fn default() -> Self {
        FvmRuntime {
            blockstore: ActorBlockstore,
            in_transaction: false,
            caller_validated: false,
            policy: Policy::default(),
        }
    }
}

impl<B> FvmRuntime<B> {
    fn assert_not_validated(&mut self) -> Result<(), ActorError> {
        if self.caller_validated {
            return Err(actor_error!(
                assertion_failed,
                "Method must validate caller identity exactly once"
            ));
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn policy_mut(&mut self) -> &mut Policy {
        &mut self.policy
    }
}

/// A stub MessageInfo implementation performing FVM syscalls to obtain its fields.
struct FvmMessage;

impl MessageInfo for FvmMessage {
    fn caller(&self) -> Address {
        Address::new_id(fvm::message::caller())
    }

    fn origin(&self) -> Address {
        Address::new_id(fvm::message::origin())
    }

    fn receiver(&self) -> Address {
        Address::new_id(fvm::message::receiver())
    }

    fn value_received(&self) -> TokenAmount {
        fvm::message::value_received()
    }

    fn gas_premium(&self) -> TokenAmount {
        fvm::message::gas_premium()
    }

    fn nonce(&self) -> u64 {
        fvm::message::nonce()
    }
}

impl<B> Runtime for FvmRuntime<B>
where
    B: Blockstore,
{
    type Blockstore = B;

    fn network_version(&self) -> NetworkVersion {
        fvm::network::version()
    }

    fn message(&self) -> &dyn MessageInfo {
        &FvmMessage
    }

    fn curr_epoch(&self) -> ChainEpoch {
        fvm::network::curr_epoch()
    }

    fn validate_immediate_caller_accept_any(&mut self) -> Result<(), ActorError> {
        self.assert_not_validated()?;
        self.caller_validated = true;
        Ok(())
    }

    fn validate_immediate_caller_is<'a, I>(&mut self, addresses: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Address>,
    {
        self.assert_not_validated()?;
        let caller_addr = self.message().caller();
        if addresses.into_iter().any(|a| *a == caller_addr) {
            self.caller_validated = true;
            Ok(())
        } else {
            Err(actor_error!(forbidden;
                "caller {} is not one of supported", caller_addr
            ))
        }
    }

    fn validate_immediate_caller_type<'a, I>(&mut self, types: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Type>,
    {
        self.assert_not_validated()?;
        let caller_cid = {
            let caller_addr = self.message().caller();
            self.get_actor_code_cid(&caller_addr.id().unwrap())
                .expect("failed to lookup caller code")
        };

        match self.resolve_builtin_actor_type(&caller_cid) {
            Some(typ) if types.into_iter().any(|t| *t == typ) => {
                self.caller_validated = true;
                Ok(())
            }
            _ => Err(actor_error!(forbidden;
                    "caller cid type {} not one of supported", caller_cid)),
        }
    }

    fn current_balance(&self) -> TokenAmount {
        fvm::sself::current_balance()
    }

    fn actor_balance(&self, id: ActorID) -> Option<TokenAmount> {
        fvm::actor::balance_of(id)
    }

    fn resolve_address(&self, address: &Address) -> Option<ActorID> {
        fvm::actor::resolve_address(address)
    }

    fn lookup_delegated_address(&self, id: ActorID) -> Option<Address> {
        fvm::actor::lookup_delegated_address(id)
    }

    fn get_actor_code_cid(&self, id: &ActorID) -> Option<Cid> {
        fvm::actor::get_actor_code_cid(&Address::new_id(*id))
    }

    fn resolve_builtin_actor_type(&self, code_id: &Cid) -> Option<Type> {
        fvm::actor::get_builtin_actor_type(code_id).and_then(Type::from_i32)
    }

    fn get_code_cid_for_type(&self, typ: Type) -> Cid {
        fvm::actor::get_code_cid_for_type(typ as i32)
    }

    fn get_randomness_from_tickets(
        &self,
        personalization: DomainSeparationTag,
        rand_epoch: ChainEpoch,
        entropy: &[u8],
    ) -> Result<[u8; RANDOMNESS_LENGTH], ActorError> {
        fvm::rand::get_chain_randomness(personalization as i64, rand_epoch, entropy).map_err(|e| {
            match e {
                ErrorNumber::LimitExceeded => {
                    actor_error!(illegal_argument; "randomness lookback exceeded: {}", e)
                }
                e => actor_error!(assertion_failed; "get chain randomness failed with an unexpected error: {}", e),
            }
        })
    }

    fn get_randomness_from_beacon(
        &self,
        personalization: DomainSeparationTag,
        rand_epoch: ChainEpoch,
        entropy: &[u8],
    ) -> Result<[u8; RANDOMNESS_LENGTH], ActorError> {
        fvm::rand::get_beacon_randomness(personalization as i64, rand_epoch, entropy).map_err(|e| {
            match e {
                ErrorNumber::LimitExceeded => {
                    actor_error!(illegal_argument; "randomness lookback exceeded: {}", e)
                }
                e => actor_error!(assertion_failed; "get beacon randomness failed with an unexpected error: {}", e),
            }
        })
    }

    fn get_state_root(&self) -> Result<Cid, ActorError> {
        Ok(fvm::sself::root()?)
    }

    fn set_state_root(&mut self, root: &Cid) -> Result<(), ActorError> {
        Ok(fvm::sself::set_root(root)?)
    }

    fn transaction<S, RT, F>(&mut self, f: F) -> Result<RT, ActorError>
    where
        S: Serialize + DeserializeOwned,
        F: FnOnce(&mut S, &mut Self) -> Result<RT, ActorError>,
    {
        let state_cid = fvm::sself::root()
            .map_err(|_| actor_error!(illegal_argument; "failed to get actor root state CID"))?;

        let mut state = ActorBlockstore
            .get_cbor::<S>(&state_cid)
            .map_err(|_| actor_error!(illegal_argument; "failed to get actor state"))?
            .expect("State does not exist for actor state root");

        self.in_transaction = true;
        let result = f(&mut state, self);
        self.in_transaction = false;

        let ret = result?;
        let new_root = ActorBlockstore.put_cbor(&state, Code::Blake2b256)
            .map_err(|e| actor_error!(illegal_argument; "failed to write actor state in transaction: {}", e.to_string()))?;
        fvm::sself::set_root(&new_root)?;
        Ok(ret)
    }

    fn store(&self) -> &B {
        &self.blockstore
    }

    fn send(
        &self,
        to: &Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
        gas_limit: Option<u64>,
        flags: SendFlags,
    ) -> Result<Response, SendError> {
        if self.in_transaction {
            // Note: It's slightly improper to call this ErrorNumber::IllegalOperation,
            // since the error arises before getting to the VM.
            return Err(SendError(ErrorNumber::IllegalOperation));
        }

        fvm::send::send(to, method, params, value, gas_limit, flags).map_err(SendError)
    }

    fn new_actor_address(&mut self) -> Result<Address, ActorError> {
        Ok(fvm::actor::next_actor_address())
    }

    fn create_actor(
        &mut self,
        code_id: Cid,
        actor_id: ActorID,
        predictable_address: Option<Address>,
    ) -> Result<(), ActorError> {
        if self.in_transaction {
            return Err(
                actor_error!(assertion_failed; "create_actor is not allowed during transaction"),
            );
        }
        fvm::actor::create_actor(actor_id, &code_id, predictable_address).map_err(|e| match e {
            ErrorNumber::IllegalArgument => {
                ActorError::illegal_argument("failed to create actor".into())
            }
            ErrorNumber::Forbidden => ActorError::forbidden("actor already exists".into()),
            _ => actor_error!(assertion_failed; "create failed with unknown error: {}", e),
        })
    }

    fn delete_actor(&mut self, beneficiary: &Address) -> Result<(), ActorError> {
        if self.in_transaction {
            return Err(
                actor_error!(assertion_failed; "delete_actor is not allowed during transaction"),
            );
        }
        Ok(fvm::sself::self_destruct(beneficiary)?)
    }

    fn total_fil_circ_supply(&self) -> TokenAmount {
        fvm::network::total_fil_circ_supply()
    }

    fn charge_gas(&mut self, name: &'static str, compute: i64) {
        fvm::gas::charge(name, compute as u64)
    }

    fn base_fee(&self) -> TokenAmount {
        fvm::network::base_fee()
    }

    fn gas_available(&self) -> u64 {
        fvm::gas::available()
    }

    fn tipset_timestamp(&self) -> u64 {
        fvm::network::tipset_timestamp()
    }

    fn tipset_cid(&self, epoch: i64) -> Option<Cid> {
        fvm::network::tipset_cid(epoch).ok()
    }

    fn read_only(&self) -> bool {
        fvm::vm::read_only()
    }
}

impl<B> Primitives for FvmRuntime<B>
where
    B: Blockstore,
{
    fn verify_signature(
        &self,
        signature: &Signature,
        signer: &Address,
        plaintext: &[u8],
    ) -> Result<(), Error> {
        match fvm::crypto::verify_signature(signature, signer, plaintext) {
            Ok(true) => Ok(()),
            Ok(false) | Err(_) => Err(Error::msg("invalid signature")),
        }
    }

    fn hash_blake2b(&self, data: &[u8]) -> [u8; 32] {
        fvm::crypto::hash_blake2b(data)
    }

    fn compute_unsealed_sector_cid(
        &self,
        proof_type: RegisteredSealProof,
        pieces: &[PieceInfo],
    ) -> Result<Cid, Error> {
        // The only actor that invokes this (market actor) is generating the
        // exit code ErrIllegalArgument. We should probably move that here, or to the syscall itself.
        fvm::crypto::compute_unsealed_sector_cid(proof_type, pieces)
            .map_err(|e| anyhow!("failed to compute unsealed sector CID; exit code: {}", e))
    }

    fn hash(&self, hasher: SupportedHashes, data: &[u8]) -> Vec<u8> {
        fvm::crypto::hash_owned(hasher, data)
    }

    fn hash_64(&self, hasher: SupportedHashes, data: &[u8]) -> ([u8; 64], usize) {
        let mut buf = [0u8; 64];
        let len = fvm::crypto::hash_into(hasher, data, &mut buf);
        (buf, len)
    }

    fn recover_secp_public_key(
        &self,
        hash: &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
        signature: &[u8; SECP_SIG_LEN],
    ) -> Result<[u8; SECP_PUB_LEN], anyhow::Error> {
        fvm::crypto::recover_secp_public_key(hash, signature)
            .map_err(|e| anyhow!("failed to recover pubkey; exit code: {}", e))
    }
}

#[cfg(not(feature = "fake-proofs"))]
impl<B> Verifier for FvmRuntime<B>
where
    B: Blockstore,
{
    fn verify_seal(&self, vi: &SealVerifyInfo) -> Result<(), Error> {
        match fvm::crypto::verify_seal(vi) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::msg("invalid seal")),
            Err(e) => Err(anyhow!("failed to verify seal: {}", e)),
        }
    }

    fn verify_post(&self, verify_info: &WindowPoStVerifyInfo) -> Result<(), Error> {
        match fvm::crypto::verify_post(verify_info) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::msg("invalid post")),
            Err(e) => Err(anyhow!("failed to verify post: {}", e)),
        }
    }

    fn verify_consensus_fault(
        &self,
        h1: &[u8],
        h2: &[u8],
        extra: &[u8],
    ) -> Result<Option<ConsensusFault>, Error> {
        fvm::crypto::verify_consensus_fault(h1, h2, extra)
            .map_err(|e| anyhow!("failed to verify fault: {}", e))
    }

    fn batch_verify_seals(&self, batch: &[SealVerifyInfo]) -> anyhow::Result<Vec<bool>> {
        fvm::crypto::batch_verify_seals(batch)
            .map_err(|e| anyhow!("failed to verify batch seals: {}", e))
    }

    fn verify_aggregate_seals(
        &self,
        aggregate: &AggregateSealVerifyProofAndInfos,
    ) -> Result<(), Error> {
        match fvm::crypto::verify_aggregate_seals(aggregate) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::msg("invalid aggregate")),
            Err(e) => Err(anyhow!("failed to verify aggregate: {}", e)),
        }
    }

    fn verify_replica_update(&self, replica: &ReplicaUpdateInfo) -> Result<(), Error> {
        match fvm::crypto::verify_replica_update(replica) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::msg("invalid replica")),
            Err(e) => Err(anyhow!("failed to verify replica: {}", e)),
        }
    }
}

#[cfg(feature = "fake-proofs")]
impl<B> Verifier for FvmRuntime<B>
where
    B: Blockstore,
{
    fn verify_seal(&self, _vi: &SealVerifyInfo) -> Result<(), Error> {
        Ok(())
    }

    fn verify_post(&self, verify_info: &WindowPoStVerifyInfo) -> Result<(), Error> {
        let mut info = verify_info.clone();
        if info.proofs.len() != 1 {
            return Err(Error::msg("expected 1 proof entry"));
        }

        info.randomness.0[31] &= 0x3f;
        let mut hasher = Sha256::new();

        hasher.update(info.randomness.0);
        for si in info.challenged_sectors {
            hasher.update(RawBytes::serialize(si)?.bytes());
        }

        let expected_proof = hasher.finalize();

        if *verify_info.proofs[0].proof_bytes.as_slice() == expected_proof[..] {
            return Ok(());
        }

        Err(Error::msg("[fake-post-validation] window post was invalid"))
    }

    fn verify_consensus_fault(
        &self,
        _h1: &[u8],
        _h2: &[u8],
        _extra: &[u8],
    ) -> Result<Option<ConsensusFault>, Error> {
        Ok(None)
    }

    fn batch_verify_seals(&self, batch: &[SealVerifyInfo]) -> anyhow::Result<Vec<bool>> {
        Ok(batch.iter().map(|_| true).collect())
    }

    fn verify_aggregate_seals(
        &self,
        _aggregate: &AggregateSealVerifyProofAndInfos,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn verify_replica_update(&self, _replica: &ReplicaUpdateInfo) -> Result<(), Error> {
        Ok(())
    }
}

impl<B> RuntimePolicy for FvmRuntime<B>
where
    B: Blockstore,
{
    fn policy(&self) -> &Policy {
        &self.policy
    }
}

/// A convenience function that built-in actors can delegate their execution to.
///
/// The trampoline takes care of boilerplate:
///
/// 0.  Initialize logging if debugging is enabled.
/// 1.  Obtains the parameter data from the FVM by fetching the parameters block.
/// 2.  Obtains the method number for the invocation.
/// 3.  Creates an FVM runtime shim.
/// 4.  Invokes the target method.
/// 5a. In case of error, aborts the execution with the emitted exit code, or
/// 5b. In case of success, stores the return data as a block and returns the latter.
pub fn trampoline<C: ActorCode>(params: u32) -> u32 {
    init_logging();

    std::panic::set_hook(Box::new(|info| {
        fvm::vm::abort(ExitCode::USR_ASSERTION_FAILED.value(), Some(&format!("{}", info)))
    }));

    let method = fvm::message::method_number();
    let params = fvm::message::params_raw(params).expect("params block invalid");

    // Construct a new runtime.
    let mut rt = FvmRuntime::default();
    // Invoke the method, aborting if the actor returns an errored exit code.
    let ret = C::invoke_method(&mut rt, method, params)
        .unwrap_or_else(|err| fvm::vm::abort(err.exit_code().value(), Some(err.msg())));

    // Abort with "assertion failed" if the actor failed to validate the caller somewhere.
    // We do this after handling the error, because the actor may have encountered an error before
    // it even could validate the caller.
    if !rt.caller_validated {
        fvm::vm::abort(ExitCode::USR_ASSERTION_FAILED.value(), Some("failed to validate caller"))
    }

    // Then handle the return value.
    match ret {
        None => NO_DATA_BLOCK_ID,
        Some(ret_block) => fvm::ipld::put_block(ret_block.codec, ret_block.data.as_slice())
            .expect("failed to write result"),
    }
}

/// If debugging is enabled in the VM, installs a logger that sends messages to the FVM log syscall.
/// Messages are prefixed with "[LEVEL] ".
/// If debugging is not enabled, no logger will be installed which means that log!() and
/// similar calls will be dropped without either formatting args or making a syscall.
/// Note that, when debugging, the log syscalls will charge gas that wouldn't be charged
/// when debugging is not enabled.
///
/// Note: this is similar to fvm::debug::init_logging() from the FVM SDK, but
/// that doesn't work (at FVM SDK v2.2).
fn init_logging() {
    struct Logger;

    impl log::Log for Logger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            // Note the log system won't automatically call enabled() before this,
            // so it's canonical to check it here.
            // But logging must have been enabled at initialisation time in order for
            // the logger to be installed.
            // There's currently no use for dynamically disabling logging, so just skip checking.
            let msg = format!("[{}] {}", record.level(), record.args());
            fvm::debug::log(msg);
        }

        fn flush(&self) {}
    }

    if fvm::debug::enabled() {
        log::set_logger(&Logger).expect("failed to enable logging");
        log::set_max_level(log::LevelFilter::Trace);
    }
}
