use crate::{
    PieceActivationManifest, PieceChange, SectorChanges, SectorContentChangedParams,
    SectorContentChangedReturn, SECTOR_CONTENT_CHANGED,
};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{
    actor_error, ActorError, AsActorError, SendError, STORAGE_MARKET_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;

use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorNumber;
use fvm_shared::sys::SendFlags;
use num_traits::Zero;
use std::collections::BTreeMap;

pub struct ActivationNotifications<'a> {
    pub sector_number: SectorNumber,
    pub sector_expiration: ChainEpoch,
    pub pieces: &'a [PieceActivationManifest],
}

/// Sends notifications of sector and piece activation to nominated receiving actors.
/// Inputs are per-sector, matching parameters provided by external callers.
/// This method groups inputs by receiving actor and sends a single notification per actor.
/// Notifications are fire-and-forget, so there is no return value.
/// If require_success is true, this method will return an error if any of the notifications fail
/// or are rejected by the receiving actor.
pub fn notify_data_consumers(
    rt: &impl Runtime,
    activations: &[ActivationNotifications],
    require_success: bool,
) -> Result<(), ActorError> {
    // Inputs are grouped by sector -> piece -> notifee+payload
    // Regroup by notifee -> sector -> piece+payload for sending.
    let mut activations_by_notifee =
        BTreeMap::<Address, BTreeMap<SectorNumber, Vec<PieceChange>>>::new();
    // Collect each sector's expiration for sending to each receiver of a notification.
    let sector_expirations: BTreeMap<SectorNumber, ChainEpoch> =
        activations.iter().map(|a| (a.sector_number, a.sector_expiration)).collect();
    for activation in activations {
        for piece in activation.pieces {
            for notifee in &piece.notify {
                activations_by_notifee
                    .entry(notifee.address)
                    .or_default()
                    .entry(activation.sector_number)
                    .or_default()
                    .push(PieceChange {
                        data: piece.cid,
                        size: piece.size,
                        payload: notifee.payload.clone(),
                    });
            }
        }
    }

    for (notifee, payloads) in activations_by_notifee {
        // Reject notifications to any actor other than the built-in market.
        if notifee != STORAGE_MARKET_ACTOR_ADDR {
            if require_success {
                return Err(
                    actor_error!(illegal_argument; "disallowed notification receiver: {}", notifee),
                );
            }
            continue;
        }
        let sectors_changes: Vec<SectorChanges> = payloads
            .into_iter()
            .map(|(sector_number, pieces)| SectorChanges {
                sector: sector_number,
                minimum_commitment_epoch: *sector_expirations.get(&sector_number).unwrap(),
                added: pieces,
            })
            .collect();

        let response = send_notification(
            rt,
            &notifee,
            SectorContentChangedParams { sectors: sectors_changes.clone() },
        );
        if require_success {
            match response {
                Ok(response) => {
                    validate_notification_response(&notifee, &sectors_changes, &response)?;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

// Sends a notification to one receiver and handles errors and syntactically invalid responses.
fn send_notification(
    rt: &impl Runtime,
    notifee: &Address,
    params: SectorContentChangedParams,
) -> Result<SectorContentChangedReturn, ActorError> {
    let gas_limit = None;
    let send_flags = SendFlags::default();
    let ret = rt.send(
        notifee,
        SECTOR_CONTENT_CHANGED,
        IpldBlock::serialize_cbor(&params)?,
        TokenAmount::zero(),
        gas_limit,
        send_flags,
    );
    match ret {
        Ok(r) => {
            if r.exit_code.is_success() {
                if let Some(data) = r.return_data {
                    // Success with non-empty return data.
                    data.deserialize().context_code(
                        crate::ERR_NOTIFICATION_RESPONSE_INVALID,
                        "invalid return data serialization",
                    )
                } else {
                    Err(ActorError::checked(
                        crate::ERR_NOTIFICATION_RESPONSE_INVALID,
                        "no return data".to_string(),
                        None,
                    ))
                }
            } else {
                Err(ActorError::checked(
                    crate::ERR_NOTIFICATION_RECEIVER_ABORTED,
                    format!("receiver aborted with {}", r.exit_code),
                    None,
                ))
            }
        }
        Err(SendError(e)) => Err(ActorError::checked(
            crate::ERR_NOTIFICATION_SEND_FAILED,
            format!("send error {}", e),
            None,
        )),
    }
}

// Checks that a notification response shape matches the request shape, and that
// all notifications were accepted.
fn validate_notification_response(
    notifee: &Address,
    request: &[SectorChanges],
    response: &SectorContentChangedReturn,
) -> Result<(), ActorError> {
    if response.sectors.len() != request.len() {
        return Err(ActorError::checked(
            crate::ERR_NOTIFICATION_RESPONSE_INVALID,
            "sector change response mismatched sectors".to_string(),
            None,
        ));
    }
    for (sreq, sresp) in request.iter().zip(response.sectors.iter()) {
        if sresp.added.len() != sreq.added.len() {
            return Err(ActorError::checked(
                crate::ERR_NOTIFICATION_RESPONSE_INVALID,
                format!("sector change response mismatched pieces for sector {}", sreq.sector),
                None,
            ));
        }
        for (nreq, nresp) in sreq.added.iter().zip(sresp.added.iter()) {
            if !nresp.accepted {
                return Err(ActorError::checked(
                    crate::ERR_NOTIFICATION_REJECTED,
                    format!(
                        "sector change rejected by {} for sector {} piece {} payload {:?}",
                        notifee, sreq.sector, nreq.data, nreq.payload
                    ),
                    None,
                ));
            }
        }
    }
    Ok(())
}
