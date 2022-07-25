use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use std::cmp::max;

#[derive(Debug, PartialEq, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct BeneficiaryTerm {
    // Quota: The total amount the current beneficiary can withdraw. Monotonic, but reset when beneficiary changes.
    #[serde(with = "bigint_ser")]
    pub quota: TokenAmount,
    // UsedQuota: The amount of quota the current beneficiary has already withdrawn
    #[serde(with = "bigint_ser")]
    pub used_quota: TokenAmount,
    // Expiration: The epoch at which the beneficiary's rights expire and revert to the owner
    pub expiration: ChainEpoch,
}

impl Cbor for BeneficiaryTerm {}

impl BeneficiaryTerm {
    pub fn default() -> BeneficiaryTerm {
        BeneficiaryTerm {
            quota: TokenAmount::default(),
            expiration: 0,
            used_quota: TokenAmount::default(),
        }
    }

    pub fn new(
        quota: TokenAmount,
        used_quota: TokenAmount,
        expiration: ChainEpoch,
    ) -> BeneficiaryTerm {
        BeneficiaryTerm { quota, expiration, used_quota }
    }

    // IsUsedUp check whether beneficiary has use up all quota
    pub fn is_used_up(&self) -> bool {
        self.used_quota >= self.quota
    }

    // IsExpire check if the beneficiary is within the validity period
    pub fn is_expire(&self, cur: ChainEpoch) -> bool {
        self.expiration <= cur
    }

    // Available get the amount that the beneficiary has not yet withdrawn
    pub fn available(&self, cur: ChainEpoch) -> TokenAmount {
        // Return 0 when the usedQuota > Quota for safe
        if self.is_expire(cur) {
            TokenAmount::default()
        } else {
            max(self.quota.clone() - self.used_quota.clone(), TokenAmount::default())
        }
    }
}

#[derive(Debug, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct PendingBeneficiaryChange {
    pub new_beneficiary: Address,
    #[serde(with = "bigint_ser")]
    pub new_quota: TokenAmount,
    pub new_expiration: ChainEpoch,
    pub approved_by_beneficiary: bool,
    pub approved_by_nominee: bool,
}

impl Cbor for PendingBeneficiaryChange {}

impl PendingBeneficiaryChange {
    pub fn new(
        new_beneficiary: Address,
        new_quota: TokenAmount,
        new_expiration: ChainEpoch,
    ) -> Self {
        PendingBeneficiaryChange {
            new_beneficiary,
            new_quota,
            new_expiration,
            approved_by_beneficiary: false,
            approved_by_nominee: false,
        }
    }
}
