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

If there is any conflict between this file and `../lotus/AGENTS.md`, prefer the Lotus file.
