// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use core::fmt;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Display;
use std::rc::Rc;

use anyhow::anyhow;
use cid::multihash::{Code, Multihash as OtherMultihash};
use cid::Cid;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::de::DeserializeOwned;
use fvm_ipld_encoding::{Cbor, CborStore, RawBytes};
use fvm_shared::actor::builtin::Type;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::clock::ChainEpoch;

use fvm_shared::commcid::{FIL_COMMITMENT_SEALED, FIL_COMMITMENT_UNSEALED};
use fvm_shared::consensus::ConsensusFault;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PieceInfo;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::{
    AggregateSealVerifyInfo, AggregateSealVerifyProofAndInfos, RegisteredSealProof,
    ReplicaUpdateInfo, SealVerifyInfo, WindowPoStVerifyInfo,
};
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum};

use multihash::derive::Multihash;
use multihash::MultihashDigest;

use rand::prelude::*;

use crate::runtime::{
    ActorCode, DomainSeparationTag, MessageInfo, Policy, Primitives, Runtime, RuntimePolicy,
    Verifier,
};
use crate::{actor_error, ActorError};

lazy_static! {
    pub static ref SYSTEM_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/system");
    pub static ref INIT_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/init");
    pub static ref CRON_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/cron");
    pub static ref ACCOUNT_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/account");
    pub static ref POWER_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/storagepower");
    pub static ref MINER_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/storageminer");
    pub static ref MARKET_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/storagemarket");
    pub static ref PAYCH_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/paymentchannel");
    pub static ref MULTISIG_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/multisig");
    pub static ref REWARD_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/reward");
    pub static ref VERIFREG_ACTOR_CODE_ID: Cid = make_builtin(b"fil/test/verifiedregistry");
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
    ]
    .into_iter()
    .collect();
    pub static ref CALLER_TYPES_SIGNABLE: Vec<Cid> =
        vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID];
    pub static ref NON_SINGLETON_CODES: BTreeMap<Cid, ()> = {
        let mut map = BTreeMap::new();
        map.insert(*ACCOUNT_ACTOR_CODE_ID, ());
        map.insert(*PAYCH_ACTOR_CODE_ID, ());
        map.insert(*MULTISIG_ACTOR_CODE_ID, ());
        map.insert(*MINER_ACTOR_CODE_ID, ());
        map
    };
}

const IPLD_RAW: u64 = 0x55;

/// Returns an identity CID for bz.
pub fn make_builtin(bz: &[u8]) -> Cid {
    Cid::new_v1(IPLD_RAW, OtherMultihash::wrap(0, bz).expect("name too long"))
}

pub struct MockRuntime {
    pub epoch: ChainEpoch,
    pub miner: Address,
    pub base_fee: TokenAmount,
    pub id_addresses: HashMap<Address, Address>,
    pub actor_code_cids: HashMap<Address, Cid>,
    pub new_actor_addr: Option<Address>,
    pub receiver: Address,
    pub caller: Address,
    pub caller_type: Cid,
    pub value_received: TokenAmount,
    pub hash_func: Box<dyn Fn(&[u8]) -> [u8; 32]>,
    pub network_version: NetworkVersion,

    // Actor State
    pub state: Option<Cid>,
    pub balance: RefCell<TokenAmount>,

    // VM Impl
    pub in_call: bool,
    pub store: MemoryBlockstore,
    pub in_transaction: bool,

    // Expectations
    pub expectations: RefCell<Expectations>,

    // policy
    pub policy: Policy,

    pub circulating_supply: TokenAmount,
}

