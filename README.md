# Built-in Filecoin actors (v8)

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

## Pre-FVM actors

Actors for the following network versions are provided as well:

- nv14 actors are provided to facilitate testing.
- nv15 actors are provided to enable the eventual nv15=>nv16 upgrade.

## Importable bundle

The main output of this repo is a [CARv1 archive](https://ipld.io/specs/transport/car/carv1/)
bundling all Wasm bytecode for all actors into a single file, with the following
characteristics:

- The CARv1 header points to a single root CID.
- The CID resolves to a Manifest data structure that associates code CIDs with
  their corresponding built-in actor types.
- The Manifest payload should be interpreted as an IPLD `Map<Cid, i32>`. Every
  entry represents a built-in actor.
- Manifest keys (CID) point to the Wasm bytecode of an actor as a single block.
- Manifest values (i32) identify the actor type, to be parsed as the
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

Precompiled actor bundles may also be provided as release binaries in this repo,
if requested by implementors.

## Releasing

We usually release all actors, the runtime, and the bundle at the same time. That means releasing:

- `fil_actors_runtime`
- `fil_actor_account`
- `fil_actor_cron`
- `fil_actor_init`
- `fil_actor_market`
- `fil_actor_miner`
- `fil_actor_multisig`
- `fil_actor_paych`
- `fil_actor_power`
- `fil_actor_reward`
- `fil_actor_system`
- `fil_actor_verifreg`
- `fil_builtin_actors_bundle`

(in that order)

To make this easier, we've added some helper scripts to the Makefile. Instructions follow.

### 1: Install Dependencies

Install:

- `jq` (with your favorite package manager)
- `cargo-edit` (with `cargo install cargo-edit`).

### 2: Bump Versions (Release)

You can bump the runtime, actors, and bundle versions with the `bump-version` target. See [Versioning](#versioning) to determine the correct version bump.

```bash
make bump-version
```

By default, this bumps the patch version. To bump to a different version, append, e.g. `BUMP=major`. Valid options are:

- `patch`
- `minor`
- `major`
- `alpha`
- `beta`

You can also _set_ a specific version with the `set-version` target.

```bash
make set-version VERSION=7.1.1
```

Commit the version changes:

```bash
git commit -a -m "Release $(make --quiet version)"
```

Finally, create a PR to commit your changes, make sure your PR is approved and merged before move to the next step!

### 3: Publish Crates

**NOTE:** If you're a not a member of the core FVM team, you'll need help with this step. Please
make a PR at this point and ask the core team to publish a release.

Run `make publish` to publish all crates to crates.io. This will likely take a while as it re-builds
everything from scratch for validation (multiple times).

**NOTE**: To do this, you'll need to:

1. Register an account with `https://crates.io` and confirm your email address (if you haven't already).
2. Login locally with `cargo login`.
3. Get yourself added to the [fvm-crate-owners](https://github.com/orgs/filecoin-project/teams/fvm-crate-owners) team.

### 4: Bump Versions (Alpha)

Finally, bump the versions to the next alpha and commit the changes:

```bash
make bump-version BUMP=alpha
git commit -a -m "Release $(make --quiet version)"
```

## Instructions for client implementations

### Obtaining an actors bundle

There are two options:

1. Building from source.
2. Downloading the precompiled release bundle from GitHub.

Instructions to build from source (option 1):

1. Clone the repo.
2. Check out the relevant branch or tag (see Versioning section below).
3. `cargo run -- -o my_bundle.car` from the workspace root.

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
