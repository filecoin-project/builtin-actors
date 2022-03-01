# Built-in Filecoin actors

This repo contains the code for the on-chain built-in actors that power the
Filecoin network starting from network version 16.

These actors are written in Rust and are designed to operate inside the
[Filecoin Virtual Machine](https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0030.md).
A reference implementation of the latter exists at
[filecoin-project/ref-fvm](https://github.com/filecoin-project/ref-fvm).

The build process of this repo compiles every actor into Wasm bytecode and
generates an aggregate bundle to be imported by all clients. The structure of
this bundle is standardized. Read below for details.

This codebase is on track to be canonicalized in [FIP-0031](https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0031.md).
As a result, this actor implementation will be the only one recognized by the network.

## Importable bundle

The main output of this repo is a [CARv1 archive](https://ipld.io/specs/transport/car/carv1/)
bundling all Wasm bytecode for all actors into a single file, with the following
characteristics:

- The CARv1 header points to a single root CID.
- The CID resolves to a data structure that should be interpreted as an IPLD
  `Map<Cid, i32>`. Every entry represents a built-in actor.
  - Keys are CIDs pointing to the Wasm bytecode of an actor as a single block.
  - Values identify the actor type, and are to be interpreted according to the
    `fvm_shared::actor::builtin::Type` enum:
    - System = 1
    - Init = 2
    - Cron = 3
    - Account = 4
    - Power = 5
    - Miner = 6
    - Market = 7
    - PaymentChannel = 8
    - Multisig = 9
    - Reward = 10
    - VerifiedRegistry = 11

The CARv1 is embedded as a byte slice at the root of the library, and exported
under the `BUNDLE_CAR` public const, for easier consumption by Rust code.

## Instructions for clients

This is how client implementations are expected to use this repo. Steps 1-4 can
be automated as part of your build process, or can be performed manually.

1. Clone the repo.
2. Check out the relevant branch or tag (see Versioning section below).
3. Copy the CAR file generated the location printed in this log line:
    ```
   warning: bundle=/path/to/repo/target/debug/build/filecoin_canonical_actors_bundle-aef13b28a60e195b/out/bundle/bundle.car
   ```
   This line is only printed as a warning due to limitations in the Cargo build
   script mechanisms (preceded by other logs).
4. Embed the CAR file bytes into your binary.
5. At client start, import the embedded CAR file into a blockstore.
6. Retain the root CID in memory to pass it to the FVM implementation. If using
   ref-fvm, it expects it as a Machine constructor argument.

Because each network version is backed by different actor code, you will need
to repeat the steps above for every network version your client supports. We
advise to use some form of an array or lookup table mapping network versions to
their respective embedded bundles.

## Versioning

With the transition to Wasm actors, every network version that modifies built-in
actor code will result in a release of this repo. In practice, this breaks the
distinction between network version and actor versions. For further details on
this, refer to [FIP-0031](https://github.com/filecoin-project/FIPs/tree/master/FIPS/fip-0031.md#non-versioned-changes-and-state-tree-migrations).

For this reason, releases of this repo will be made with version numbers that
correlate to network versions, starting from v14.0.

## About this codebase

### Relation to specs-actors

This repo supersedes [specs-actors](https://github.com/filecoin-project/specs-actors),
and fulfils two roles:
- executable specification of built-in actors.
- canonical, portable implementation of built-in actors.

### Credits

This codebase was originally forked from the Chocolate  [Forest client](https://github.com/ChainSafe/forest/)
and was adapted to the FVM environment.

## Community

Because this codebase is a common good across all Filecoin client
implementations, it serves as a convergence area for all Core Devs regardless
of the implementation or project they identify with.

## License

Dual-licensed: [MIT](./LICENSE-MIT), [Apache Software License v2](./LICENSE-APACHE), by way of the
[Permissive License Stack](https://protocol.ai/blog/announcing-the-permissive-license-stack/).