#[derive(Default)]
pub struct Expectations {
    pub expect_validate_caller_any: bool,
    pub expect_validate_caller_addr: Option<Vec<Address>>,
    pub expect_validate_caller_type: Option<Vec<Cid>>,
    pub expect_sends: VecDeque<ExpectedMessage>,
    pub expect_create_actor: Option<ExpectCreateActor>,
    pub expect_delete_actor: Option<Address>,
    pub expect_verify_sigs: VecDeque<ExpectedVerifySig>,
    pub expect_verify_seal: Option<ExpectVerifySeal>,
    pub expect_verify_post: Option<ExpectVerifyPoSt>,
    pub expect_compute_unsealed_sector_cid: VecDeque<ExpectComputeUnsealedSectorCid>,
    pub expect_verify_consensus_fault: Option<ExpectVerifyConsensusFault>,
    pub expect_get_randomness_tickets: VecDeque<ExpectRandomness>,
    pub expect_get_randomness_beacon: Option<ExpectRandomness>,
    pub expect_batch_verify_seals: Option<ExpectBatchVerifySeals>,
    pub expect_aggregate_verify_seals: Option<ExpectAggregateVerifySeals>,
    pub expect_replica_verify: Option<ExpectReplicaVerify>,
    pub expect_gas_charge: VecDeque<i64>,
}

impl Expectations {
    fn reset(&mut self) {
        *self = Default::default();
    }

