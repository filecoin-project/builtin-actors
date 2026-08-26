use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::strict_bytes;
use fvm_ipld_encoding::Error;

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    pub validator: Address,
}

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ActivateSectorParams {
    pub sector_numbers: BitField,
    pub verifier_signature: Vec<u8>,
}

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ActivateSectorReturn {
    pub sector_numbers: BitField,
}

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct CompactSectorNumbersParams {
    pub mask_sector_numbers: BitField,
}

pub const SIGNATURE_DOMAIN_SEPARATION_SEALER_NUMBERS: &[u8] = b"fil_sealernumbers:";

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct VerifierSignaturePayload {
    #[serde(with = "strict_bytes")]
    pub domain: Vec<u8>,
    pub sector_numbers: BitField,
    pub sealer_id_actor: Address,
    pub miner_actor: Address,
}

impl VerifierSignaturePayload {
    pub fn new(sector_numbers: BitField, sealer_id_actor: Address, miner_actor: Address) -> Self {
        Self { 
            domain: SIGNATURE_DOMAIN_SEPARATION_SEALER_NUMBERS.to_vec(), 
            sector_numbers, 
            sealer_id_actor,
            miner_actor,
        }
    }
    
    pub fn serialize(&self) -> Result<Vec<u8>, Error> {
        fvm_ipld_encoding::to_vec(self)
    }
}

