// TODO
// This constant inlines the CAR bundle containing the bytecode for
// canonical actors.
//
// The CAR contains a multiroot header, enumerating the CIDs of the
// bytecode blocks that follow.
//
// For now, each bytecode entry is a single IPLD slab; content is not chunked
// or laid out in a DAG.
// pub const BUNDLE_CAR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundle.car"));
