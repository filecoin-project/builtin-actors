// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use core::fmt;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::Rc;

use anyhow::anyhow;
use cid::multihash::{Code, Multihash as OtherMultihash};
use cid::Cid;
use fvm_ipld_blockstore::{Blockstore, MemoryBlockstore};
use fvm_ipld_encoding::de::DeserializeOwned;
use fvm_ipld_encoding::CborStore;
use fvm_shared::address::Payload;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::commcid::{FIL_COMMITMENT_SEALED, FIL_COMMITMENT_UNSEALED};
use fvm_shared::consensus::ConsensusFault;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::crypto::signature::{
    Signature, SECP_PUB_LEN, SECP_SIG_LEN, SECP_SIG_MESSAGE_HASH_SIZE,
};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::piece::PieceInfo;
use fvm_shared::randomness::RANDOMNESS_LENGTH;
use fvm_shared::sector::{
    AggregateSealVerifyInfo, AggregateSealVerifyProofAndInfos, RegisteredSealProof,
    ReplicaUpdateInfo, SealVerifyInfo, WindowPoStVerifyInfo,
};
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum, Response};

use multihash::derive::Multihash;
use multihash::MultihashDigest;

use rand::prelude::*;
use serde::Serialize;

use crate::runtime::builtins::Type;
use crate::runtime::{
    ActorCode, DomainSeparationTag, MessageInfo, Policy, Primitives, Runtime, RuntimePolicy,
    Verifier, EMPTY_ARR_CID,
};
use crate::{actor_error, ActorError, SendError};
use libsecp256k1::{recover, Message, RecoveryId, Signature as EcsdaSignature};

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::chainid::ChainID;
use fvm_shared::event::ActorEvent;
use fvm_shared::sys::SendFlags;

lazy_static::lazy_static! {
    pub static ref SYSTEM_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/system");
    pub static ref INIT_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/init");
    pub static ref CRON_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/cron");
    pub static ref ACCOUNT_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/account");
    pub static ref POWER_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/storagepower");
    pub static ref MINER_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/storageminer");
    pub static ref MARKET_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/storagemarket");
    pub static ref PAYCH_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/paymentchannel");
    pub static ref MULTISIG_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/multisig");
    pub static ref REWARD_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/reward");
    pub static ref VERIFREG_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/verifiedregistry");
    pub static ref DATACAP_TOKEN_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/datacap");
    pub static ref PLACEHOLDER_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/placeholder");
    pub static ref EVM_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/evm");
    pub static ref EAM_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/eam");
    pub static ref ETHACCOUNT_ACTOR_CODE_ID: Cid = make_identity_cid(b"fil/test/ethaccount");

    pub static ref ACTOR_TYPES: BTreeMap<Cid, Type> = {
        let mut map = BTreeMap::new();
        map.insert(*SYSTEM_ACTOR_CODE_ID, Type::System);
        map.insert(*INIT_ACTOR_CODE_ID, Type::Init);
        map.insert(*CRON_ACTOR_CODE_ID, Type::Cron);
        map.insert(*ACCOUNT_ACTOR_CODE_ID, Type::Account);
        map.insert(*POWER_ACTOR_CODE_ID, Type::Power);
        map.insert(*MINER_ACTOR_CODE_ID, Type::Miner);
        map.insert(*MARKET_ACTOR_CODE_ID, Type::Market);
        map.insert(*PAYCH_ACTOR_CODE_ID, Type::PaymentChannel);
        map.insert(*MULTISIG_ACTOR_CODE_ID, Type::Multisig);
        map.insert(*REWARD_ACTOR_CODE_ID, Type::Reward);
        map.insert(*VERIFREG_ACTOR_CODE_ID, Type::VerifiedRegistry);
        map.insert(*DATACAP_TOKEN_ACTOR_CODE_ID, Type::DataCap);
        map.insert(*PLACEHOLDER_ACTOR_CODE_ID, Type::Placeholder);
        map.insert(*EVM_ACTOR_CODE_ID, Type::EVM);
        map.insert(*EAM_ACTOR_CODE_ID, Type::EAM);
        map.insert(*ETHACCOUNT_ACTOR_CODE_ID, Type::EthAccount);
        map
    };
    pub static ref ACTOR_CODES: BTreeMap<Type, Cid> = [
        (Type::System, *SYSTEM_ACTOR_CODE_ID),
        (Type::Init, *INIT_ACTOR_CODE_ID),
        (Type::Cron, *CRON_ACTOR_CODE_ID),
        (Type::Account, *ACCOUNT_ACTOR_CODE_ID),
        (Type::Power, *POWER_ACTOR_CODE_ID),
        (Type::Miner, *MINER_ACTOR_CODE_ID),
        (Type::Market, *MARKET_ACTOR_CODE_ID),
        (Type::PaymentChannel, *PAYCH_ACTOR_CODE_ID),
        (Type::Multisig, *MULTISIG_ACTOR_CODE_ID),
        (Type::Reward, *REWARD_ACTOR_CODE_ID),
        (Type::VerifiedRegistry, *VERIFREG_ACTOR_CODE_ID),
        (Type::DataCap, *DATACAP_TOKEN_ACTOR_CODE_ID),
        (Type::Placeholder, *PLACEHOLDER_ACTOR_CODE_ID),
        (Type::EVM, *EVM_ACTOR_CODE_ID),
        (Type::EAM, *EAM_ACTOR_CODE_ID),
        (Type::EthAccount, *ETHACCOUNT_ACTOR_CODE_ID),
    ]
    .into_iter()
    .collect();
    pub static ref NON_SINGLETON_CODES: BTreeMap<Cid, ()> = {
        let mut map = BTreeMap::new();
        map.insert(*ACCOUNT_ACTOR_CODE_ID, ());
        map.insert(*PAYCH_ACTOR_CODE_ID, ());
        map.insert(*MULTISIG_ACTOR_CODE_ID, ());
        map.insert(*MINER_ACTOR_CODE_ID, ());
        map.insert(*PLACEHOLDER_ACTOR_CODE_ID, ());
        map.insert(*EVM_ACTOR_CODE_ID, ());
        map.insert(*ETHACCOUNT_ACTOR_CODE_ID, ());
        map
    };
}

const IPLD_RAW: u64 = 0x55;

/// Returns an identity CID for bz.
pub fn make_identity_cid(bz: &[u8]) -> Cid {
    Cid::new_v1(IPLD_RAW, OtherMultihash::wrap(0, bz).expect("name too long"))
}

/// Enable logging to enviornment. Returns error if already init.
pub fn init_logging() -> Result<(), log::SetLoggerError> {
    pretty_env_logger::try_init()
}

