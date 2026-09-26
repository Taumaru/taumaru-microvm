# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`AGENTS.md` holds the binding contributor rules (English-only artifacts, SDK/CLI boundary, SDK
no-panic/no-output rules, Design System for the CLI, quality gates). The governing source of
truth is `.specify/memory/constitution.md`. Read both before non-trivial changes; this file only
adds orientation that is not written down there.

## Commands

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features

# One crate / one integration test file / one test
cargo test -p taumaru-microvm --test failure_paths
cargo test -p taumaru-microvm-cli --test command_surface
cargo test -p taumaru-microvm <test_name_substring>

# Run the CLI (binary name is `microvm`)
cargo run -p taumaru-microvm-cli -- <command>
```

Set `TAUMARU_HOME=<dir>` to point the CLI at an isolated home instead of `~/.taumaru-microvm`.

## Workspace

- `crates/sdk` — package `taumaru-microvm`, published library. Everything public goes through
  deliberate re-exports in `src/lib.rs`; `lib.rs` Rustdoc also carries API migration notes
  (e.g. the 0.2 snapshot format change) — add compatibility notes there for breaking changes.
- `crates/cli` — package `taumaru-microvm-cli`, binary `microvm`, `publish = false`.
  `tests/package_boundary.rs` guards the dependency direction.

## SDK architecture

- `MicroVmSdk` (`src/manager.rs`, plus `src/manager/restore.rs`) is the single entry point and
  the only place lifecycle use cases live. `MicroVmSdk::new(home)` normalizes the home, creates
  the managed layout, and opens/migrates SQLite at `<home>/state/inventory.db`.
  `with_registry_base_url` exists so tests can point at a local fixture registry.
- Home layout: `artifacts/{kernels,rootfs}`, `tools/` (firecracker/firectl fallbacks),
  `vms/<name>/` (per-VM volume: `rootfs.ext4`, `firecracker.sock`, `firecracker.log`,
  `ssh/id_ed25519*`), `tmp/snapshots`, `state/inventory.db`. The manager validates persisted
  paths against this layout before acting and takes a per-VM name lock on `vms/<name>`.
- `ports/` are `pub(crate)` traits held as `Arc<dyn …>` on `MicroVmSdk`: repository, artifacts
  (registry), storage (ext4), credentials (ed25519), network (Linux host-only/LAN), runtime
  (Firecracker process), runtime_disk (loop + Device Mapper copy-on-write). `adapters/`
  implements each; `adapters/archive` implements the versioned tar+zstd snapshot format.
- **No persisted lifecycle state.** `MicroVmState` is verified at call time: `Running` iff the
  VM's control socket answers, otherwise `Stopped` (migration `0004` dropped the stored state).
  Do not reintroduce a stored state column.
- SQL migrations live in `crates/sdk/migrations/NNNN_*.sql` and are applied by
  `adapters/persistence/migrations.rs`; add a new numbered file rather than editing old ones.
- All failures are `SdkError` variants (`src/error.rs`); the CLI maps them to messages.

## CLI architecture

- `main.rs` parses (`cli.rs`, clap derive) and dispatches through `commands::run`; each command
  module receives a `CliContext` (SDK + `TerminalCapabilities`).
- Most commands need root (Firecracker, networking, Device Mapper). They call
  `privilege::require_privileged`, which re-executes the binary via `sudo`/`pkexec` with a
  `TAUMARU_ESCALATED` marker and forwards `TAUMARU_HOME`; in non-interactive mode it fails
  instead of prompting.
- `error.rs` renders every error as what happened / why / what next; exit code is `130` for
  cancellation, `1` otherwise. `output/human.rs` owns all human formatting; colour is disabled
  for non-TTY output and `NO_COLOR`.

## Tests

- SDK integration tests in `crates/sdk/tests/` use `tests/support/mod.rs` (`FixtureServer`, a
  local HTTP registry with fixture payloads from `tests/fixtures/`) with `tempfile` homes.
  Unit tests that need fake ports live in `#[cfg(test)]` modules next to the code, e.g. the
  large `mod tests` at the bottom of `manager.rs`.
- CLI surface tests (`crates/cli/tests/command_surface.rs`) drive the built binary.

## Feature workflow

Features are specified with Spec Kit under `specs/NNN-<slug>/` (spec, plan, research,
data-model, quickstart, tasks). Skills are in `.agents/skills/speckit-*`; helper scripts in
`.specify/scripts/bash/`. Check the relevant spec before changing a feature's behavior.
