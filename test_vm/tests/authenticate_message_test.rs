use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_account::Method::AuthenticateMessageExported;
use fil_actors_runtime::test_utils::hash;
use fil_actors_runtime::EAM_ACTOR_ID;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::crypto::hash::SupportedHashes::Keccak256;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use test_vm::util::{apply_code, apply_ok, create_accounts, generate_deal_proposal};
use test_vm::VM;

// Using a deal proposal as a serialized message, we confirm that:
// - calls to Account::authenticate_message with valid signatures succeed
// - calls to Account::authenticate_message with invalid signatures fail
#[test]
fn account_authenticate_message() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);
    let addr = create_accounts(&v, 1, TokenAmount::from_whole(10_000))[0];

    let proposal =
        generate_deal_proposal(addr, addr, TokenAmount::zero(), TokenAmount::zero(), 0, 0);
    let proposal_ser =
        RawBytes::serialize(proposal).expect("failed to marshal deal proposal").to_vec();

    // With a good sig, message succeeds
    let authenticate_message_params = AuthenticateMessageParams {
        signature: proposal_ser.clone(),
        message: proposal_ser.clone(),
    };
    apply_ok(
        &v,
        addr,
        addr,
        TokenAmount::zero(),
        AuthenticateMessageExported as u64,
        Some(authenticate_message_params),
    );

    // Bad, bad sig! message fails
    let authenticate_message_params =
        AuthenticateMessageParams { signature: vec![], message: proposal_ser };
    apply_code(
        &v,
        addr,
        addr,
        TokenAmount::zero(),
        AuthenticateMessageExported as u64,
        Some(authenticate_message_params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
}

// Using a deal proposal as a serialized message, we confirm that:
// - calls to EthAccount::authenticate_message with valid signatures succeed
#[test]
fn ethaccount_authenticate_message_success() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);
    let addr = create_accounts(&v, 1, TokenAmount::from_whole(10_000))[0];
    let rng = &mut ChaCha8Rng::seed_from_u64(0);
    let secret_key = libsecp256k1::SecretKey::random(rng);

    let proposal =
        generate_deal_proposal(addr, addr, TokenAmount::zero(), TokenAmount::zero(), 0, 0);
    let proposal_ser =
        RawBytes::serialize(proposal).expect("failed to marshal deal proposal").to_vec();

    let msg_hash = get_hash_for_signature(&proposal_ser);
    let (good_sig, recovery_id) = libsecp256k1::sign(&msg_hash, &secret_key);

    let pub_key = libsecp256k1::recover(&msg_hash, &good_sig, &recovery_id).unwrap();
    let pub_key_ser = pub_key.serialize();
    let pub_key_hash = hash(Keccak256, &pub_key_ser[1..]).0;

    let eth_addr = Address::new_delegated(EAM_ACTOR_ID, &pub_key_hash[12..32]).unwrap();

    // Create a Placeholder by sending to it
    apply_ok(&v, addr, eth_addr, TokenAmount::from_whole(2), METHOD_SEND, None::<RawBytes>);

    // Create the EthAccount by sending from the Placeholder
    apply_ok(&v, eth_addr, addr, TokenAmount::from_whole(1), METHOD_SEND, None::<RawBytes>);

    let mut good_sig_ser = [0; 65];
    good_sig_ser[..64].copy_from_slice(&good_sig.serialize());
    good_sig_ser[64] = recovery_id.serialize();

    // With a good sig, message succeeds
    let authenticate_message_params =
        AuthenticateMessageParams { signature: good_sig_ser.to_vec(), message: proposal_ser };
    apply_ok(
        &v,
        addr,
        eth_addr,
        TokenAmount::zero(),
        AuthenticateMessageExported as u64,
        Some(authenticate_message_params),
    );
}

// Using a deal proposal as a serialized message, we confirm that
// calls to EthAccount::authenticate_message with invalid signatures fail
#[test]
fn ethaccount_authenticate_message_failure() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);
    let addr = create_accounts(&v, 1, TokenAmount::from_whole(10_000))[0];
    let rng = &mut ChaCha8Rng::seed_from_u64(0);
    let secret_key = libsecp256k1::SecretKey::random(rng);

    let proposal =
        generate_deal_proposal(addr, addr, TokenAmount::zero(), TokenAmount::zero(), 0, 0);
    let proposal_ser =
        RawBytes::serialize(proposal).expect("failed to marshal deal proposal").to_vec();

    let msg_hash = get_hash_for_signature(&proposal_ser);
    let (good_sig, recovery_id) = libsecp256k1::sign(&msg_hash, &secret_key);

    let pub_key = libsecp256k1::recover(&msg_hash, &good_sig, &recovery_id).unwrap();
    let pub_key_ser = pub_key.serialize();
    let pub_key_hash = hash(Keccak256, &pub_key_ser[1..]).0;

    let eth_addr = Address::new_delegated(EAM_ACTOR_ID, &pub_key_hash[12..32]).unwrap();

    // Create a Placeholder by sending to it
    apply_ok(&v, addr, eth_addr, TokenAmount::from_whole(2), METHOD_SEND, None::<RawBytes>);

    // Create the EthAccount by sending from the Placeholder
    apply_ok(&v, eth_addr, addr, TokenAmount::from_whole(1), METHOD_SEND, None::<RawBytes>);

    // To test a bad sig, we sign the correct payload with a different key (this is a bit more comprehensive than simply flipping a bit)

    let other_key = libsecp256k1::SecretKey::random(rng);
    assert_ne!(secret_key, other_key);

    let (bad_sig, bad_recovery_id) = libsecp256k1::sign(&msg_hash, &other_key);
    let mut bad_sig_ser = [0; 65];
    bad_sig_ser[..64].copy_from_slice(&bad_sig.serialize());
    bad_sig_ser[64] = bad_recovery_id.serialize();

    let authenticate_message_params =
        AuthenticateMessageParams { signature: bad_sig_ser.to_vec(), message: proposal_ser };
    apply_code(
        &v,
        addr,
        addr,
        TokenAmount::zero(),
        AuthenticateMessageExported as u64,
        Some(authenticate_message_params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
}

fn get_hash_for_signature(bytes: &[u8]) -> libsecp256k1::Message {
    let hash: [u8; 32] = blake2b_simd::Params::new()
        .hash_length(32)
        .to_state()
        .update(bytes)
        .finalize()
        .as_bytes()
        .try_into()
        .expect("fixed array size");

    libsecp256k1::Message::parse(&hash)
}
