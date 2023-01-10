use hex_literal::hex;
use indicatif::ProgressBar;
use num_traits::Zero;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use tracing::{error, info, trace, warn};

use cid::multihash::{Code, MultihashDigest};
use fvm_shared::{address::Address, crypto::hash::SupportedHashes, econ::TokenAmount, IPLD_RAW};

use fil_actor_eam::EthAddress;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fvm_ipld_blockstore::{Block, Blockstore, MemoryBlockstore};
use fvm_ipld_encoding::{strict_bytes, BytesDe, Cbor};
use fvm_ipld_kamt::Config as KamtConfig;

use fil_actor_evm::{
    interpreter::system::{StateKamt, MAX_CODE_SIZE},
    state::State as EvmState,
};

use test_vm::{util::create_accounts, VM};

use crate::common::{B160, B256, SKIP_TESTS};

use super::models::{SpecName, TestSuit};

#[derive(Debug, Error)]
pub enum TestError {
    // #[error(" Test:{spec_id:?}:{id}, Root missmatched, Expected: {expect:?} got:{got:?}")]
    // RootMissmatch {
    //     spec_id: SpecId,
    //     id: usize,
    //     got: B256,
    //     expect: B256,
    // },
    #[error("Serde json error")]
    SerdeDeserialize(#[from] serde_json::Error),
    #[error("Internal system error")]
    SystemError,
    // #[error("Unknown private key: {private_key:?}")]
    // UnknownPrivateKey { private_key: B256 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct ContractParams(#[serde(with = "strict_bytes")] pub Vec<u8>);

impl Cbor for ContractParams {}

lazy_static::lazy_static! {
    // The Solidity compiler creates contiguous array item keys.
    // To prevent the tree from going very deep we use extensions,
    // which the Kamt supports and does in all cases.
    //
    // There are maximum 32 levels in the tree with the default bit width of 8.
    // The top few levels will have a higher level of overlap in their hashes.
    // Intuitively these levels should be used for routing, not storing data.
    //
    // The only exception to this is the top level variables in the contract
    // which solidity puts in the first few slots. There having to do extra
    // lookups is burdensome, and they will always be accessed even for arrays
    // because that's where the array length is stored.
    //
    // However, for Solidity, the size of the KV pairs is 2x256, which is
    // comparable to a size of a CID pointer plus extension metadata.
    // We can keep the root small either by force-pushing data down,
    // or by not allowing many KV pairs in a slot.
    //
    // The following values have been set by looking at how the charts evolved
    // with the test contract. They might not be the best for other contracts.
    static ref KAMT_CONFIG: KamtConfig = KamtConfig {
        min_data_depth: 0,
        bit_width: 5,
        max_array_width: 1
    };
}

lazy_static::lazy_static! {
    pub static ref MAP_CALLER_KEYS: HashMap<B256, B160> = {
        vec![
        (
            B256(hex!(
                "45a915e4d060149eb4365960e6a7a45f334393093061116b197e3240065ff2d8"
            )),
            B160(hex!("a94f5374fce5edbc8e2a8697c15331677e6ebf0b")),
        ),
        (
            B256(hex!(
                "c85ef7d79691fe79573b1a7064c19c1a9819ebdbd1faaab1a8ec92344438aaf4"
            )),
            B160(hex!("cd2a3d9f938e13cd947ec05abc7fe734df8dd826")),
        ),
        (
            B256(hex!(
                "044852b2a670ade5407e78fb2863c51de9fcb96542a07186fe3aeda6bb8a116d"
            )),
            B160(hex!("82a978b3f5962a5b0957d9ee9eef472ee55b42f1")),
        ),
        (
            B256(hex!(
                "6a7eeac5f12b409d42028f66b0b2132535ee158cfda439e3bfdd4558e8f4bf6c"
            )),
            B160(hex!("c9c5a15a403e41498b6f69f6f89dd9f5892d21f7")),
        ),
        (
            B256(hex!(
                "a95defe70ebea7804f9c3be42d20d24375e2a92b9d9666b832069c5f3cd423dd"
            )),
            B160(hex!("3fb1cd2cd96c6d5c0b5eb3322d807b34482481d4")),
        ),
        (
            B256(hex!(
                "fe13266ff57000135fb9aa854bbfe455d8da85b21f626307bf3263a0c2a8e7fe"
            )),
            B160(hex!("dcc5ba93a1ed7e045690d722f2bf460a51c61415")),
        ),
    ]
    .into_iter()
    .collect()
    };
}

#[allow(dead_code)]
fn skip_pre_test(test_name: &str, owner_address: &B160) -> bool {
    let rval = SKIP_TESTS.state.iter().any(|state_test| {
        state_test.pre_tests.as_ref().map_or_else(
            || false,
            |maybe_pre_tests| {
                if let Some(pre_test) = maybe_pre_tests.get(test_name) {
                    let owner_address_str = format!("{:#?}", owner_address);
                    pre_test.pre_owners[0] == "*"
                        || pre_test.pre_owners.contains(&owner_address_str)
                } else {
                    false
                }
            },
        )
    });

    if rval {
        warn!(
            "Skipping Pre Test test_name: '{:#?}', owner_address: '{:#?}'",
            test_name, owner_address,
        );
    }

    rval
}

fn skip_post_test(test_name: &str, chain_spec: &SpecName, test_id: usize) -> bool {
    let rval = SKIP_TESTS.state.iter().any(|state_test| {
        state_test.post_tests.as_ref().map_or_else(
            || false,
            |maybe_post_tests| {
                if let Some(post_test) = maybe_post_tests.get(test_name) {
                    chain_spec == &post_test.chain_spec
                        && (post_test.sub_numbers[0] == "*"
                            || post_test.sub_numbers.contains(&test_id.to_string()))
                } else {
                    false
                }
            },
        )
    });

    if rval {
        warn!(
            "Skipping Post Test test_name: '{:#?}', chain_spec: '{:#?}', test_id: {:#?}",
            test_name, chain_spec, test_id
        );
    }

    rval
}

fn execute_test_suit(path: &Path, elapsed: &Arc<Mutex<Duration>>) -> Result<(), TestError> {
    let json_reader = std::fs::read(path).unwrap();
    let suit: TestSuit = serde_json::from_reader(&*json_reader)?;

    let store = MemoryBlockstore::new();
    let test_vm = VM::new_with_singletons(&store);
    let account = create_accounts(&test_vm, 1, TokenAmount::from_whole(10_000))[0];
    let mut pre_contract_cache: HashMap<B160, Address> = HashMap::new();

    let timer = Instant::now();

    for (name, unit) in suit.0.iter() {
        // info!("{:#?}:{:#?}", name, unit);

        // TODO :: Process env block, is it needed for FEVM context ?

        // Process the "pre" block & deploy the contracts
        for (_test_id, (address, info)) in unit.pre.iter().enumerate() {
            assert!(
                info.code.len() < MAX_CODE_SIZE,
                "{}",
                format!(
                    "EVM byte code length ({}) is exceeding the maximum allowed of {MAX_CODE_SIZE}",
                    info.code.len()
                )
            );

            assert!(
                info.code.first() != Some(&0xEF),
                "EIP-3541: Contract code starting with the 0xEF byte is disallowed.",
            );

            let create_result = test_vm
                .apply_message(
                    account,
                    EAM_ACTOR_ADDR,
                    TokenAmount::zero(),
                    fil_actor_eam::Method::CreateAccount as u64,
                    Some(fil_actor_eam::InitAccountParams {
                        eth_address: EthAddress(address.as_bytes().try_into().unwrap()),
                    }),
                )
                .unwrap();

            assert!(
                create_result.code.is_success(),
                "failed to create the new actor :: {:#?} | Path :: {:#?}",
                create_result.message,
                path,
            );

            let create_return: fil_actor_eam::Create2Return =
                create_result.ret.deserialize().expect("failed to decode results");

            let actor_address = Address::new_id(create_return.actor_id);

            test_vm.mutate_state(actor_address, |st: &mut EvmState| {
                let hasher = Code::try_from(SupportedHashes::Keccak256 as u64).unwrap(); // supported hashes are all implemented in multihash
                let code_hash = multihash::Multihash::wrap(
                    SupportedHashes::Keccak256 as u64,
                    &hasher.digest(&info.code).to_bytes(),
                )
                .expect("failed to hash bytecode with keccak");

                let cid = store
                    .put(Code::Blake2b256, &Block::new(IPLD_RAW, info.code.clone()))
                    .expect("failed to write bytecode");

                let mut slots = StateKamt::new_with_config(store.clone(), KAMT_CONFIG.clone());

                st.bytecode = cid;
                st.bytecode_hash = code_hash;
                st.contract_state = slots.flush().expect("failed to flush contract state");
                st.nonce = info.nonce;
            });

            pre_contract_cache.insert(*address, actor_address);
        }

        // Process the "Post" & "transaction" block
        for (spec_name, tests) in &unit.post {
            for (id, test) in tests.iter().enumerate() {
                if skip_post_test(name.as_ref(), spec_name, id + 1) {
                    continue;
                }

                info!("Executing TestCase {:#?}::{:#?}::{:#?}", name, spec_name, id,);

                let gas_limit = *unit.transaction.gas_limit.get(test.indexes.gas).unwrap();
                let gas_limit = u64::try_from(gas_limit).unwrap_or(u64::MAX);
                let _tx_gas_limit = gas_limit;
                let tx_data = unit.transaction.data.get(test.indexes.data).unwrap().clone();
                let _tx_value = *unit.transaction.value.get(test.indexes.value).unwrap();

                if unit.transaction.to.is_none() {
                    continue;
                }

                let actor_address = pre_contract_cache.get(&unit.transaction.to.unwrap()).unwrap();

                let call_result = test_vm
                    .apply_message(
                        account,
                        *actor_address,
                        TokenAmount::zero(),
                        fil_actor_evm::Method::InvokeContract as u64,
                        Some(ContractParams(tx_data.to_vec())),
                    )
                    .unwrap();

                assert!(
                    call_result.code.is_success(),
                    "failed to call the new actor {}",
                    call_result.message
                );

                let BytesDe(_return_value) =
                    call_result.ret.deserialize().expect("failed to deserialize results");
            }
        }
    }

    let timer = timer.elapsed();

    *elapsed.lock().unwrap() += timer;

    Ok(())
}

pub fn run(test_files: Vec<PathBuf>, num_threads: usize) -> Result<(), TestError> {
    let endjob = Arc::new(AtomicBool::new(false));
    let console_bar = Arc::new(ProgressBar::new(test_files.len() as u64));
    let mut joins: Vec<std::thread::JoinHandle<Result<(), TestError>>> = Vec::new();
    let queue = Arc::new(Mutex::new((0, test_files.clone())));
    let elapsed = Arc::new(Mutex::new(std::time::Duration::ZERO));

    let num_threads = if num_threads > num_cpus::get() { num_cpus::get() } else { num_threads };

    for _ in 0..num_threads {
        let queue = queue.clone();
        let endjob = endjob.clone();
        let console_bar = console_bar.clone();
        let elapsed = elapsed.clone();

        joins.push(
            std::thread::Builder::new()
                .stack_size(50 * 1024 * 1024)
                .spawn(move || loop {
                    let (index, test_path) = {
                        let mut queue = queue.lock().unwrap();
                        if queue.1.len() <= queue.0 {
                            return Ok(());
                        }
                        let test_path = queue.1[queue.0].clone();
                        queue.0 += 1;
                        (queue.0 - 1, test_path)
                    };

                    if endjob.load(Ordering::SeqCst) {
                        return Ok(());
                    }

                    trace!("Calling testfile => {:#?}", test_path);

                    if let Err(err) = execute_test_suit(&test_path, &elapsed) {
                        endjob.store(true, Ordering::SeqCst);
                        error!(
                            "Test Failed => [{:#?}] path:{:#?} err:{:#?}",
                            index, test_path, err
                        );
                        return Err(err);
                    }

                    trace!("TestDone => {:#?}", test_path);
                    console_bar.inc(1);
                })
                .unwrap(),
        );
    }

    for handler in joins {
        handler.join().map_err(|_| TestError::SystemError)??;
    }

    console_bar.finish();
    info!(
        "Finished Processing of {:#?} Files in Time:{:#?}",
        test_files.len(),
        elapsed.lock().unwrap()
    );
    Ok(())
}
