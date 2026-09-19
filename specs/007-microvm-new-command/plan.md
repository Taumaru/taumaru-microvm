# Implementation Plan: CLI `new` Command for Guided MicroVM Creation

**Branch**: `007-microvm-new-command` | **Date**: 2026-09-19 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/007-microvm-new-command/spec.md`

## Summary

Add a CLI-only guided creation flow `microvm new [NAME]` that collects name,
image, disk, memory, vCPUs, and network mode through `inquire` prompts (every
value also accepted as a flag), confirms once, then provisions prerequisites in
dependency order (runtime bundle → default kernel → selected image) and calls
the existing `create_microvm` operation with its progress observer feeding one
per-step presentation. Reuse the download command's catalog, signal handling,
and renderer patterns. One minimal additive SDK read-only query
(`is_distribution_image_ready`) backs the per-image downloaded marker; no
schema migration, no new dependency, no lifecycle semantics change.

## Technical Context

**Language/Version**: Rust edition 2024 (workspace toolchain; no new MSRV
declared by this feature).

**Primary Dependencies**: Existing `clap` 4.x (command surface with optional
positional name), `inquire` 0.9.x (`Text`/`Select`/`Confirm` prompts),
`indicatif` 0.18.x (truthful terminal progress), `semver` 1.x (deterministic
runtime-package ordering, CLI-internal), `thiserror` 2.x (CLI error
composition), `tokio` 1.x with the existing workspace features,
`tokio-util` 0.7.x (SDK-owned cooperative cancellation token), and the local
`taumaru-microvm` SDK. No new workspace dependencies. Do not add a direct
`crossterm`, HTTP, SQLite, hashing, or Firecracker dependency to the CLI.

**Storage**: No schema change. `MicroVmSdk` keeps owning the normalized
inventory and its migration lifecycle. The new SDK query is read-only against
the existing `resolve_distribution_image` path plus file-integrity recheck.
The CLI holds only an ephemeral in-memory request and terminal result for one
invocation.

**Testing**: `cargo test -p taumaru-microvm-cli --all-targets --all-features`
for focused CLI tests first, then
`cargo test -p taumaru-microvm --all-targets --all-features` for the new SDK
query, followed by the workspace gates: `cargo fmt --all -- --check`,
`cargo check --all-targets --all-features`,
`cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-targets --all-features`. SDK tests use the existing registry
fixture (`crates/sdk/tests/fixtures/manifest.json`) and deterministic download
fakes; CLI tests use an SDK-shaped fake client and never touch the network or
a real terminal.

**Target Platform**: Linux host CLI with interactive TTY and non-TTY/script
execution. Output must remain understandable with `NO_COLOR`, limited terminal
width, or no terminal.

**Project Type**: Rust workspace CLI application consuming a publishable Rust
SDK library.

**Performance Goals**: Show discovery status immediately; show a running state
within one second after confirmation under normal terminal conditions; stream
progress without buffering artifacts; make no second transfer for SDK-valid
cache hits; keep request construction linear in listed images.

**Constraints**: `main.rs` stays thin. All registry, cache, integrity,
persistence, and Firecracker behavior stays behind the SDK. No transfer starts
before confirmation (interactive) or validation (non-interactive). Runtime
failure stops dependent work. Abort-on-invalid with no re-prompt loop.
Expected failures return formatted nonzero CLI results without panic, stack
trace, or unsolicited SDK output. Creation-phase Ctrl-C settles before
reporting; provisioning-phase Ctrl-C exits `130`. All repository artifacts
and user-facing product strings remain in English.

**Scale/Scope**: Exactly one image per invocation with its distribution
default kernel, the managed default VM directory, and automatic network
addressing. Kernel override, custom volume path, explicit LAN address,
multiple images, and start/stop/connect lifecycle operations are out of scope.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Pre-Phase 0 Gate

| Principle | Status | Design response |
|---|---|---|
| I. SDK-First Shared Core | PASS | Creation and acquisition live in `MicroVmSdk`; the CLI only prompts, validates display forms, sequences SDK calls, and renders results. One additive read-only query; no second lifecycle engine. |
| II. Panic-Free, Silent SDK Boundary | PASS | New SDK query returns `Result<bool, SdkError>` with Rustdoc; no printing, logging, process control, or global state. CLI maps typed errors to three-part messages. |
| III. Explicit Local State and Lifecycle | PASS | CLI passes the explicit resolved home to the SDK and never opens SQLite; readiness reuses the per-image inventory rows. |
| IV. Closed for Modification, Open for Extension | PASS | Additive SDK method plus reuse of existing download catalog/executor patterns; CLI changes go through the existing `cli.rs`/`commands`/`output` boundary. No second engine. |
| V. Calm, Accessible CLI and Intentional Documentation | PASS | Guided prompts with abort-on-invalid, single image select with text markers, sequential truthful progress, text-plus-symbol status, non-color/narrow modes, `130` on provisioning interrupt. |

No constitution violations require a complexity exception.

### Post-Phase 1 Gate

| Check | Status | Evidence |
|---|---|---|
| Dependency direction | PASS | `crates/cli` depends on `crates/sdk`; no SDK-to-CLI dependency. Public SDK addition is one read-only method on `MicroVmSdk`, no new exported type. |
| SDK ownership | PASS | `commands/new.rs` calls list/download/create methods and renders results; no hash, path, inventory, or Firecracker logic in the CLI. Marker comes from the SDK boolean, never from CLI path inspection. |
| Home and database ownership | PASS | `context.rs` resolves `TAUMARU_HOME` or the default; `MicroVmSdk` owns directory creation and migrations. No schema change. |
| Plan safety | PASS | Immutable request plus explicit confirmation gate precede all SDK calls. Runtime failure is terminal for dependents. Abort-on-invalid before any transfer. |
| Async boundary | PASS | `main` runs the async CLI through Tokio; SDK calls are awaited sequentially in dependency order; callbacks update the renderer. Creation Ctrl-C settles the await before reporting. |
| Output accessibility | PASS | Text labels accompany semantic emphasis; non-TTY, `NO_COLOR`, and narrow-terminal modes stay line-oriented and deterministic. |
| Testability | PASS | Pure parsing plus SDK-shaped fake clients cover input rules, ordering, and failure paths; SDK fixture tests cover the new query without the public registry. |

No post-design gate is violated.

## Project Structure

### Documentation (this feature)

```text
specs/007-microvm-new-command/
├── plan.md                         # This file ($speckit-plan command output)
├── research.md                     # Phase 0 decisions and alternatives
├── data-model.md                   # Ephemeral request/result model and state transitions
├── quickstart.md                   # Operator and verification guide
├── contracts/
│   ├── sdk-image-readiness.md      # New read-only SDK query contract
│   └── cli-new-command.md          # New command input, output, exit-status contract
└── tasks.md                        # Phase 2 output ($speckit-tasks command)
```

### Source Code (repository root)

```text
Cargo.toml                            # unchanged (no new dependencies)
Cargo.lock                            # unchanged unless tooling refreshes it
crates/sdk/
├── src/
│   ├── lib.rs                       # unchanged (method needs no new export)
│   ├── manager.rs                   # is_distribution_image_ready (read-only)
│   └── tests (integration)          # readiness: ready, missing, stale, error paths
crates/cli/
├── src/
│   ├── cli.rs                       # Command::New + NewArgs (positional name + flags)
│   ├── commands/
│   │   ├── mod.rs                   # dispatch Command::New
│   │   ├── download.rs              # widen visibility of catalog/executor helpers (no behavior change)
│   │   └── new.rs                   # request collection, parsing, prompts, executor, summary
│   ├── error.rs                     # new-command validation/cancel/result messaging (download text untouched)
│   └── output/
│       └── human.rs                 # new-command review, progress lines, creation summary
└── tests/
    ├── command_surface.rs           # new entry help, non-interactive rejection, boundary checks
    └── package_boundary.rs          # unchanged expectations (no new CLI dependencies)
```

**Structure Decision**: Keep the existing two-crate layout and module
ownership. No new crates, top-level directories, or speculative layers. The
SDK addition is one method on `MicroVmSdk` reusing the private resolution
path. CLI changes stay inside `cli.rs`, `commands/new.rs` (new),
`commands/download.rs` (visibility only), `output/human.rs`, and `error.rs`.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| — | — | — |

No violations. Table retained as empty by template convention.
