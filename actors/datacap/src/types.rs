use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct MintParams {
    // Recipient of the newly minted tokens.
    pub to: Address,
    // Amount of tokens to mint.
    pub amount: TokenAmount,
    // Addresses to be granted operator allowance for the newly minted tokens.
    pub operators: Vec<Address>,
}

impl Cbor for MintParams {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct DestroyParams {
    pub owner: Address,
    pub amount: TokenAmount,
}

impl Cbor for DestroyParams {}
