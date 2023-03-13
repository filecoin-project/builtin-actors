use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct ConstructorParams {
    pub governor: Address,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct NameReturn {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct SymbolReturn {
    pub symbol: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct TotalSupplyReturn {
    pub supply: TokenAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct BalanceParams {
    pub address: Address,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct BalanceReturn {
    pub balance: TokenAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct GetAllowanceReturn {
    pub allowance: TokenAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct IncreaseAllowanceReturn {
    pub new_allowance: TokenAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct DecreaseAllowanceReturn {
    pub new_allowance: TokenAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct RevokeAllowanceReturn {
    pub old_allowance: TokenAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct MintParams {
    // Recipient of the newly minted tokens.
    pub to: Address,
    // Amount of tokens to mint.
    pub amount: TokenAmount,
    // Addresses to be granted effectively-infinite operator allowance for the recipient.
    pub operators: Vec<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct DestroyParams {
    pub owner: Address,
    pub amount: TokenAmount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct GranularityReturn {
    pub granularity: u64,
}
