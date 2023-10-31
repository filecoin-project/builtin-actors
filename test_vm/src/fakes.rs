use anyhow::anyhow;
use anyhow::Error;
use anyhow::Result;
use cid::multihash::Code;
use cid::multihash::MultihashDigest;
use cid::Cid;
use fvm_shared::address::{Address, SECP_PUB_LEN};
use fvm_shared::consensus::ConsensusFault;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::crypto::signature::{Signature, SECP_SIG_LEN, SECP_SIG_MESSAGE_HASH_SIZE};
use fvm_shared::piece::PieceInfo;
use fvm_shared::sector::AggregateSealVerifyProofAndInfos;
use fvm_shared::sector::RegisteredSealProof;
use fvm_shared::sector::ReplicaUpdateInfo;
use fvm_shared::sector::SealVerifyInfo;
use fvm_shared::sector::WindowPoStVerifyInfo;
use integer_encoding::VarInt;

use fil_actors_runtime::runtime::Primitives;
use fil_actors_runtime::test_utils::{make_piece_cid, recover_secp_public_key};

/// Fake implementation of runtime primitives.
#[derive(Default, Clone)]
#[allow(clippy::type_complexity)]
pub struct FakePrimitives {
    pub hash_blake2b: Option<fn(&[u8]) -> [u8; 32]>,
    pub hash: Option<fn(SupportedHashes, &[u8]) -> Vec<u8>>,
    pub hash_64: Option<fn(SupportedHashes, &[u8]) -> ([u8; 64], usize)>,
    pub compute_unsealed_sector_cid:
        Option<fn(RegisteredSealProof, &[PieceInfo]) -> Result<Cid, Error>>,
    pub recover_secp_public_key: Option<
        fn(
            &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
            &[u8; SECP_SIG_LEN],
        ) -> Result<[u8; SECP_PUB_LEN], Error>,
    >,
    pub verify_post: Option<fn(&WindowPoStVerifyInfo) -> Result<(), Error>>,
    pub verify_consensus_fault:
        Option<fn(&[u8], &[u8], &[u8]) -> Result<Option<ConsensusFault>, Error>>,
    pub batch_verify_seals: Option<fn(&[SealVerifyInfo]) -> Result<Vec<bool>>>,
    pub verify_aggregate_seals: Option<fn(&AggregateSealVerifyProofAndInfos) -> Result<(), Error>>,
    pub verify_signature: Option<fn(&Signature, &Address, &[u8]) -> Result<(), Error>>,
    pub verify_replica_update: Option<fn(&ReplicaUpdateInfo) -> Result<(), Error>>,
}

impl Primitives for FakePrimitives {
    fn hash_blake2b(&self, data: &[u8]) -> [u8; 32] {
        if let Some(override_fn) = self.hash_blake2b {
            override_fn(data)
        } else {
            blake2b_simd::Params::new()
                .hash_length(32)
                .to_state()
                .update(data)
                .finalize()
                .as_bytes()
                .try_into()
                .unwrap()
        }
    }

    fn hash(&self, hasher: SupportedHashes, data: &[u8]) -> Vec<u8> {
        if let Some(override_fn) = self.hash {
            override_fn(hasher, data)
        } else {
            let hasher = Code::try_from(hasher as u64).unwrap(); // supported hashes are all implemented in multihash
            hasher.digest(data).digest().to_owned()
        }
    }

    fn hash_64(&self, hasher: SupportedHashes, data: &[u8]) -> ([u8; 64], usize) {
        if let Some(override_fn) = self.hash_64 {
            override_fn(hasher, data)
        } else {
            let hasher = Code::try_from(hasher as u64).unwrap();
            let (len, buf, ..) = hasher.digest(data).into_inner();
            (buf, len as usize)
        }
    }

    fn compute_unsealed_sector_cid(
        &self,
        proof_type: RegisteredSealProof,
        pieces: &[PieceInfo],
    ) -> Result<Cid, Error> {
        if let Some(override_fn) = self.compute_unsealed_sector_cid {
            override_fn(proof_type, pieces)
        } else {
            let mut buf: Vec<u8> = Vec::new();
            let ptv: i64 = proof_type.into();
            buf.extend(ptv.encode_var_vec());
            for p in pieces {
                buf.extend(&p.cid.to_bytes());
                buf.extend(p.size.0.encode_var_vec())
            }
            Ok(make_piece_cid(&buf))
        }
    }

    fn verify_signature(
        &self,
        signature: &Signature,
        signer: &Address,
        plaintext: &[u8],
    ) -> Result<(), Error> {
        if let Some(override_fn) = self.verify_signature {
            return override_fn(signature, signer, plaintext);
        }

        // default behaviour expects signature bytes to be equal to plaintext
        if signature.bytes != plaintext {
            return Err(anyhow::format_err!(
                "invalid signature (mock sig validation expects siggy bytes to be equal to plaintext)"
            ));
        }
        Ok(())
    }

    fn recover_secp_public_key(
        &self,
        hash: &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
        signature: &[u8; SECP_SIG_LEN],
    ) -> Result<[u8; SECP_PUB_LEN], Error> {
        if let Some(override_fn) = self.recover_secp_public_key {
            override_fn(hash, signature)
        } else {
            recover_secp_public_key(hash, signature)
                .map_err(|_| anyhow!("failed to recover pubkey"))
        }
    }

    fn verify_replica_update(&self, replica: &ReplicaUpdateInfo) -> Result<(), Error> {
        if let Some(override_fn) = self.verify_replica_update {
            override_fn(replica)
        } else {
            Ok(())
        }
    }

    fn verify_post(&self, verify_info: &WindowPoStVerifyInfo) -> Result<(), Error> {
        if let Some(override_fn) = self.verify_post {
            override_fn(verify_info)
        } else {
            Ok(())
        }
    }

    fn verify_consensus_fault(
        &self,
        h1: &[u8],
        h2: &[u8],
        extra: &[u8],
    ) -> Result<Option<ConsensusFault>, Error> {
        if let Some(override_fn) = self.verify_consensus_fault {
            override_fn(h1, h2, extra)
        } else {
            Ok(None)
        }
    }

    fn batch_verify_seals(&self, batch: &[SealVerifyInfo]) -> Result<Vec<bool>> {
        if let Some(override_fn) = self.batch_verify_seals {
            override_fn(batch)
        } else {
            Ok(vec![true; batch.len()])
        }
    }

    fn verify_aggregate_seals(
        &self,
        aggregate: &AggregateSealVerifyProofAndInfos,
    ) -> Result<(), Error> {
        if let Some(override_fn) = self.verify_aggregate_seals {
            override_fn(aggregate)
        } else {
            Ok(())
        }
    }
}