    fn verify(&mut self) {
        assert!(!self.expect_validate_caller_any, "expected ValidateCallerAny, not received");
        assert!(
            self.expect_validate_caller_addr.is_none(),
            "expected ValidateCallerAddr {:?}, not received",
            self.expect_validate_caller_addr
        );
        assert!(
            self.expect_validate_caller_type.is_none(),
            "expected ValidateCallerType {:?}, not received",
            self.expect_validate_caller_type
        );
        assert!(
            self.expect_sends.is_empty(),
            "expected all message to be send, unsent messages {:?}",
            self.expect_sends
        );
        assert!(
            self.expect_create_actor.is_none(),
            "expected actor to be created, uncreated actor: {:?}",
            self.expect_create_actor
        );
        assert!(
            self.expect_delete_actor.is_none(),
            "expected actor to be deleted: {:?}",
            self.expect_delete_actor
        );
        assert!(
            self.expect_verify_sigs.is_empty(),
            "expect_verify_sigs: {:?}, not received",
            self.expect_verify_sigs
        );
        assert!(
            self.expect_verify_seal.is_none(),
            "expect_verify_seal {:?}, not received",
            self.expect_verify_seal
        );
        assert!(
            self.expect_verify_post.is_none(),
            "expect_verify_post {:?}, not received",
            self.expect_verify_post
        );
        assert!(
            self.expect_compute_unsealed_sector_cid.is_empty(),
            "expect_compute_unsealed_sector_cid: {:?}, not received",
            self.expect_compute_unsealed_sector_cid
        );
        assert!(
            self.expect_verify_consensus_fault.is_none(),
            "expect_verify_consensus_fault {:?}, not received",
            self.expect_verify_consensus_fault
        );
        assert!(
            self.expect_get_randomness_tickets.is_empty(),
            "expect_get_randomness_tickets {:?}, not received",
            self.expect_get_randomness_tickets
        );
        assert!(
            self.expect_get_randomness_beacon.is_none(),
            "expect_get_randomness_beacon {:?}, not received",
            self.expect_get_randomness_beacon
        );
        assert!(
            self.expect_batch_verify_seals.is_none(),
            "expect_batch_verify_seals {:?}, not received",
            self.expect_batch_verify_seals
        );
        assert!(
            self.expect_aggregate_verify_seals.is_none(),
            "expect_aggregate_verify_seals {:?}, not received",
            self.expect_aggregate_verify_seals
        );
        assert!(
            self.expect_replica_verify.is_none(),
            "expect_replica_verify {:?}, not received",
            self.expect_replica_verify
        );
        assert!(
            self.expect_gas_charge.is_empty(),
            "expect_gas_charge {:?}, not received",
            self.expect_gas_charge
        );
    }
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self {
            epoch: Default::default(),
            miner: Address::new_id(0),
            base_fee: Default::default(),
            id_addresses: Default::default(),
            actor_code_cids: Default::default(),
            new_actor_addr: Default::default(),
            receiver: Address::new_id(0),
            caller: Address::new_id(0),
            caller_type: Default::default(),
            value_received: Default::default(),
            hash_func: Box::new(blake2b_256),
            network_version: NetworkVersion::V0,
            state: Default::default(),
            balance: Default::default(),
            in_call: Default::default(),
            store: Default::default(),
            in_transaction: Default::default(),
            expectations: Default::default(),
            policy: Default::default(),
            circulating_supply: Default::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExpectCreateActor {
    pub code_id: Cid,
    pub actor_id: ActorID,
}

#[derive(Clone, Debug)]
pub struct ExpectedMessage {
    pub to: Address,
    pub method: MethodNum,
    pub params: RawBytes,
    pub value: TokenAmount,

    // returns from applying expectedMessage
    pub send_return: RawBytes,
    pub exit_code: ExitCode,
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
    out: Randomness,
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

pub fn expect_empty(res: RawBytes) {
    assert_eq!(res, RawBytes::default());
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

impl MockRuntime {
    ///// Runtime access for tests /////

    pub fn get_state<T: Cbor>(&self) -> T {
        self.store_get(self.state.as_ref().unwrap())
    }

    pub fn replace_state<C: Cbor>(&mut self, obj: &C) {
        self.state = Some(self.store_put(obj));
    }

    pub fn set_balance(&mut self, amount: TokenAmount) {
        *self.balance.get_mut() = amount;
    }

    pub fn add_balance(&mut self, amount: TokenAmount) {
        *self.balance.get_mut() += amount;
    }

    pub fn set_value(&mut self, value: TokenAmount) {
        self.value_received = value;
    }

    pub fn set_caller(&mut self, code_id: Cid, address: Address) {
        self.caller = address;
        self.caller_type = code_id;
        self.actor_code_cids.insert(address, code_id);
    }

    pub fn set_address_actor_type(&mut self, address: Address, actor_type: Cid) {
        self.actor_code_cids.insert(address, actor_type);
    }

    pub fn get_id_address(&self, address: &Address) -> Option<Address> {
        if address.protocol() == Protocol::ID {
            return Some(*address);
        }
        self.id_addresses.get(address).cloned()
    }

    pub fn call<A: ActorCode>(
        &mut self,
        method_num: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError> {
        self.in_call = true;
        let prev_state = self.state;
        let res = A::invoke_method(self, method_num, params);

        if res.is_err() {
            self.state = prev_state;
        }
        self.in_call = false;
        res
    }

    /// Verifies that all mock expectations have been met.
    pub fn verify(&mut self) {
        self.expectations.borrow_mut().verify()
    }

    /// Clears all mock expectations.
    pub fn reset(&mut self) {
        self.expectations.borrow_mut().reset();
    }

    ///// Mock expectations /////

    #[allow(dead_code)]
    pub fn expect_validate_caller_addr(&mut self, addr: Vec<Address>) {
        assert!(!addr.is_empty(), "addrs must be non-empty");
        self.expectations.get_mut().expect_validate_caller_addr = Some(addr);
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
    pub fn expect_validate_caller_type(&mut self, types: Vec<Cid>) {
        assert!(!types.is_empty(), "addrs must be non-empty");
        self.expectations.borrow_mut().expect_validate_caller_type = Some(types);
    }

    #[allow(dead_code)]
    pub fn expect_validate_caller_any(&self) {
        self.expectations.borrow_mut().expect_validate_caller_any = true;
    }

    #[allow(dead_code)]
    pub fn expect_delete_actor(&mut self, beneficiary: Address) {
        self.expectations.borrow_mut().expect_delete_actor = Some(beneficiary);
    }

    #[allow(dead_code)]
    pub fn expect_send(
        &mut self,
        to: Address,
        method: MethodNum,
        params: RawBytes,
        value: TokenAmount,
        send_return: RawBytes,
        exit_code: ExitCode,
    ) {
        self.expectations.borrow_mut().expect_sends.push_back(ExpectedMessage {
            to,
            method,
            params,
            value,
            send_return,
            exit_code,
        })
    }

    #[allow(dead_code)]
    pub fn expect_create_actor(&mut self, code_id: Cid, actor_id: ActorID) {
        let a = ExpectCreateActor { code_id, actor_id };
        self.expectations.borrow_mut().expect_create_actor = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_verify_seal(&mut self, seal: SealVerifyInfo, exit_code: ExitCode) {
        let a = ExpectVerifySeal { seal, exit_code };
        self.expectations.borrow_mut().expect_verify_seal = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_verify_post(&mut self, post: WindowPoStVerifyInfo, exit_code: ExitCode) {
        let a = ExpectVerifyPoSt { post, exit_code };
        self.expectations.borrow_mut().expect_verify_post = Some(a);
    }

    #[allow(dead_code)]
    pub fn set_received(&mut self, amount: TokenAmount) {
        self.value_received = amount;
    }

    #[allow(dead_code)]
    pub fn set_circulating_supply(&mut self, circ_supply: TokenAmount) {
        self.circulating_supply = circ_supply;
    }

    #[allow(dead_code)]
    pub fn set_epoch(&mut self, epoch: ChainEpoch) {
        self.epoch = epoch;
    }

    pub fn expect_get_randomness_from_tickets(
        &mut self,
        tag: DomainSeparationTag,
        epoch: ChainEpoch,
        entropy: Vec<u8>,
        out: Randomness,
    ) {
        let a = ExpectRandomness { tag, epoch, entropy, out };
        self.expectations.borrow_mut().expect_get_randomness_tickets.push_back(a);
    }

    #[allow(dead_code)]
    pub fn expect_get_randomness_from_beacon(
        &mut self,
        tag: DomainSeparationTag,
        epoch: ChainEpoch,
        entropy: Vec<u8>,
        out: Randomness,
    ) {
        let a = ExpectRandomness { tag, epoch, entropy, out };
        self.expectations.borrow_mut().expect_get_randomness_beacon = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_batch_verify_seals(
        &mut self,
        input: Vec<SealVerifyInfo>,
        result: anyhow::Result<Vec<bool>>,
    ) {
        let a = ExpectBatchVerifySeals { input, result };
        self.expectations.borrow_mut().expect_batch_verify_seals = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_aggregate_verify_seals(
        &mut self,
        in_svis: Vec<AggregateSealVerifyInfo>,
        in_proof: Vec<u8>,
        result: anyhow::Result<()>,
    ) {
        let a = ExpectAggregateVerifySeals { in_svis, in_proof, result };
        self.expectations.borrow_mut().expect_aggregate_verify_seals = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_replica_verify(&mut self, input: ReplicaUpdateInfo, result: anyhow::Result<()>) {
        let a = ExpectReplicaVerify { input, result };
        self.expectations.borrow_mut().expect_replica_verify = Some(a);
    }

    #[allow(dead_code)]
    pub fn expect_gas_charge(&mut self, value: i64) {
        self.expectations.borrow_mut().expect_gas_charge.push_back(value);
    }

    ///// Private helpers /////

    fn require_in_call(&self) {
        assert!(self.in_call, "invalid runtime invocation outside of method call")
    }

    fn store_put<C: Cbor>(&self, o: &C) -> Cid {
        self.store.put_cbor(&o, Code::Blake2b256).unwrap()
    }

    fn store_get<T: DeserializeOwned>(&self, cid: &Cid) -> T {
        self.store.get_cbor(cid).unwrap().unwrap()
    }
}

impl MessageInfo for MockRuntime {
    fn caller(&self) -> Address {
        self.caller
    }
    fn receiver(&self) -> Address {
        self.receiver
    }
    fn value_received(&self) -> TokenAmount {
        self.value_received.clone()
    }
}

impl Runtime<MemoryBlockstore> for MockRuntime {
    fn network_version(&self) -> NetworkVersion {
        self.network_version
    }

    fn message(&self) -> &dyn MessageInfo {
        self.require_in_call();
        self
    }

    fn curr_epoch(&self) -> ChainEpoch {
        self.require_in_call();
        self.epoch
    }

    fn validate_immediate_caller_accept_any(&mut self) -> Result<(), ActorError> {
        self.require_in_call();
        assert!(
            self.expectations.borrow_mut().expect_validate_caller_any,
            "unexpected validate-caller-any"
        );
        self.expectations.borrow_mut().expect_validate_caller_any = false;
        Ok(())
    }

    fn validate_immediate_caller_is<'a, I>(&mut self, addresses: I) -> Result<(), ActorError>
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
    fn validate_immediate_caller_type<'a, I>(&mut self, types: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Type>,
    {
        self.require_in_call();
        assert!(
            self.expectations.borrow_mut().expect_validate_caller_type.is_some(),
            "unexpected validate caller code"
        );

        let find_by_type = |typ| {
            (*ACTOR_TYPES)
                .iter()
                .find_map(|(cid, t)| if t == typ { Some(cid) } else { None })
                .cloned()
                .unwrap()
        };
        let types: Vec<Cid> = types.into_iter().map(find_by_type).collect();
        let expected_caller_type =
            self.expectations.borrow_mut().expect_validate_caller_type.clone().unwrap();
        assert_eq!(
            &types, &expected_caller_type,
            "unexpected validate caller code {:?}, expected {:?}",
            types, expected_caller_type,
        );

        for expected in &types {
            if &self.caller_type == expected {
                self.expectations.borrow_mut().expect_validate_caller_type = None;
                return Ok(());
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

    fn resolve_address(&self, address: &Address) -> Option<Address> {
        self.require_in_call();
        if address.protocol() == Protocol::ID {
            return Some(*address);
        }
        self.id_addresses.get(address).cloned()
    }

    fn get_actor_code_cid(&self, addr: &Address) -> Option<Cid> {
        self.require_in_call();
        self.actor_code_cids.get(addr).cloned()
    }

    fn get_randomness_from_tickets(
        &self,
        tag: DomainSeparationTag,
        epoch: ChainEpoch,
        entropy: &[u8],
    ) -> Result<Randomness, ActorError> {
        let expected = self
            .expectations
            .borrow_mut()
            .expect_get_randomness_tickets
            .pop_front()
            .expect("unexpected call to get_randomness_from_tickets");

        assert!(epoch <= self.epoch, "attempt to get randomness from future");
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
    ) -> Result<Randomness, ActorError> {
        let expected = self
            .expectations
            .borrow_mut()
            .expect_get_randomness_beacon
            .take()
            .expect("unexpected call to get_randomness_from_beacon");

        assert!(epoch <= self.epoch, "attempt to get randomness from future");
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

    fn create<C: Cbor>(&mut self, obj: &C) -> Result<(), ActorError> {
        if self.state.is_some() {
            return Err(actor_error!(illegal_state; "state already constructed"));
        }
        self.state = Some(self.store_put(obj));
        Ok(())
    }

    fn state<C: Cbor>(&self) -> Result<C, ActorError> {
        Ok(self.store_get(self.state.as_ref().unwrap()))
    }

    fn transaction<C, RT, F>(&mut self, f: F) -> Result<RT, ActorError>
    where
        C: Cbor,
        F: FnOnce(&mut C, &mut Self) -> Result<RT, ActorError>,
    {
        if self.in_transaction {
            return Err(actor_error!(user_assertion_failed; "nested transaction"));
        }
        let mut read_only = self.state()?;
        self.in_transaction = true;
        let ret = f(&mut read_only, self);
        if ret.is_ok() {
            self.state = Some(self.store_put(&read_only));
        }
        self.in_transaction = false;
        ret
    }

    fn store(&self) -> &MemoryBlockstore {
        &self.store
    }

    fn send(
        &self,
        to: Address,
        method: MethodNum,
        params: RawBytes,
        value: TokenAmount,
    ) -> Result<RawBytes, ActorError> {
        self.require_in_call();
        if self.in_transaction {
            return Err(actor_error!(user_assertion_failed; "side-effect within transaction"));
        }

        assert!(
            !self.expectations.borrow_mut().expect_sends.is_empty(),
            "unexpected expectedMessage to: {:?} method: {:?}, value: {:?}, params: {:?}",
            to,
            method,
            value,
            params
        );

        let expected_msg = self.expectations.borrow_mut().expect_sends.pop_front().unwrap();

        assert!(
            expected_msg.to == to
                && expected_msg.method == method
                && expected_msg.params == params
                && expected_msg.value == value,
            "expectedMessage being sent does not match expectation.\n\
             Message  - to: {:?}, method: {:?}, value: {:?}, params: {:?}\n\
             Expected - to: {:?}, method: {:?}, value: {:?}, params: {:?}",
            to,
            method,
            value,
            params,
            expected_msg.to,
            expected_msg.method,
            expected_msg.value,
            expected_msg.params,
        );

        {
            let mut balance = self.balance.borrow_mut();
            if value > *balance {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_SENDER_STATE_INVALID,
                    format!("cannot send value: {:?} exceeds balance: {:?}", value, *balance),
                ));
            }
            *balance -= value;
        }

        match expected_msg.exit_code {
            ExitCode::OK => Ok(expected_msg.send_return),
            x => Err(ActorError::unchecked(x, "Expected message Fail".to_string())),
        }
    }

    fn new_actor_address(&mut self) -> Result<Address, ActorError> {
        self.require_in_call();
        let ret = *self.new_actor_addr.as_ref().expect("unexpected call to new actor address");
        self.new_actor_addr = None;
        Ok(ret)
    }

    fn create_actor(&mut self, code_id: Cid, actor_id: ActorID) -> Result<(), ActorError> {
        self.require_in_call();
        if self.in_transaction {
            return Err(actor_error!(user_assertion_failed; "side-effect within transaction"));
        }
        let expect_create_actor = self
            .expectations
            .borrow_mut()
            .expect_create_actor
            .take()
            .expect("unexpected call to create actor");

        assert!(expect_create_actor.code_id == code_id && expect_create_actor.actor_id == actor_id, "unexpected actor being created, expected code: {:?} address: {:?}, actual code: {:?} address: {:?}", expect_create_actor.code_id, expect_create_actor.actor_id, code_id, actor_id);
        Ok(())
    }

    fn delete_actor(&mut self, addr: &Address) -> Result<(), ActorError> {
        self.require_in_call();
        if self.in_transaction {
            return Err(actor_error!(user_assertion_failed; "side-effect within transaction"));
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
        self.circulating_supply.clone()
    }

    fn charge_gas(&mut self, _: &'static str, value: i64) {
        let mut exs = self.expectations.borrow_mut();
        assert!(!exs.expect_gas_charge.is_empty(), "unexpected gas charge {:?}", value);
        let expected = exs.expect_gas_charge.pop_front().unwrap();
        assert_eq!(expected, value, "expected gas charge {:?}, actual {:?}", expected, value);
    }

    fn base_fee(&self) -> TokenAmount {
        self.base_fee.clone()
    }
}

impl Primitives for MockRuntime {
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
        (*self.hash_func)(data)
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
            "Unexpected compute_unsealed_sector_cid : pieces mismatch"
        );

        if exp.exit_code != ExitCode::OK {
            return Err(anyhow!(ActorError::unchecked(
                exp.exit_code,
                "Expected Failure".to_string(),
            )));
        }
        Ok(exp.cid)
    }
}

impl Verifier for MockRuntime {
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

impl RuntimePolicy for MockRuntime {
    fn policy(&self) -> &Policy {
        &self.policy
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

// multihash library doesn't support poseidon hashing, so we fake it
#[derive(Clone, Copy, Debug, Eq, Multihash, PartialEq)]
#[mh(alloc_size = 64)]
enum MhCode {
    #[mh(code = 0xb401, hasher = multihash::Sha2_256)]
    PoseidonFake,
    #[mh(code = 0x1012, hasher = multihash::Sha2_256)]
    Sha256TruncPaddedFake,
}

pub fn make_cid(input: &[u8], prefix: u64) -> Cid {
    let hash = MhCode::Sha256TruncPaddedFake.digest(input);
    Cid::new_v1(prefix, hash)
}

pub fn make_piece_cid(input: &[u8]) -> Cid {
    make_cid(input, FIL_COMMITMENT_UNSEALED)
}

pub fn make_sealed_cid(input: &[u8]) -> Cid {
    make_cid(input, FIL_COMMITMENT_SEALED)
}

pub fn new_bls_addr(s: u8) -> Address {
    let seed = [s; 32];
    let mut rng: StdRng = SeedableRng::from_seed(seed);
    let mut key = [0u8; 48];
    rng.fill_bytes(&mut key);
    Address::new_bls(&key).unwrap()
}

/// Accumulates a sequence of messages (e.g. validation failures).
#[derive(Default)]
pub struct MessageAccumulator {
    /// Accumulated messages.
    /// This is a `Rc<RefCell>` to support accumulators derived from `with_prefix()` accumulating to
    /// the same underlying collection.
    msgs: Rc<RefCell<Vec<String>>>,
    /// Optional prefix to all new messages, e.g. describing higher level context.
    prefix: String,
}

impl MessageAccumulator {
    /// Returns a new accumulator backed by the same collection, that will prefix each new message with
    /// a formatted string.
    pub fn with_prefix(&self, prefix: &str) -> Self {
        MessageAccumulator { msgs: self.msgs.clone(), prefix: self.prefix.to_owned() + prefix }
    }

    pub fn is_empty(&self) -> bool {
        self.msgs.borrow().is_empty()
    }

    pub fn messages(&self) -> Vec<String> {
        self.msgs.borrow().to_owned()
    }

    /// Adds a message to the accumulator
    pub fn add(&self, msg: &str) {
        self.msgs.borrow_mut().push(format!("{}{msg}", self.prefix));
    }

    /// Adds messages from another accumulator to this one
    pub fn add_all(&self, other: &Self) {
        self.msgs.borrow_mut().extend_from_slice(&other.msgs.borrow());
    }

    /// Adds a message if predicate is false
    pub fn require(&self, predicate: bool, msg: &str) {
        if !predicate {
            self.add(msg);
        }
    }

    /// Adds a message if result is `Err`. Underlying error must be `Display`.
    pub fn require_no_error<V, E: Display>(&self, result: Result<V, E>, msg: &str) {
        if let Err(e) = result {
            self.add(&format!("{msg}: {e}"));
        }
    }
}

#[cfg(test)]
mod message_accumulator_test {
    use super::*;

    #[test]
    fn adds_messages() {
        let acc = MessageAccumulator::default();
        acc.add("Cthulhu");

        let msgs = acc.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs, vec!["Cthulhu"]);

        acc.add("Azathoth");
        let msgs = acc.messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs, vec!["Cthulhu", "Azathoth"]);
    }

    #[test]
    fn adds_on_predicate() {
        let acc = MessageAccumulator::default();
        acc.require(true, "Cthulhu");

        let msgs = acc.messages();
        assert_eq!(msgs.len(), 0);
        assert!(acc.is_empty());

        acc.require(false, "Azathoth");
        let msgs = acc.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs, vec!["Azathoth"]);
        assert!(!acc.is_empty());
    }

    #[test]
    fn require_no_error() {
        let fiasco: Result<(), String> = Err("fiasco".to_owned());
        let acc = MessageAccumulator::default();
        acc.require_no_error(fiasco, "Cthulhu says");

        let msgs = acc.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs, vec!["Cthulhu says: fiasco"]);
    }

    #[test]
    fn prefixes() {
        let acc = MessageAccumulator::default();
        acc.add("peasant");

        let gods_acc = acc.with_prefix("elder god -> ");
        gods_acc.add("Cthulhu");

        assert_eq!(acc.messages(), vec!["peasant", "elder god -> Cthulhu"]);
        assert_eq!(gods_acc.messages(), vec!["peasant", "elder god -> Cthulhu"]);
    }

    #[test]
    fn add_all() {
        let acc1 = MessageAccumulator::default();
        acc1.add("Cthulhu");

        let acc2 = MessageAccumulator::default();
        acc2.add("Azathoth");

        let acc3 = MessageAccumulator::default();
        acc3.add_all(&acc1);
        acc3.add_all(&acc2);

        assert_eq!(acc3.messages(), vec!["Cthulhu", "Azathoth"]);
    }
}