pub struct MockRuntime<BS = MemoryBlockstore> {
    pub epoch: RefCell<ChainEpoch>,
    pub miner: Address,
    pub base_fee: RefCell<TokenAmount>,
    pub chain_id: ChainID,
    pub id_addresses: RefCell<HashMap<Address, Address>>,
    pub delegated_addresses: RefCell<HashMap<ActorID, Address>>,
    pub actor_code_cids: RefCell<HashMap<Address, Cid>>,
    pub new_actor_addr: RefCell<Option<Address>>,
    pub receiver: Address,
    pub caller: RefCell<Address>,
    pub caller_type: RefCell<Cid>,
    pub origin: RefCell<Address>,
    pub value_received: RefCell<TokenAmount>,
    #[allow(clippy::type_complexity)]
    pub hash_func: Box<dyn Fn(SupportedHashes, &[u8]) -> ([u8; 64], usize)>,
    #[allow(clippy::type_complexity)]
    pub recover_secp_pubkey_fn: Box<
        dyn Fn(
            &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
            &[u8; SECP_SIG_LEN],
        ) -> Result<[u8; SECP_PUB_LEN], ()>,
    >,
    pub network_version: NetworkVersion,

    // Actor State
    pub state: RefCell<Option<Cid>>,
    pub balance: RefCell<TokenAmount>,

    // VM Impl
    pub in_call: RefCell<bool>,
    pub store: Rc<BS>,
    pub in_transaction: RefCell<bool>,

    // Expectations
    pub expectations: RefCell<Expectations>,

    // policy
    pub policy: Policy,

    pub circulating_supply: RefCell<TokenAmount>,

    pub gas_limit: u64,
    pub gas_premium: TokenAmount,
    pub actor_balances: HashMap<ActorID, TokenAmount>,
    pub tipset_timestamp: u64,
    pub tipset_cids: Vec<Cid>,
}

#[derive(Default)]
pub struct Expectations {
    pub expect_validate_caller_any: bool,
    pub expect_validate_caller_addr: Option<Vec<Address>>,
    pub expect_validate_caller_f4_namespace: Option<Vec<u64>>,
    pub expect_validate_caller_type: Option<Vec<Type>>,
    pub expect_sends: VecDeque<ExpectedMessage>,
    pub expect_create_actor: Option<ExpectCreateActor>,
    pub expect_delete_actor: Option<Address>,
    pub expect_verify_sigs: VecDeque<ExpectedVerifySig>,
    pub expect_verify_seal: Option<ExpectVerifySeal>,
    pub expect_verify_post: Option<ExpectVerifyPoSt>,
    pub expect_compute_unsealed_sector_cid: VecDeque<ExpectComputeUnsealedSectorCid>,
    pub expect_verify_consensus_fault: Option<ExpectVerifyConsensusFault>,
    pub expect_get_randomness_tickets: VecDeque<ExpectRandomness>,
    pub expect_get_randomness_beacon: VecDeque<ExpectRandomness>,
    pub expect_batch_verify_seals: Option<ExpectBatchVerifySeals>,
    pub expect_aggregate_verify_seals: Option<ExpectAggregateVerifySeals>,
    pub expect_replica_verify: Option<ExpectReplicaVerify>,
    pub expect_gas_charge: VecDeque<i64>,
    pub expect_gas_available: VecDeque<u64>,
    pub expect_emitted_events: VecDeque<ActorEvent>,
    skip_verification_on_drop: bool,
}

impl Expectations {
    fn reset(&mut self) {
        self.skip_verification_on_drop = true;
        *self = Default::default();
    }

