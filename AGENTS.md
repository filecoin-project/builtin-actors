# Agents Notes

This repository inherits its agent workflow, conventions, and project guidance from the Lotus workstream.

- For the canonical instructions, see `../lotus/AGENTS.md`.
- Treat that document as the source of truth for coding style, review etiquette, CI expectations, and how agents should coordinate across repos in this workstream.

Repo‑local quick notes:
- Tests: `make test` (uses cargo-nextest across the workspace).
- Lint: `make check` (clippy; warnings are errors).
- Formatting: `make rustfmt`.
- EIP‑7702 is always active in this bundle (no runtime NV gating).
- EIP‑7702 design notes live at `../eip7702.md`.

Current Work Priority (EIP‑7702)
- Interpreter minimalized: CALL/STATICCALL to EOAs route via `METHOD_SEND`; no internal delegation re‑follow.
- Legacy EVM ApplyAndCall/InvokeAsEoa removed; `InvokeAsEoaWithRoot` remains for VM intercept.
- Decode robustness: no unwraps or silent fallbacks; decode errors return `illegal_state`.
- Tests (green):
  - EVM: core unit tests; no legacy 7702 tests.
  - EthAccount: tuple cap boundary, duplicates under receiver‑only, value‑transfer short‑circuit; nonce init/increment covered with one ignored test.

Quick Validation
- Build/lint/tests (workspace):
  - `make check && cargo test -p fil_actor_evm && cargo test -p fil_actor_ethaccount`
- Docker bundle + ref‑fvm tests: see `../lotus/AGENTS.md` for the Docker harness and commands.

If there is any conflict between this file and `../lotus/AGENTS.md`, prefer the Lotus file.
