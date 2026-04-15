# EVM Precompile Test Data

These data files come from [go-ethereum](https://github.com/ethereum/go-ethereum/tree/master/core/vm/testdata/precompiles) and are therefore licensed under the LGPLv3. However, they're not included in published crates (they're excluded by cargo) or build artifacts. They're only loaded at runtime by a few unit tests, and therefore trivially meet the requirements of the LGPL.

## EIP-7951 P256VERIFY vectors

`eip7951_p256verify.json` is vendored from:

- Source URL: <https://eips.ethereum.org/assets/eip-7951/test-vectors.json>
- Retrieved: `2026-02-27` (UTC)
- Upstream reference: `ethereum/EIPs@1fe9c4d8710dce7c437f42c749e5390bec668c29` (`master` at retrieval time)
- Local file SHA256: `6ed75dffbc0dd70defc7aceb7299df75dfb2a18184fbac229482c0a4b6601d6d`

Tests consume these vectors for secp256r1/P256VERIFY behavior checks. The parser supports `NoBenchmark` filtering, although the current upstream set marks all vectors as benchmarkable.