    fn verify(&mut self) {
        // If we don't reset them, we'll try to re-verify on drop. If something fails, we'll panic
        // twice and abort making the tests difficult to debug.
        self.skip_verification_on_drop = true;
        let this = std::mem::take(self);

        assert!(!this.expect_validate_caller_any, "expected ValidateCallerAny, not received");
        assert!(
            this.expect_validate_caller_addr.is_none(),
            "expected ValidateCallerAddr {:?}, not received",
            this.expect_validate_caller_addr
        );
        assert!(
            this.expect_validate_caller_f4_namespace.is_none(),
            "expected ValidateCallerF4Namespace {:?}, not received",
            this.expect_validate_caller_f4_namespace
        );
        assert!(
            this.expect_validate_caller_type.is_none(),
            "expected ValidateCallerType {:?}, not received",
            this.expect_validate_caller_type
        );
        assert!(
            this.expect_sends.is_empty(),
            "expected all message to be send, unsent messages {:?}",
            this.expect_sends
        );
        assert!(
            this.expect_create_actor.is_none(),
            "expected actor to be created, uncreated actor: {:?}",
            this.expect_create_actor
        );
        assert!(
            this.expect_delete_actor.is_none(),
            "expected actor to be deleted: {:?}",
            this.expect_delete_actor
        );
        assert!(
            this.expect_verify_sigs.is_empty(),
            "expect_verify_sigs: {:?}, not received",
            this.expect_verify_sigs
        );
        assert!(
            this.expect_verify_seal.is_none(),
            "expect_verify_seal {:?}, not received",
            this.expect_verify_seal
        );
        assert!(
            this.expect_verify_post.is_none(),
            "expect_verify_post {:?}, not received",
            this.expect_verify_post
        );
        assert!(
            this.expect_compute_unsealed_sector_cid.is_empty(),
            "expect_compute_unsealed_sector_cid: {:?}, not received",
            this.expect_compute_unsealed_sector_cid
        );
        assert!(
            this.expect_verify_consensus_fault.is_none(),
            "expect_verify_consensus_fault {:?}, not received",
            this.expect_verify_consensus_fault
        );
        assert!(
            this.expect_get_randomness_tickets.is_empty(),
            "expect_get_randomness_tickets {:?}, not received",
            this.expect_get_randomness_tickets
        );
        assert!(
            this.expect_get_randomness_beacon.is_empty(),
            "expect_get_randomness_beacon {:?}, not received",
            this.expect_get_randomness_beacon
        );
        assert!(
            this.expect_batch_verify_seals.is_none(),
            "expect_batch_verify_seals {:?}, not received",
            this.expect_batch_verify_seals
        );
        assert!(
            this.expect_aggregate_verify_seals.is_none(),
            "expect_aggregate_verify_seals {:?}, not received",
            this.expect_aggregate_verify_seals
        );
        assert!(
            this.expect_replica_verify.is_none(),
            "expect_replica_verify {:?}, not received",
            this.expect_replica_verify
        );
        assert!(
            this.expect_gas_charge.is_empty(),
            "expect_gas_charge {:?}, not received",
            this.expect_gas_charge
        );
        assert!(
            this.expect_gas_available.is_empty(),
            "expect_gas_available {:?}, not received",
            this.expect_gas_available
        );
        assert!(
            this.expect_emitted_events.is_empty(),
            "expect_emitted_events {:?}, not received",
            this.expect_emitted_events
        );
    }
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<BS> MockRuntime<BS> {
    pub fn new(store: BS) -> Self {
        Self {
            epoch: Default::default(),
            miner: Address::new_id(0),
            base_fee: Default::default(),
            chain_id: ChainID::from(0),
            id_addresses: Default::default(),
            delegated_addresses: Default::default(),
            actor_code_cids: Default::default(),
            new_actor_addr: Default::default(),
            receiver: Address::new_id(0),
            caller: RefCell::new(Address::new_id(0)),
            caller_type: Default::default(),
            origin: RefCell::new(Address::new_id(0)),
            value_received: Default::default(),
            hash_func: Box::new(hash),
            recover_secp_pubkey_fn: Box::new(recover_secp_public_key),
            network_version: NetworkVersion::V0,
            state: Default::default(),
            balance: Default::default(),
            in_call: Default::default(),
            store: Rc::new(store),
            in_transaction: Default::default(),
            expectations: Default::default(),
            policy: Default::default(),
            circulating_supply: Default::default(),
            gas_limit: 10_000_000_000u64,
            gas_premium: Default::default(),
            actor_balances: Default::default(),
            tipset_timestamp: Default::default(),
            tipset_cids: Default::default(),
        }
    }
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct ExpectCreateActor {
    pub code_id: Cid,
    pub actor_id: ActorID,
    pub predictable_address: Option<Address>,
}

#[derive(Clone, Debug)]
pub struct ExpectedMessage {
    pub to: Address,
    pub method: MethodNum,
    pub params: Option<IpldBlock>,
    pub value: TokenAmount,
    pub gas_limit: Option<u64>,
    pub send_flags: SendFlags,

    // returns from applying expectedMessage
    pub send_return: Option<IpldBlock>,
    pub exit_code: ExitCode,
    pub send_error: Option<ErrorNumber>,
}

#[derive(Debug)]
pub struct ExpectedVerifySig {
    pub sig: Signature,
    pub signer: Address,
    pub plaintext: Vec<u8>,
    pub result: Result<(), anyhow::Error>,
}

#[derive(Clone, Debug)]
pub struct ExpectVerifySeal {
    seal: SealVerifyInfo,
    exit_code: ExitCode,
}

#[derive(Clone, Debug)]
pub struct ExpectVerifyPoSt {
    post: WindowPoStVerifyInfo,
    exit_code: ExitCode,
}

#[derive(Clone, Debug)]
pub struct ExpectVerifyConsensusFault {
    require_correct_input: bool,
    block_header_1: Vec<u8>,
    block_header_2: Vec<u8>,
    block_header_extra: Vec<u8>,
    fault: Option<ConsensusFault>,
    exit_code: ExitCode,
}

#[derive(Clone, Debug)]
pub struct ExpectComputeUnsealedSectorCid {
    reg: RegisteredSealProof,
    pieces: Vec<PieceInfo>,
    cid: Cid,
    exit_code: ExitCode,
}

#[derive(Clone, Debug)]
pub struct ExpectRandomness {
    tag: DomainSeparationTag,
    epoch: ChainEpoch,
    entropy: Vec<u8>,
    out: [u8; RANDOMNESS_LENGTH],
}

#[derive(Debug)]
pub struct ExpectBatchVerifySeals {
    input: Vec<SealVerifyInfo>,
    result: anyhow::Result<Vec<bool>>,
}

#[derive(Debug)]
pub struct ExpectAggregateVerifySeals {
    in_svis: Vec<AggregateSealVerifyInfo>,
    in_proof: Vec<u8>,
    result: anyhow::Result<()>,
}

#[derive(Debug)]
pub struct ExpectReplicaVerify {
    input: ReplicaUpdateInfo,
    result: anyhow::Result<()>,
}

pub fn expect_empty(res: Option<IpldBlock>) {
    assert!(res.is_none());
}

pub fn expect_abort_contains_message<T: fmt::Debug>(
    expect_exit_code: ExitCode,
    expect_msg: &str,
    res: Result<T, ActorError>,
) {
    let err = res.expect_err(&format!(
        "expected abort with exit code {}, but call succeeded",
        expect_exit_code
    ));
    assert_eq!(
        err.exit_code(),
        expect_exit_code,
        "expected failure with exit code {}, but failed with exit code {}; error message: {}",
        expect_exit_code,
        err.exit_code(),
        err.msg(),
    );
    let err_msg = err.msg();
    assert!(
        err.msg().contains(expect_msg),
        "expected err message '{}' to contain '{}'",
        err_msg,
        expect_msg,
    );
}

pub fn expect_abort<T: fmt::Debug>(exit_code: ExitCode, res: Result<T, ActorError>) {
    expect_abort_contains_message(exit_code, "", res);
}

impl<BS: Blockstore> MockRuntime<BS> {
    ///// Runtime access for tests /////

    pub fn get_state<T: DeserializeOwned>(&self) -> T {
        self.store_get(self.state.borrow().as_ref().unwrap())
    }

    pub fn replace_state<T: Serialize>(&self, obj: &T) {
        self.state.replace(Some(self.store_put(obj)));
    }

    pub fn set_balance(&self, amount: TokenAmount) {
        self.balance.replace(amount);
    }

    pub fn get_balance(&self) -> TokenAmount {
        self.balance.borrow().to_owned()
    }

    pub fn add_balance(&self, amount: TokenAmount) {
        self.balance.replace_with(|b| b.clone() + amount);
    }

    pub fn set_caller(&self, code_id: Cid, address: Address) {
        // fail if called with a non-ID address, since the caller() method must always return an ID
        address.id().unwrap();
        self.caller.replace(address);
        self.caller_type.replace(code_id);
        self.actor_code_cids.borrow_mut().insert(address, code_id);
    }

    pub fn set_origin(&self, address: Address) {
        self.origin.replace(address);
    }

    pub fn set_address_actor_type(&self, address: Address, actor_type: Cid) {
        self.actor_code_cids.borrow_mut().insert(address, actor_type);
    }

    pub fn get_id_address(&self, address: &Address) -> Option<Address> {
        if address.protocol() == Protocol::ID {
            return Some(*address);
        }
        self.id_addresses.borrow().get(address).cloned()
    }

    pub fn add_id_address(&self, source: Address, target: Address) {
        assert_eq!(target.protocol(), Protocol::ID, "target must use ID address protocol");
        self.id_addresses.borrow_mut().insert(source, target);
    }

    pub fn set_delegated_address(&self, source: ActorID, target: Address) {
        assert_eq!(
            target.protocol(),
            Protocol::Delegated,
            "target must use Delegated address protocol"
        );
        self.delegated_addresses.borrow_mut().insert(source, target);
        self.id_addresses.borrow_mut().insert(target, Address::new_id(source));
    }

    pub fn call<A: ActorCode>(
        &self,
        method_num: MethodNum,
        params: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        self.in_call.replace(true);
        let prev_state = *self.state.borrow();
        let res = A::invoke_method(self, method_num, params);

        if res.is_err() {
            self.state.replace(prev_state);
        }
        self.in_call.replace(false);
        res
    }

    /// Verifies that all mock expectations have been met (and resets the expectations).
    pub fn verify(&self) {
        self.expectations.borrow_mut().verify()
    }

    /// Clears all mock expectations.
    pub fn reset(&self) {
        self.expectations.borrow_mut().reset();
    }

    ///// Mock expectations /////

    #[allow(dead_code)]
    pub fn expect_validate_caller_addr(&self, addr: Vec<Address>) {
        assert!(!addr.is_empty(), "addrs must be non-empty");
        self.expectations.borrow_mut().expect_validate_caller_addr = Some(addr);
    }

    #[allow(dead_code)]
    pub fn expect_verify_signature(&self, exp: ExpectedVerifySig) {
        self.expectations.borrow_mut().expect_verify_sigs.push_back(exp);
    }

    #[allow(dead_code)]
    pub fn expect_verify_consensus_fault(
        &self,
        h1: Vec<u8>,
        h2: Vec<u8>,
        extra: Vec<u8>,
        fault: Option<ConsensusFault>,
        exit_code: ExitCode,
    ) {
        self.expectations.borrow_mut().expect_verify_consensus_fault =
            Some(ExpectVerifyConsensusFault {
                require_correct_input: true,
                block_header_1: h1,
                block_header_2: h2,
                block_header_extra: extra,
                fault,
                exit_code,
            });
    }

    #[allow(dead_code)]
    pub fn expect_compute_unsealed_sector_cid(
        &self,
        reg: RegisteredSealProof,
        pieces: Vec<PieceInfo>,
        cid: Cid,
        exit_code: ExitCode,
    ) {
        let exp = ExpectComputeUnsealedSectorCid { reg, pieces, cid, exit_code };
        self.expectations.borrow_mut().expect_compute_unsealed_sector_cid.push_back(exp);
    }

    #[allow(dead_code)]
    pub fn expect_validate_caller_type(&self, types: Vec<Type>) {
        assert!(!types.is_empty(), "addrs must be non-empty");
        self.expectations.borrow_mut().expect_validate_caller_type = Some(types);
    }

    #[allow(dead_code)]
    pub fn expect_validate_caller_any(&self) {
        self.expectations.borrow_mut().expect_validate_caller_any = true;
    }

    #[allow(dead_code)]
    pub fn expect_validate_caller_namespace(&self, namespaces: Vec<u64>) {
        assert!(!namespaces.is_empty(), "f4 namespaces must be non-empty");
        self.expectations.borrow_mut().expect_validate_caller_f4_namespace = Some(namespaces);
    }

    #[allow(dead_code)]
    pub fn expect_delete_actor(&self, beneficiary: Address) {
        self.expectations.borrow_mut().expect_delete_actor = Some(beneficiary);
    }

    #[allow(dead_code)]
    pub fn expect_send_simple(
        &self,
        to: Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
        send_return: Option<IpldBlock>,
        exit_code: ExitCode,
    ) {
        self.expect_send(
            to,
            method,
            params,
            value,
            None,
            SendFlags::default(),
            send_return,
            exit_code,
            None,
        )
    }

    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub fn expect_send(
        &self,
        to: Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
        gas_limit: Option<u64>,
        send_flags: SendFlags,
        send_return: Option<IpldBlock>,
        exit_code: ExitCode,
        send_error: Option<ErrorNumber>,
    ) {
        self.expectations.borrow_mut().expect_sends.push_back(ExpectedMessage {
            to,
            method,
            params,
            value,
            gas_limit,
            send_flags,
            send_return,
            exit_code,
            send_error,
        })
    }

    #[allow(dead_code)]
    pub fn expect_create_actor(
        &self,
        code_id: Cid,
        actor_id: ActorID,
        predictable_address: Option<Address>,
    ) {
        let a = ExpectCreateActor { code_id, actor_id, predictable_address };
        self.expectations.borrow_mut().expect_create_actor = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_verify_seal(&self, seal: SealVerifyInfo, exit_code: ExitCode) {
        let a = ExpectVerifySeal { seal, exit_code };
        self.expectations.borrow_mut().expect_verify_seal = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_verify_post(&self, post: WindowPoStVerifyInfo, exit_code: ExitCode) {
        let a = ExpectVerifyPoSt { post, exit_code };
        self.expectations.borrow_mut().expect_verify_post = Some(a);
    }

    #[allow(dead_code)]
    pub fn set_received(&self, amount: TokenAmount) {
        self.value_received.replace(amount);
    }

    #[allow(dead_code)]
    pub fn set_base_fee(&self, base_fee: TokenAmount) {
        self.base_fee.replace(base_fee);
    }

    #[allow(dead_code)]
    pub fn set_circulating_supply(&self, circ_supply: TokenAmount) {
        self.circulating_supply.replace(circ_supply);
    }

    #[allow(dead_code)]
    pub fn set_epoch(&self, epoch: ChainEpoch) -> ChainEpoch {
        self.epoch.replace(epoch);
        epoch
    }

    pub fn expect_get_randomness_from_tickets(
        &self,
        tag: DomainSeparationTag,
        epoch: ChainEpoch,
        entropy: Vec<u8>,
        out: [u8; RANDOMNESS_LENGTH],
    ) {
        let a = ExpectRandomness { tag, epoch, entropy, out };
        self.expectations.borrow_mut().expect_get_randomness_tickets.push_back(a);
    }

    #[allow(dead_code)]
    pub fn expect_get_randomness_from_beacon(
        &self,
        tag: DomainSeparationTag,
        epoch: ChainEpoch,
        entropy: Vec<u8>,
        out: [u8; RANDOMNESS_LENGTH],
    ) {
        let a = ExpectRandomness { tag, epoch, entropy, out };
        self.expectations.borrow_mut().expect_get_randomness_beacon.push_back(a);
    }

    #[allow(dead_code)]
    pub fn expect_batch_verify_seals(
        &self,
        input: Vec<SealVerifyInfo>,
        result: anyhow::Result<Vec<bool>>,
    ) {
        let a = ExpectBatchVerifySeals { input, result };
        self.expectations.borrow_mut().expect_batch_verify_seals = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_aggregate_verify_seals(
        &self,
        in_svis: Vec<AggregateSealVerifyInfo>,
        in_proof: Vec<u8>,
        result: anyhow::Result<()>,
    ) {
        let a = ExpectAggregateVerifySeals { in_svis, in_proof, result };
        self.expectations.borrow_mut().expect_aggregate_verify_seals = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_replica_verify(&self, input: ReplicaUpdateInfo, result: anyhow::Result<()>) {
        let a = ExpectReplicaVerify { input, result };
        self.expectations.borrow_mut().expect_replica_verify = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_gas_charge(&self, value: i64) {
        self.expectations.borrow_mut().expect_gas_charge.push_back(value);
    }

    #[allow(dead_code)]
    pub fn expect_gas_available(&self, value: u64) {
        self.expectations.borrow_mut().expect_gas_available.push_back(value);
    }

    #[allow(dead_code)]
    pub fn expect_emitted_event(&self, event: ActorEvent) {
        self.expectations.borrow_mut().expect_emitted_events.push_back(event)
    }

    ///// Private helpers /////

    fn require_in_call(&self) {
        assert!(*self.in_call.borrow(), "invalid runtime invocation outside of method call")
    }

    fn store_put<T: Serialize>(&self, o: &T) -> Cid {
        self.store.put_cbor(&o, Code::Blake2b256).unwrap()
    }

    fn store_get<T: DeserializeOwned>(&self, cid: &Cid) -> T {
        self.store.get_cbor(cid).unwrap().unwrap()
    }
}

impl<BS> MessageInfo for MockRuntime<BS> {
    fn nonce(&self) -> u64 {
        0
    }

    fn caller(&self) -> Address {
        *self.caller.borrow()
    }
    fn origin(&self) -> Address {
        *self.origin.borrow()
    }
    fn receiver(&self) -> Address {
        self.receiver
    }
    fn value_received(&self) -> TokenAmount {
        self.value_received.borrow().clone()
    }
    fn gas_premium(&self) -> TokenAmount {
        self.gas_premium.clone()
    }
}

impl<BS: Blockstore> Runtime for MockRuntime<BS> {
    type Blockstore = Rc<BS>;

    fn network_version(&self) -> NetworkVersion {
        self.network_version
    }

    fn message(&self) -> &dyn MessageInfo {
        self.require_in_call();
        self
    }

    fn curr_epoch(&self) -> ChainEpoch {
        self.require_in_call();
        *self.epoch.borrow()
    }

    fn validate_immediate_caller_accept_any(&self) -> Result<(), ActorError> {
        self.require_in_call();
        assert!(
            self.expectations.borrow_mut().expect_validate_caller_any,
            "unexpected validate-caller-any"
        );
        self.expectations.borrow_mut().expect_validate_caller_any = false;
        Ok(())
    }

    fn validate_immediate_caller_is<'a, I>(&self, addresses: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Address>,
    {
        self.require_in_call();

        let addrs: Vec<Address> = addresses.into_iter().cloned().collect();

        let mut expectations = self.expectations.borrow_mut();
        assert!(
            expectations.expect_validate_caller_addr.is_some(),
            "unexpected validate caller addrs"
        );

        let expected_addrs = expectations.expect_validate_caller_addr.as_ref().unwrap();
        assert_eq!(
            &addrs, expected_addrs,
            "unexpected validate caller addrs {:?}, expected {:?}",
            addrs, &expectations.expect_validate_caller_addr
        );

        for expected in &addrs {
            if self.message().caller() == *expected {
                expectations.expect_validate_caller_addr = None;
                return Ok(());
            }
        }
        expectations.expect_validate_caller_addr = None;
        Err(actor_error!(forbidden;
                "caller address {:?} forbidden, allowed: {:?}",
                self.message().caller(), &addrs
        ))
    }

    fn validate_immediate_caller_namespace<I>(&self, namespaces: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = u64>,
    {
        self.require_in_call();

        let namespaces: Vec<u64> = namespaces.into_iter().collect();

        let mut expectations = self.expectations.borrow_mut();
        assert!(
            expectations.expect_validate_caller_f4_namespace.is_some(),
            "unexpected validate caller namespace"
        );

        let expected_namespaces =
            expectations.expect_validate_caller_f4_namespace.as_ref().unwrap();

        assert_eq!(
            &namespaces, expected_namespaces,
            "unexpected validate caller namespace {:?}, expected {:?}",
            namespaces, &expectations.expect_validate_caller_f4_namespace
        );

        let caller_f4 = self.lookup_delegated_address(self.caller().id().unwrap());

        assert!(caller_f4.is_some(), "unexpected caller doesn't have a delegated address");

        for id in namespaces.iter() {
            let bound_address = match caller_f4.unwrap().payload() {
                Payload::Delegated(d) => d.namespace(),
                _ => unreachable!(
                    "lookup_delegated_address should always return a delegated address"
                ),
            };
            if bound_address == *id {
                expectations.expect_validate_caller_f4_namespace = None;
                return Ok(());
            }
        }
        expectations.expect_validate_caller_addr = None;
        Err(actor_error!(forbidden;
                "caller address {:?} forbidden, allowed: {:?}",
                self.message().caller(), &namespaces
        ))
    }

    fn validate_immediate_caller_type<'a, I>(&self, types: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Type>,
    {
        self.require_in_call();
        assert!(
            self.expectations.borrow_mut().expect_validate_caller_type.is_some(),
            "unexpected validate caller code"
        );

        let types: Vec<Type> = types.into_iter().copied().collect();
        let expected_caller_type =
            self.expectations.borrow_mut().expect_validate_caller_type.clone().unwrap();
        assert_eq!(
            &types, &expected_caller_type,
            "unexpected validate caller code {:?}, expected {:?}",
            types, expected_caller_type,
        );

        if let Some(call_type) = self.resolve_builtin_actor_type(&*self.caller_type.borrow()) {
            for expected in &types {
                if &call_type == expected {
                    self.expectations.borrow_mut().expect_validate_caller_type = None;
                    return Ok(());
                }
            }
        }

        self.expectations.borrow_mut().expect_validate_caller_type = None;
        Err(actor_error!(forbidden; "caller type {:?} forbidden, allowed: {:?}",
                self.caller_type, types))
    }

    fn current_balance(&self) -> TokenAmount {
        self.require_in_call();
        self.balance.borrow().clone()
    }

    fn actor_balance(&self, id: ActorID) -> Option<TokenAmount> {
        self.require_in_call();
        self.actor_balances.get(&id).cloned()
    }

    fn resolve_address(&self, address: &Address) -> Option<ActorID> {
        self.require_in_call();
        if let &Payload::ID(id) = address.payload() {
            return Some(id);
        }

        match self.get_id_address(address) {
            None => None,
            Some(addr) => {
                if let &Payload::ID(id) = addr.payload() {
                    return Some(id);
                }
                None
            }
        }
    }

    fn lookup_delegated_address(&self, id: ActorID) -> Option<Address> {
        self.require_in_call();
        self.delegated_addresses.borrow().get(&id).copied()
    }

    fn get_actor_code_cid(&self, id: &ActorID) -> Option<Cid> {
        self.require_in_call();
        self.actor_code_cids.borrow().get(&Address::new_id(*id)).cloned()
    }

    fn get_randomness_from_tickets(
        &self,
        tag: DomainSeparationTag,
        epoch: ChainEpoch,
        entropy: &[u8],
    ) -> Result<[u8; RANDOMNESS_LENGTH], ActorError> {
        let expected = self
            .expectations
            .borrow_mut()
            .expect_get_randomness_tickets
            .pop_front()
            .expect("unexpected call to get_randomness_from_tickets");

        assert!(epoch <= *self.epoch.borrow(), "attempt to get randomness from future");
        assert_eq!(
            expected.tag, tag,
            "unexpected domain separation tag, expected: {:?}, actual: {:?}",
            expected.tag, tag
        );
        assert_eq!(
            expected.epoch, epoch,
            "unexpected epoch, expected: {:?}, actual: {:?}",
            expected.epoch, epoch
        );
        assert_eq!(
            expected.entropy, *entropy,
            "unexpected entroy, expected {:?}, actual: {:?}",
            expected.entropy, entropy
        );

        Ok(expected.out)
    }

    fn get_randomness_from_beacon(
        &self,
        tag: DomainSeparationTag,
        epoch: ChainEpoch,
        entropy: &[u8],
    ) -> Result<[u8; RANDOMNESS_LENGTH], ActorError> {
        let expected = self
            .expectations
            .borrow_mut()
            .expect_get_randomness_beacon
            .pop_front()
            .expect("unexpected call to get_randomness_from_beacon");

        assert!(epoch <= *self.epoch.borrow(), "attempt to get randomness from future");
        assert_eq!(
            expected.tag, tag,
            "unexpected domain separation tag, expected: {:?}, actual: {:?}",
            expected.tag, tag
        );
        assert_eq!(
            expected.epoch, epoch,
            "unexpected epoch, expected: {:?}, actual: {:?}",
            expected.epoch, epoch
        );
        assert_eq!(
            expected.entropy, *entropy,
            "unexpected entroy, expected {:?}, actual: {:?}",
            expected.entropy, entropy
        );

        Ok(expected.out)
    }

    fn create<T: Serialize>(&self, obj: &T) -> Result<(), ActorError> {
        if self.state.borrow().is_some() {
            return Err(actor_error!(illegal_state; "state already constructed"));
        }
        self.state.replace(Some(self.store_put(obj)));
        Ok(())
    }

    fn state<T: DeserializeOwned>(&self) -> Result<T, ActorError> {
        Ok(self.store_get(self.state.borrow().as_ref().unwrap()))
    }

    fn get_state_root(&self) -> Result<Cid, ActorError> {
        Ok(self.state.borrow().unwrap_or(EMPTY_ARR_CID))
    }

    fn set_state_root(&self, root: &Cid) -> Result<(), ActorError> {
        self.state.replace(Some(*root));
        Ok(())
    }

    fn transaction<S, RT, F>(&self, f: F) -> Result<RT, ActorError>
    where
        S: Serialize + DeserializeOwned,
        F: FnOnce(&mut S, &Self) -> Result<RT, ActorError>,
    {
        if *self.in_transaction.borrow() {
            return Err(actor_error!(assertion_failed; "nested transaction"));
        }
        let mut read_only = self.state()?;
        self.in_transaction.replace(true);
        let ret = f(&mut read_only, self);
        if ret.is_ok() {
            self.state.replace(Some(self.store_put(&read_only)));
        }
        self.in_transaction.replace(false);
        ret
    }

    fn store(&self) -> &Rc<BS> {
        &self.store
    }

    fn send(
        &self,
        to: &Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
        gas_limit: Option<u64>,
        send_flags: SendFlags,
    ) -> Result<Response, SendError> {
        self.require_in_call();
        if *self.in_transaction.borrow() {
            return Ok(Response { exit_code: ExitCode::USR_ASSERTION_FAILED, return_data: None });
        }

        assert!(
            !self.expectations.borrow_mut().expect_sends.is_empty(),
            "unexpected message to: {:?} method: {:?}, value: {:?}, params: {:?}",
            to,
            method,
            value,
            params
        );

        let expected_msg = self.expectations.borrow_mut().expect_sends.pop_front().unwrap();

        assert_eq!(expected_msg.to, *to);
        assert_eq!(expected_msg.method, method);
        assert_eq!(expected_msg.params, params);
        assert_eq!(expected_msg.value, value);
        assert_eq!(expected_msg.gas_limit, gas_limit, "gas limit did not match expectation");
        assert_eq!(expected_msg.send_flags, send_flags, "send flags did not match expectation");

        if let Some(e) = expected_msg.send_error {
            return Err(SendError(e));
        }

        {
            let mut balance = self.balance.borrow_mut();
            if value > *balance {
                return Err(SendError(ErrorNumber::InsufficientFunds));
            }
            *balance -= value;
        }

        Ok(Response { exit_code: expected_msg.exit_code, return_data: expected_msg.send_return })
    }

    fn new_actor_address(&self) -> Result<Address, ActorError> {
        self.require_in_call();
        let ret =
            *self.new_actor_addr.borrow().as_ref().expect("unexpected call to new actor address");
        self.new_actor_addr.replace(None);
        Ok(ret)
    }

    fn create_actor(
        &self,
        code_id: Cid,
        actor_id: ActorID,
        predictable_address: Option<Address>,
    ) -> Result<(), ActorError> {
        self.require_in_call();
        if *self.in_transaction.borrow() {
            return Err(actor_error!(assertion_failed; "side-effect within transaction"));
        }
        let expect_create_actor = self
            .expectations
            .borrow_mut()
            .expect_create_actor
            .take()
            .expect("unexpected call to create actor");

        assert_eq!(
            expect_create_actor,
            ExpectCreateActor { code_id, actor_id, predictable_address },
            "unexpected actor being created"
        );
        self.set_address_actor_type(Address::new_id(actor_id), code_id);
        Ok(())
    }

    fn delete_actor(&self, addr: &Address) -> Result<(), ActorError> {
        self.require_in_call();
        if *self.in_transaction.borrow() {
            return Err(actor_error!(assertion_failed; "side-effect within transaction"));
        }
        let exp_act = self.expectations.borrow_mut().expect_delete_actor.take();
        if exp_act.is_none() {
            panic!("unexpected call to delete actor: {}", addr);
        }
        if exp_act.as_ref().unwrap() != addr {
            panic!("attempt to delete wrong actor. Expected: {}, got: {}", exp_act.unwrap(), addr);
        }
        Ok(())
    }

    fn resolve_builtin_actor_type(&self, code_id: &Cid) -> Option<Type> {
        self.require_in_call();
        (*ACTOR_TYPES).get(code_id).cloned()
    }

    fn get_code_cid_for_type(&self, typ: Type) -> Cid {
        self.require_in_call();
        (*ACTOR_TYPES)
            .iter()
            .find_map(|(cid, t)| if *t == typ { Some(cid) } else { None })
            .cloned()
            .unwrap()
    }

    fn total_fil_circ_supply(&self) -> TokenAmount {
        self.circulating_supply.borrow().clone()
    }

    fn charge_gas(&self, _: &'static str, value: i64) {
        let mut exs = self.expectations.borrow_mut();
        assert!(!exs.expect_gas_charge.is_empty(), "unexpected gas charge {:?}", value);
        let expected = exs.expect_gas_charge.pop_front().unwrap();
        assert_eq!(expected, value, "expected gas charge {:?}, actual {:?}", expected, value);
    }

    fn base_fee(&self) -> TokenAmount {
        self.base_fee.borrow().clone()
    }

    fn gas_available(&self) -> u64 {
        let mut exs = self.expectations.borrow_mut();
        assert!(!exs.expect_gas_available.is_empty(), "unexpected gas available call");
        exs.expect_gas_available.pop_front().unwrap()
    }

    fn tipset_timestamp(&self) -> u64 {
        self.tipset_timestamp
    }

    fn tipset_cid(&self, epoch: i64) -> Result<Cid, ActorError> {
        let offset = *self.epoch.borrow() - epoch;
        // Can't get tipset for:
        // - current or future epochs
        // - negative epochs
        // - epochs beyond FINALITY of current epoch
        if offset <= 0 || epoch < 0 || offset > self.policy.chain_finality {
            return Err(
                actor_error!(illegal_argument; "invalid epoch to fetch tipset_cid {}", epoch),
            );
        }
        Ok(*self.tipset_cids.get(epoch as usize).unwrap())
    }

    fn emit_event(&self, event: &ActorEvent) -> Result<(), ActorError> {
        let expected = self
            .expectations
            .borrow_mut()
            .expect_emitted_events
            .pop_front()
            .expect("unexpected call to emit_evit");

        assert_eq!(*event, expected);

        Ok(())
    }

    fn chain_id(&self) -> ChainID {
        self.chain_id
    }

    fn read_only(&self) -> bool {
        false
    }
}

impl<BS> Primitives for MockRuntime<BS> {
    fn verify_signature(
        &self,
        signature: &Signature,
        signer: &Address,
        plaintext: &[u8],
    ) -> anyhow::Result<()> {
        if self.expectations.borrow_mut().expect_verify_sigs.is_empty() {
            panic!(
                "Unexpected signature verification sig: {:?}, signer: {}, plaintext: {}",
                signature,
                signer,
                hex::encode(plaintext)
            );
        }
        let exp = self.expectations.borrow_mut().expect_verify_sigs.pop_front();
        if let Some(exp) = exp {
            if exp.sig != *signature || exp.signer != *signer || &exp.plaintext[..] != plaintext {
                panic!(
                    "unexpected signature verification\n\
                    sig: {:?}, signer: {}, plaintext: {}\n\
                    expected sig: {:?}, signer: {}, plaintext: {}",
                    signature,
                    signer,
                    hex::encode(plaintext),
                    exp.sig,
                    exp.signer,
                    hex::encode(exp.plaintext)
                )
            }
            exp.result?
        } else {
            panic!(
                "unexpected syscall to verify signature: {:?}, signer: {}, plaintext: {}",
                signature,
                signer,
                hex::encode(plaintext)
            )
        }
        Ok(())
    }

    fn hash_blake2b(&self, data: &[u8]) -> [u8; 32] {
        let (digest, _) = (*self.hash_func)(SupportedHashes::Blake2b256, data);
        let mut ret = [0u8; 32];
        ret.copy_from_slice(&digest[..32]);
        ret
    }

    fn hash(&self, hasher: SupportedHashes, data: &[u8]) -> Vec<u8> {
        let (digest, len) = (*self.hash_func)(hasher, data);
        Vec::from(&digest[..len])
    }

    fn compute_unsealed_sector_cid(
        &self,
        reg: RegisteredSealProof,
        pieces: &[PieceInfo],
    ) -> anyhow::Result<Cid> {
        let exp = self
            .expectations
            .borrow_mut()
            .expect_compute_unsealed_sector_cid
            .pop_front()
            .expect("Unexpected syscall to ComputeUnsealedSectorCID");

        assert_eq!(exp.reg, reg, "Unexpected compute_unsealed_sector_cid : reg mismatch");
        assert!(
            exp.pieces[..].eq(pieces),
            "Unexpected compute_unsealed_sector_cid : pieces mismatch, exp: {:?}, got: {:?}",
            exp.pieces,
            pieces,
        );

        if exp.exit_code != ExitCode::OK {
            return Err(anyhow!(ActorError::unchecked(
                exp.exit_code,
                "Expected Failure".to_string(),
            )));
        }
        Ok(exp.cid)
    }

    fn recover_secp_public_key(
        &self,
        hash: &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
        signature: &[u8; SECP_SIG_LEN],
    ) -> Result<[u8; SECP_PUB_LEN], anyhow::Error> {
        (*self.recover_secp_pubkey_fn)(hash, signature)
            .map_err(|_| anyhow!("failed to recover pubkey."))
    }

    fn hash_64(&self, hasher: SupportedHashes, data: &[u8]) -> ([u8; 64], usize) {
        (*self.hash_func)(hasher, data)
    }
}

impl<BS> Verifier for MockRuntime<BS> {
    fn verify_seal(&self, seal: &SealVerifyInfo) -> anyhow::Result<()> {
        let exp = self
            .expectations
            .borrow_mut()
            .expect_verify_seal
            .take()
            .expect("Unexpected syscall to verify seal");

        assert_eq!(exp.seal, *seal, "Unexpected seal verification");
        if exp.exit_code != ExitCode::OK {
            return Err(anyhow!(ActorError::unchecked(
                exp.exit_code,
                "Expected Failure".to_string(),
            )));
        }
        Ok(())
    }

    fn verify_post(&self, post: &WindowPoStVerifyInfo) -> anyhow::Result<()> {
        let exp = self
            .expectations
            .borrow_mut()
            .expect_verify_post
            .take()
            .expect("Unexpected syscall to verify PoSt");

        assert_eq!(exp.post, *post, "Unexpected PoSt verification");
        if exp.exit_code != ExitCode::OK {
            return Err(anyhow!(ActorError::unchecked(
                exp.exit_code,
                "Expected Failure".to_string(),
            )));
        }
        Ok(())
    }

    fn verify_consensus_fault(
        &self,
        h1: &[u8],
        h2: &[u8],
        extra: &[u8],
    ) -> anyhow::Result<Option<ConsensusFault>> {
        let exp = self
            .expectations
            .borrow_mut()
            .expect_verify_consensus_fault
            .take()
            .expect("Unexpected syscall to verify_consensus_fault");

        if exp.require_correct_input {
            assert_eq!(exp.block_header_1, h1, "Header 1 mismatch");
            assert_eq!(exp.block_header_2, h2, "Header 2 mismatch");
            assert_eq!(exp.block_header_extra, extra, "Header extra mismatch");
        }
        if exp.exit_code != ExitCode::OK {
            return Err(anyhow!(ActorError::unchecked(
                exp.exit_code,
                "Expected Failure".to_string(),
            )));
        }
        Ok(exp.fault)
    }

    fn batch_verify_seals(&self, batch: &[SealVerifyInfo]) -> anyhow::Result<Vec<bool>> {
        let exp = self
            .expectations
            .borrow_mut()
            .expect_batch_verify_seals
            .take()
            .expect("unexpected call to batch verify seals");
        assert_eq!(exp.input.len(), batch.len(), "length mismatch");

        for (i, exp_svi) in exp.input.iter().enumerate() {
            assert_eq!(
                exp_svi.sealed_cid, batch[i].sealed_cid,
                "sealed CID mismatch at index {}",
                i
            );
            assert_eq!(
                exp_svi.unsealed_cid, batch[i].unsealed_cid,
                "unsealed CID mismatch at index {}",
                i
            );
        }
        exp.result
    }

    fn verify_aggregate_seals(
        &self,
        aggregate: &AggregateSealVerifyProofAndInfos,
    ) -> anyhow::Result<()> {
        let exp = self
            .expectations
            .borrow_mut()
            .expect_aggregate_verify_seals
            .take()
            .expect("unexpected call to verify aggregate seals");
        assert_eq!(exp.in_svis.len(), aggregate.infos.len(), "length mismatch");
        for (i, exp_svi) in exp.in_svis.iter().enumerate() {
            assert_eq!(exp_svi.sealed_cid, aggregate.infos[i].sealed_cid, "mismatched sealed CID");
            assert_eq!(
                exp_svi.unsealed_cid, aggregate.infos[i].unsealed_cid,
                "mismatched unsealed CID"
            );
        }
        assert_eq!(exp.in_proof, aggregate.proof, "proof mismatch");
        exp.result
    }

    fn verify_replica_update(&self, replica: &ReplicaUpdateInfo) -> anyhow::Result<()> {
        let exp = self
            .expectations
            .borrow_mut()
            .expect_replica_verify
            .take()
            .expect("unexpected call to verify replica update");
        assert_eq!(exp.input.update_proof_type, replica.update_proof_type, "mismatched proof type");
        assert_eq!(exp.input.new_sealed_cid, replica.new_sealed_cid, "mismatched new sealed CID");
        assert_eq!(exp.input.old_sealed_cid, replica.old_sealed_cid, "mismatched old sealed CID");
        assert_eq!(
            exp.input.new_unsealed_cid, replica.new_unsealed_cid,
            "mismatched new unsealed CID"
        );
        exp.result
    }
}

impl<BS> RuntimePolicy for MockRuntime<BS> {
    fn policy(&self) -> &Policy {
        &self.policy
    }
}

// The Expectations are by default verified on drop().
// In order to clear the unsatisfied expectations in tests, use MockRuntime#reset().
impl Drop for Expectations {
    fn drop(&mut self) {
        if !self.skip_verification_on_drop && !std::thread::panicking() {
            self.verify();
        }
    }
}

pub fn blake2b_256(data: &[u8]) -> [u8; 32] {
    blake2b_simd::Params::new()
        .hash_length(32)
        .to_state()
        .update(data)
        .finalize()
        .as_bytes()
        .try_into()
        .unwrap()
}

pub fn hash(hasher: SupportedHashes, data: &[u8]) -> ([u8; 64], usize) {
    let hasher = Code::try_from(hasher as u64).unwrap();
    let (_, digest, written) = hasher.digest(data).into_inner();
    (digest, written as usize)
}

#[allow(clippy::result_unit_err)]
pub fn recover_secp_public_key(
    hash: &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
    signature: &[u8; SECP_SIG_LEN],
) -> Result<[u8; SECP_PUB_LEN], ()> {
    // generate types to recover key from
    let rec_id = RecoveryId::parse(signature[64]).map_err(|_| ())?;
    let message = Message::parse(hash);

    // Signature value without recovery byte
    let mut s = [0u8; 64];
    s.copy_from_slice(signature[..64].as_ref());

    // generate Signature
    let sig = EcsdaSignature::parse_standard(&s).map_err(|_| ())?;
    Ok(recover(&message, &sig, &rec_id).map_err(|_| ())?.serialize())
}

// multihash library doesn't support poseidon hashing, so we fake it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Multihash)]
#[mh(alloc_size = 64)]
enum MhCode {
    #[mh(code = 0xb401, hasher = multihash::Sha2_256)]
    PoseidonFake,
    #[mh(code = 0x1012, hasher = multihash::Sha2_256)]
    Sha256TruncPaddedFake,
}

fn make_cid(input: &[u8], prefix: u64, hash: MhCode) -> Cid {
    let hash = hash.digest(input);
    Cid::new_v1(prefix, hash)
}

pub fn make_cid_sha(input: &[u8], prefix: u64) -> Cid {
    make_cid(input, prefix, MhCode::Sha256TruncPaddedFake)
}

pub fn make_cid_poseidon(input: &[u8], prefix: u64) -> Cid {
    make_cid(input, prefix, MhCode::PoseidonFake)
}

pub fn make_piece_cid(input: &[u8]) -> Cid {
    make_cid_sha(input, FIL_COMMITMENT_UNSEALED)
}

pub fn make_sealed_cid(input: &[u8]) -> Cid {
    make_cid_poseidon(input, FIL_COMMITMENT_SEALED)
}

pub fn new_bls_addr(s: u8) -> Address {
    let seed = [s; 32];
    let mut rng: StdRng = SeedableRng::from_seed(seed);
    let mut key = [0u8; 48];
    rng.fill_bytes(&mut key);
    Address::new_bls(&key).unwrap()
}
