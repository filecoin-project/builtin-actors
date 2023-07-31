use frc46_token::receiver::{FRC46TokenReceived, FRC46_TOKEN_TYPE};
use frc46_token::token::types::BurnParams;
use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};
use fvm_shared::{ActorID, METHOD_SEND};
use num_traits::Zero;

use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_datacap::BalanceParams;
use fil_actor_market::{
    BatchActivateDealsParams, OnMinerSectorsTerminateParams, SectorDeals,
    VerifyDealsForActivationParams,
};
use fil_actor_miner::ext::verifreg::ClaimID;
use fil_actor_miner::{IsControllingAddressParam, PowerPair};
use fil_actor_power::{UpdateClaimedPowerParams, UpdatePledgeTotalParams};
use fil_actor_verifreg::GetClaimsParams;
use fil_actors_runtime::{
    BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};

use vm_api::trace::ExpectInvocation;

/// Static helper functions for creating invocation expectations.
pub struct Expect {}

impl Expect {
    pub fn send(from: Address, to: Address, v: Option<TokenAmount>) -> ExpectInvocation {
        ExpectInvocation { from, to, method: METHOD_SEND, value: v, ..Default::default() }
    }
    pub fn burn(from: Address, v: Option<TokenAmount>) -> ExpectInvocation {
        Self::send(from, BURNT_FUNDS_ACTOR_ADDR, v)
    }
    pub fn market_activate_deals(
        from: Address,
        deals: Vec<DealID>,
        sector_number: SectorNumber,
        sector_expiry: ChainEpoch,
        sector_type: RegisteredSealProof,
    ) -> ExpectInvocation {
        let params = IpldBlock::serialize_cbor(&BatchActivateDealsParams {
            sectors: vec![SectorDeals {
                sector_number,
                deal_ids: deals,
                sector_expiry,
                sector_type,
            }],
        })
        .unwrap();
        ExpectInvocation {
            from,
            to: STORAGE_MARKET_ACTOR_ADDR,
            method: fil_actor_market::Method::BatchActivateDeals as u64,
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn market_sectors_terminate(
        from: Address,
        epoch: ChainEpoch,
        sectors: Vec<SectorNumber>,
    ) -> ExpectInvocation {
        let bf = BitField::try_from_bits(sectors).unwrap();
        let params =
            IpldBlock::serialize_cbor(&OnMinerSectorsTerminateParams { epoch, sectors: bf })
                .unwrap();
        ExpectInvocation {
            from,
            to: STORAGE_MARKET_ACTOR_ADDR,
            method: fil_actor_market::Method::OnMinerSectorsTerminate as u64,
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn market_verify_deals(from: Address, sectors: Vec<SectorDeals>) -> ExpectInvocation {
        let params =
            IpldBlock::serialize_cbor(&VerifyDealsForActivationParams { sectors }).unwrap();
        ExpectInvocation {
            from,
            to: STORAGE_MARKET_ACTOR_ADDR,
            method: fil_actor_market::Method::VerifyDealsForActivation as u64,
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn miner_cron(to: Address) -> ExpectInvocation {
        ExpectInvocation {
            from: STORAGE_POWER_ACTOR_ADDR,
            to,
            method: fil_actor_miner::Method::OnDeferredCronEvent as u64,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn miner_is_controlling_address(
        from: Address,
        to: Address,
        address: Address,
    ) -> ExpectInvocation {
        let params = IpldBlock::serialize_cbor(&IsControllingAddressParam { address }).unwrap();
        ExpectInvocation {
            from,
            to,
            method: fil_actor_miner::Method::IsControllingAddressExported as u64,
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn power_current_total(from: Address) -> ExpectInvocation {
        ExpectInvocation {
            from,
            to: STORAGE_POWER_ACTOR_ADDR,
            method: fil_actor_power::Method::CurrentTotalPower as u64,
            subinvocs: Some(vec![]),
            value: Some(TokenAmount::zero()),
            ..Default::default()
        }
    }
    pub fn power_enrol_cron(from: Address) -> ExpectInvocation {
        // Note: params are unchecked.
        ExpectInvocation {
            from,
            to: STORAGE_POWER_ACTOR_ADDR,
            method: fil_actor_power::Method::EnrollCronEvent as u64,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn power_submit_porep(from: Address) -> ExpectInvocation {
        // Note: params are unchecked.
        ExpectInvocation {
            from,
            to: STORAGE_POWER_ACTOR_ADDR,
            method: fil_actor_power::Method::SubmitPoRepForBulkVerify as u64,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn power_update_claim(from: Address, delta: PowerPair) -> ExpectInvocation {
        let params = IpldBlock::serialize_cbor(&UpdateClaimedPowerParams {
            raw_byte_delta: delta.raw,
            quality_adjusted_delta: delta.qa,
        })
        .unwrap();
        ExpectInvocation {
            from,
            to: STORAGE_POWER_ACTOR_ADDR,
            method: fil_actor_power::Method::UpdateClaimedPower as u64,
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn power_update_pledge(from: Address, amount: Option<TokenAmount>) -> ExpectInvocation {
        let params = amount.map(|a| {
            IpldBlock::serialize_cbor(&UpdatePledgeTotalParams { pledge_delta: a }).unwrap()
        });
        ExpectInvocation {
            from,
            to: STORAGE_POWER_ACTOR_ADDR,
            method: fil_actor_power::Method::UpdatePledgeTotal as u64,
            params,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn reward_update_kpi() -> ExpectInvocation {
        // Note: params are unchecked
        ExpectInvocation {
            from: STORAGE_POWER_ACTOR_ADDR,
            to: REWARD_ACTOR_ADDR,
            method: fil_actor_reward::Method::UpdateNetworkKPI as u64,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn reward_this_epoch(from: Address) -> ExpectInvocation {
        ExpectInvocation {
            from,
            to: REWARD_ACTOR_ADDR,
            method: fil_actor_reward::Method::ThisEpochReward as u64,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn verifreg_get_claims(
        from: Address,
        miner: ActorID,
        ids: Vec<ClaimID>,
    ) -> ExpectInvocation {
        let params =
            IpldBlock::serialize_cbor(&GetClaimsParams { provider: miner, claim_ids: ids })
                .unwrap();
        ExpectInvocation {
            from,
            to: VERIFIED_REGISTRY_ACTOR_ADDR,
            method: fil_actor_verifreg::Method::GetClaims as u64,
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn frc42_balance(from: Address, to: Address, address: Address) -> ExpectInvocation {
        let params = Some(IpldBlock::serialize_cbor(&BalanceParams { address }).unwrap());
        ExpectInvocation {
            from,
            to,
            method: frc42_dispatch::method_hash!("Balance"),
            params,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn frc44_authenticate(
        from: Address,
        to: Address,
        message: Vec<u8>,
        signature: Vec<u8>,
    ) -> ExpectInvocation {
        let params =
            IpldBlock::serialize_cbor(&AuthenticateMessageParams { message, signature }).unwrap();
        ExpectInvocation {
            from,
            to,
            method: frc42_dispatch::method_hash!("AuthenticateMessage"),
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn frc46_burn(from: Address, to: Address, amount: TokenAmount) -> ExpectInvocation {
        let params = IpldBlock::serialize_cbor(&BurnParams { amount }).unwrap();
        ExpectInvocation {
            from,
            to,
            method: frc42_dispatch::method_hash!("Burn"),
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn frc46_receiver(
        from: Address,
        to: Address,
        payer: ActorID,
        beneficiary: ActorID,
        operator: ActorID,
        amount: TokenAmount,
        operator_data: Option<RawBytes>,
    ) -> ExpectInvocation {
        let payload = IpldBlock::serialize_cbor(&FRC46TokenReceived {
            from: payer,
            to: beneficiary,
            operator,
            amount,
            operator_data: operator_data.unwrap_or_default(),
            token_data: RawBytes::default(),
        })
        .unwrap();
        let params = IpldBlock::serialize_cbor(&UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: payload.unwrap().data.into(),
        })
        .unwrap();
        ExpectInvocation {
            from,
            to,
            method: frc42_dispatch::method_hash!("Receive"),
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
}
