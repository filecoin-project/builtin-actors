use fil_actor_miner::{ChangeBeneficiaryParams, Method as MinerMethod};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredSealProof;
use test_vm::util::{
    apply_code, change_beneficiary, change_beneficiary_, change_owner_address,
    change_owner_address_, create_accounts, create_accounts_, create_miner, create_miner_,
    get_beneficiary,
};
use test_vm::TestVM;

enum ConcreteVM<'vm, 'bs> {
    TestVM(&'vm TestVM<'bs>),
    _BenchmarkVM(()),
}

/// Sample test that can be injected with either a TestVM (rust execution, stack traceable etc) or a
/// _BenchmarkVM (wasm execution, gas metering etc)
fn change_owner_test(concrete_vm: ConcreteVM) {
    let vm = match concrete_vm {
        ConcreteVM::TestVM(tvm) => tvm.vm(),
        ConcreteVM::_BenchmarkVM(_) => todo!(),
    };

    // when the tests cases are merged, remove the match and use the concrete_vm directly
    let addrs = create_accounts_(vm, 3, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, new_owner, beneficiary) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // create miner
    let miner_id = create_miner_(
        vm,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    )
    .0;

    change_beneficiary_(
        vm,
        owner,
        miner_id,
        &ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 100),
    );
    change_owner_address_(vm, owner, miner_id, new_owner);
    match concrete_vm {
        ConcreteVM::TestVM(tvm) => {
            let miner_info = tvm.get_miner_info(miner_id);
            assert_eq!(new_owner, miner_info.pending_owner_address.unwrap());
        }
        #[cfg(feature = "benchmark")]
        ConcreteVM::_BenchmarkVM(_) => {
            // this block shouldn't compile unless the feature is enabled
            compilation_error;
        }
        _ => {
            unimplemented!("Only TestVM and BenchmarkVM are supported")
        }
    }

    change_owner_address_(vm, new_owner, miner_id, new_owner);

    match concrete_vm {
        ConcreteVM::TestVM(tvm) => {
            let miner_info = tvm.get_miner_info(miner_id);
            assert!(miner_info.pending_owner_address.is_none());
            assert_eq!(new_owner, miner_info.owner);
            assert_eq!(new_owner, miner_info.beneficiary);
        }
        ConcreteVM::_BenchmarkVM(_) => todo!(),
    }
}

#[cfg(feature = "benchmark")]
#[test]
fn benchmark_change_owner_success() {
    println!("Running benchmark test");
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    change_owner_test(ConcreteVM::TestVM(&v));
    v.assert_state_invariants();
}

#[cfg(not(feature = "benchmark"))]
#[test]
fn test_change_owner_success() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    change_owner_test(ConcreteVM::TestVM(&v));
    v.assert_state_invariants();
}

// #[test]
// fn change_owner_success() {
//     let store = MemoryBlockstore::new();
//     let mut v = TestVM::new_with_singletons(&store);
//     let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
//     let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
//     let (owner, worker, new_owner, beneficiary) = (addrs[0], addrs[0], addrs[1], addrs[2]);

//     // create miner
//     let miner_id = create_miner(
//         &mut v,
//         owner,
//         worker,
//         seal_proof.registered_window_post_proof().unwrap(),
//         TokenAmount::from_whole(1_000),
//     )
//     .0;

//     change_beneficiary(
//         &v,
//         owner,
//         miner_id,
//         &ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 100),
//     );
//     change_owner_address(&v, owner, miner_id, new_owner);
//     let miner_info = v.get_miner_info(miner_id);
//     assert_eq!(new_owner, miner_info.pending_owner_address.unwrap());

//     change_owner_address(&v, new_owner, miner_id, new_owner);
//     let miner_info = v.get_miner_info(miner_id);
//     assert!(miner_info.pending_owner_address.is_none());
//     assert_eq!(new_owner, miner_info.owner);
//     assert_eq!(new_owner, miner_info.beneficiary);

//     v.assert_state_invariants();
// }

// #[test]
// fn keep_beneficiary_when_owner_changed() {
//     let store = MemoryBlockstore::new();
//     let mut v = TestVM::new_with_singletons(&store);
//     let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
//     let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
//     let (owner, worker, new_owner, beneficiary) = (addrs[0], addrs[0], addrs[1], addrs[2]);

//     // create miner
//     let miner_id = create_miner(
//         &mut v,
//         owner,
//         worker,
//         seal_proof.registered_window_post_proof().unwrap(),
//         TokenAmount::from_whole(1_000),
//     )
//     .0;

//     change_beneficiary(
//         &v,
//         owner,
//         miner_id,
//         &ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 100),
//     );
//     change_beneficiary(
//         &v,
//         beneficiary,
//         miner_id,
//         &ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 100),
//     );
//     assert_eq!(beneficiary, get_beneficiary(&v, worker, miner_id).active.beneficiary);

//     change_owner_address(&v, owner, miner_id, new_owner);
//     change_owner_address(&v, new_owner, miner_id, new_owner);
//     let miner_info = v.get_miner_info(miner_id);
//     assert!(miner_info.pending_owner_address.is_none());
//     assert_eq!(new_owner, miner_info.owner);
//     assert_eq!(beneficiary, miner_info.beneficiary);

//     v.assert_state_invariants();
// }

// #[test]
// fn change_owner_fail() {
//     let store = MemoryBlockstore::new();
//     let mut v = TestVM::new_with_singletons(&store);
//     let addrs = create_accounts(&v, 4, TokenAmount::from_whole(10_000));
//     let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
//     let (owner, worker, new_owner, addr) = (addrs[0], addrs[0], addrs[1], addrs[2]);

//     // create miner
//     let miner_id = create_miner(
//         &mut v,
//         owner,
//         worker,
//         seal_proof.registered_window_post_proof().unwrap(),
//         TokenAmount::from_whole(1_000),
//     )
//     .0;

//     // only owner can proposal
//     apply_code(
//         &v,
//         addr,
//         miner_id,
//         TokenAmount::zero(),
//         MinerMethod::ChangeOwnerAddress as u64,
//         Some(new_owner),
//         ExitCode::USR_FORBIDDEN,
//     );

//     change_owner_address(&v, owner, miner_id, new_owner);
//     // proposal must be the same
//     apply_code(
//         &v,
//         new_owner,
//         miner_id,
//         TokenAmount::zero(),
//         MinerMethod::ChangeOwnerAddress as u64,
//         Some(addr),
//         ExitCode::USR_ILLEGAL_ARGUMENT,
//     );
//     // only pending can confirm
//     apply_code(
//         &v,
//         addr,
//         miner_id,
//         TokenAmount::zero(),
//         MinerMethod::ChangeOwnerAddress as u64,
//         Some(new_owner),
//         ExitCode::USR_FORBIDDEN,
//     );
//     //only miner can change proposal
//     apply_code(
//         &v,
//         addr,
//         miner_id,
//         TokenAmount::zero(),
//         MinerMethod::ChangeOwnerAddress as u64,
//         Some(addr),
//         ExitCode::USR_FORBIDDEN,
//     );

//     //miner change proposal
//     change_owner_address(&v, owner, miner_id, addr);
//     //confirm owner proposal
//     change_owner_address(&v, addr, miner_id, addr);
//     let miner_info = v.get_miner_info(miner_id);
//     assert!(miner_info.pending_owner_address.is_none());
//     assert_eq!(addr, miner_info.owner);
//     assert_eq!(addr, miner_info.beneficiary);

//     v.assert_state_invariants();
// }
