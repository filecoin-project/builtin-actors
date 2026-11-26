use cid::Cid;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorNumber;
use fvm_shared::{ActorID, METHOD_SEND};
use num_traits::Zero;

use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_market::OnMinerSectorsTerminateParams;
use fil_actor_miner::{IsControllingAddressParam, PowerPair};
use fil_actor_miner::{PieceChange, SectorChanges, SectorContentChangedParams};
use fil_actor_power::{UpdateClaimedPowerParams, UpdatePledgeTotalParams};
use fil_actors_runtime::{
    BURNT_FUNDS_ACTOR_ADDR, EventBuilder, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR,
    STORAGE_MARKET_ACTOR_ID, STORAGE_POWER_ACTOR_ADDR, STORAGE_POWER_ACTOR_ID,
};

use vm_api::trace::{EmittedEvent, ExpectInvocation};

/// Static helper functions for creating invocation expectations.
pub struct Expect {}

impl Expect {
    pub fn send(from: ActorID, to: Address, v: Option<TokenAmount>) -> ExpectInvocation {
        ExpectInvocation { from, to, method: METHOD_SEND, value: v, ..Default::default() }
    }
    pub fn burn(from: ActorID, v: Option<TokenAmount>) -> ExpectInvocation {
        Self::send(from, BURNT_FUNDS_ACTOR_ADDR, v)
    }
    pub fn market_content_changed(
        from: ActorID,
        deals: Vec<DealID>,
        client_id: ActorID,
        sector_number: SectorNumber,
        sector_expiry: ChainEpoch,
        pieces_changes: Vec<PieceChange>,
    ) -> ExpectInvocation {
        let params = IpldBlock::serialize_cbor(&SectorContentChangedParams {
            sectors: vec![SectorChanges {
                sector: sector_number,
                minimum_commitment_epoch: sector_expiry,
                added: pieces_changes,
            }],
        })
        .unwrap();

        let events: Vec<EmittedEvent> = deals
            .iter()
            .map(|deal_id| Expect::build_market_event("deal-activated", *deal_id, client_id, from))
            .collect();

        ExpectInvocation {
            from,
            to: STORAGE_MARKET_ACTOR_ADDR,
            method: fil_actor_market::Method::SectorContentChangedExported as u64,
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            events: Some(events),
            ..Default::default()
        }
    }
    pub fn market_sectors_terminate(
        from: ActorID,
        epoch: ChainEpoch,
        sectors: Vec<SectorNumber>,
        deals: Vec<(DealID, ActorID)>,
    ) -> ExpectInvocation {
        let bf = BitField::try_from_bits(sectors).unwrap();
        let params =
            IpldBlock::serialize_cbor(&OnMinerSectorsTerminateParams { epoch, sectors: bf })
                .unwrap();

        let events: Vec<EmittedEvent> = deals
            .into_iter()
            .map(|(deal_id, client)| {
                Expect::build_market_event("deal-terminated", deal_id, client, from)
            })
            .collect();

        ExpectInvocation {
            from,
            to: STORAGE_MARKET_ACTOR_ADDR,
            method: fil_actor_market::Method::OnMinerSectorsTerminate as u64,
            params: Some(params),
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![Expect::burn(STORAGE_MARKET_ACTOR_ID, None)]),
            events: Some(events),
            ..Default::default()
        }
    }
    pub fn miner_cron(to: Address) -> ExpectInvocation {
        ExpectInvocation {
            from: STORAGE_POWER_ACTOR_ID,
            to,
            method: fil_actor_miner::Method::OnDeferredCronEvent as u64,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn miner_is_controlling_address(
        from: ActorID,
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
    pub fn power_current_total(from: ActorID) -> ExpectInvocation {
        ExpectInvocation {
            from,
            to: STORAGE_POWER_ACTOR_ADDR,
            method: fil_actor_power::Method::CurrentTotalPower as u64,
            subinvocs: Some(vec![]),
            value: Some(TokenAmount::zero()),
            ..Default::default()
        }
    }
    pub fn power_enrol_cron(from: ActorID) -> ExpectInvocation {
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
    pub fn power_update_claim(from: ActorID, delta: PowerPair) -> ExpectInvocation {
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
    pub fn power_update_pledge(from: ActorID, amount: Option<TokenAmount>) -> ExpectInvocation {
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
            from: STORAGE_POWER_ACTOR_ID,
            to: REWARD_ACTOR_ADDR,
            method: fil_actor_reward::Method::UpdateNetworkKPI as u64,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }
    pub fn reward_this_epoch(from: ActorID) -> ExpectInvocation {
        ExpectInvocation {
            from,
            to: REWARD_ACTOR_ADDR,
            method: fil_actor_reward::Method::ThisEpochReward as u64,
            value: Some(TokenAmount::zero()),
            subinvocs: Some(vec![]),
            ..Default::default()
        }
    }

    pub fn frc44_authenticate(
        from: ActorID,
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
    #[allow(clippy::too_many_arguments)]
    pub fn build_market_event(
        typ: &str,
        deal_id: DealID,
        client: ActorID,
        provider: ActorID,
    ) -> EmittedEvent {
        EmittedEvent {
            emitter: STORAGE_MARKET_ACTOR_ID,
            event: EventBuilder::new()
                .typ(typ)
                .field_indexed("id", &deal_id)
                .field_indexed("client", &client)
                .field_indexed("provider", &provider)
                .build()
                .unwrap(),
        }
    }

    pub fn build_miner_event(
        typ: &str,
        miner_id: ActorID,
        sector_number: SectorNumber,
    ) -> EmittedEvent {
        EmittedEvent {
            emitter: miner_id,
            event: EventBuilder::new()
                .typ(typ)
                .field_indexed("sector", &sector_number)
                .build()
                .unwrap(),
        }
    }

    pub fn build_sector_activation_event(
        typ: &str,
        miner_id: ActorID,
        sector_number: SectorNumber,
        unsealed_cid: Option<Cid>,
        pieces: &Vec<(Cid, u64)>,
    ) -> EmittedEvent {
        let mut base_event = EventBuilder::new()
            .typ(typ)
            .field_indexed("sector", &sector_number)
            .field_indexed("unsealed-cid", &unsealed_cid);

        for piece in pieces {
            base_event =
                base_event.field_indexed("piece-cid", &piece.0).field("piece-size", &piece.1);
        }

        EmittedEvent { emitter: miner_id, event: base_event.build().unwrap() }
    }
}
