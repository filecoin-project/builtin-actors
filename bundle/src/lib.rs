/// The bundled CAR embedded as a byte slice for easy consumption by Rust programs.
///
/// The root CID of the CAR points to an actor index data structure. It is a
/// CBOR-encoded IPLD Map<String, Cid>, enumerating actor name and their
/// respective CIDs.
///
/// The actor names are values from this enumeration:
///
/// - "account"
/// - "cron"
/// - "init"
/// - "market"
/// - "miner"
/// - "multisig"
/// - "paych"
/// - "power"
/// - "reward"
/// - "system"
/// - "verifreg"
///
/// The Filecoin client must import the contents of CAR into the blockstore, but
/// may opt to exclude the index data structure.
pub const BUNDLE_CAR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundle/bundle.car"));
