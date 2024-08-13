# Built-in Filecoin actors

This repo contains the code for the on-chain built-in actors that power the
Filecoin network starting from network version 16, epoch 1960320 on 2022-07-06.

These actors are written in Rust and are designed to operate inside the
[Filecoin Virtual Machine](https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0030.md).
A reference implementation of the latter exists at
[filecoin-project/ref-fvm](https://github.com/filecoin-project/ref-fvm).

The build process of this repo compiles every actor into Wasm bytecode and
generates an aggregate bundle to be imported by all clients. The structure of
this bundle is standardized. Read below for details.

This codebase was canonicalized in [FIP-0031](https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0031.md).
As a result, this actor implementation is the only one recognized by the network
from network version 16 onwards.

## Pre-FVM actors

Actors for the following network versions prior to nv16 are implemented here as
well:

- nv14 actors to facilitate testing.
- nv15 actors to enable the nv15=>nv16 upgrade.

## Importable bundle

The main output of this repo is a [CARv1 archive](https://ipld.io/specs/transport/car/carv1/)
bundling all Wasm bytecode for all actors into a single file, with the following
characteristics:

- The CARv1 header points to a single root CID.
- The root CID resolves to a [DAG-CBOR](https://ipld.io/specs/codecs/dag-cbor/spec/)
  encoded block defining a `Manifest` type (defined below) containing a version
  number for the bundle format (currently always `1`) and a CID for a
  `ManifestPayload`.
- The `ManifestPayload` (defined below) is contained within a DAG-CBOR encoded
  block and defines a type that associates actor type names with their
  corresponding CIDs.
- The CIDs for all actors are contained within the same CARv1 archive as
  compiled Wasm bytecode contained within RAW blocks.

### Manifest [schema](https://ipld.io/docs/schemas/)

```ipldsch
# Manifest is encoded as: [version, CID]
type Manifest struct {
  version Int
  payload &ManifestPayload
} representation tuple

# ManifestPayload is encoded as: [ ["actorkey", CID], ["actorkey", CID], ... ]
#
# It alternatively may be interpreted as:
#   type ManifestPayload {String : &ActorBytecode} representation listpairs
# Or simply as a list of tuples.
type ManifestPayload struct {
  system &ActorBytecode
  init &ActorBytecode
  cron &ActorBytecode
  account &ActorBytecode
  storagepower &ActorBytecode
  storageminer &ActorBytecode
  storagemarket &ActorBytecode
  paymentchannel &ActorBytecode
  multisig &ActorBytecode
  reward &ActorBytecode
  verifiedregistry &ActorBytecode
  datacap &ActorBytecode
  placeholder &ActorBytecode
  evm &ActorBytecode
  eam &ActorBytecode
  ethaccount &ActorBytecode
} representation listpairs

# RAW block
type ActorBytecode bytes
```

Precompiled actor bundles are provided as [release binaries][releases] in this repo. The
[`fil_builtin_actors_bundle`](https://crates.io/crates/fil_builtin_actors_bundle) crate on
[crates.io](https://crates.io) will not be updated.

## [Releasing](RELEASE.md)

## Instructions for client implementations

### Obtaining an actors bundle

There are two options:

1. Building from source.
2. Downloading the precompiled release bundle from GitHub.

Instructions to build from source (option 1):

1. Clone the repo.
2. Check out the relevant branch or tag (see Versioning section below).
3. `make bundle` from the workspace root.

The bundle be written to `output/builtin-actors.car`.

Both options are compatible with automation via scripts or CI pipelines.

### Integrating an actors bundle

This part is implementation-specific. Options include:

1. Embedding the bundle's CARv1 bytes into the distribution's binary.
2. Downloading CARv1 files on start (with some form of checksumming for added security).

### Loading and using the actors bundle with ref-fvm

Once the implementation has validated the authenticity of the bundle, it is
expected to do the following:

1. Import the CARv1 into the blockstore.
2. Retain the root CID in memory, indexed by network version.
3. Feed the root CID to ref-fvm's Machine constructor, to tell ref-fvm which
   CodeCID maps to which built-in actor.

### Multiple network version support

Because every network version may be backed by different actor code,
implementations should be ready to load multiple actor bundles and index them
by network version.

When instantiating the ref-fvm Machine, both the network version and the
corresponding Manifest root CID must be passed.

## Versioning

A fair question is how crate versioning relates to the protocol concept of
`ActorVersion`. We adopt a policy similar to specs-actors:

- Major number in crate version correlates with `ActorVersion`.
- We generally don't use minor versions; these are always set to `0`.
- We strive for round major crate versions to denote the definitive release for
  a given network upgrade. However, due to the inability to predict certain
  aspects of software engineering, this is not a hard rule and further releases
  may be made by bumping the patch number.

Development versions will use qualifiers such as -rc (release candidate).

As an example of application of this policy to a v10 actor version lineage:

- Unstable development versions are referenced by commit hash.
- Stable development versions are tagged as release candidates: 10.0.0-rc1, 10.0.0-rc2, etc.
- Definitive release: 10.0.0.
- Patched definitive release: 10.0.1.
- Patched definitive release: 10.0.2.
- Network upgrade goes live with 10.0.2.

## About this codebase

### Relation to specs-actors

This repo supersedes [specs-actors](https://github.com/filecoin-project/specs-actors),
and fulfils two roles:
- executable specification of built-in actors.
- canonical, portable implementation of built-in actors.

### Credits

This codebase was originally forked from the actors v6 implementation of the
[Forest client](https://github.com/ChainSafe/forest/), and was adapted to the
FVM environment.

## Community

Because this codebase is a common good across all Filecoin client
implementations, it serves as a convergence area for all Core Devs regardless
of the implementation or project they identify with.

## License

Dual-licensed: [MIT](./LICENSE-MIT), [Apache Software License v2](./LICENSE-APACHE), by way of the
[Permissive License Stack](https://protocol.ai/blog/announcing-the-permissive-license-stack/).

Except the EVM precompile [test data](actors/evm/precompile-testdata), which is licensed under the
LGPL v3 and not included in crates or build artifacts.

[releases]: https://github.com/filecoin-project/builtin-actors/releases
