# Implementation Plan: Image Artifact Download

**Branch**: `005-image-artifact-download` | **Date**: 2026-09-19 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/005-image-artifact-download/spec.md`

## Summary

Add additive single-image acquisition to the SDK (`download_distribution_image` plus
cancellation-aware variant, returning a new `DownloadedDistributionImage`) while keeping the
existing whole-distribution operations untouched. Rebuild the CLI download flow around one
image multi-select: flatten host-compatible images across distributions, sort by distribution
then image, deduplicate repeats, resolve each affected distribution's default kernel with no
operator override, and execute runtime packages first, then unique default kernels, then
selected images. Rename the entry to `microvm artifacts download` with repeatable
`--image DISTRIBUTION=IMAGE` for automation, removing `--distribution`/`--kernel`.

No SQLite migration. Persistence already keys per image
(`distribution_image:distro:image`) and `create_microvm` already takes
`distribution_id + image_id`, so creation needs no change.

## Technical Context

**Language/Version**: Rust edition 2024; repository toolchain is Rust 1.98.1. No new MSRV
declared by this feature.

**Primary Dependencies**: Existing `clap` 4.x (command surface, nested subcommands),
`inquire` 0.9.x (single image `MultiSelect`, confirmation), `indicatif` 0.18.x (truthful
terminal progress), `semver` 1.x (deterministic runtime-package ordering, CLI-internal),
`thiserror` 2.x (CLI error composition), `tokio` 1.x with the existing workspace features,
`tokio-util` 0.7.x (SDK-owned cooperative cancellation token), and the local
`taumaru-microvm` SDK. No new workspace dependencies. Do not add a direct `crossterm`,
HTTP, SQLite, hashing, or Firecracker dependency to the CLI.

**Storage**: No schema change. `MicroVmSdk` keeps owning the normalized inventory
(`distributions`, `distribution_images`, `distribution_kernels`, `downloads`) and its
migration lifecycle. Single-image acquisition reuses `persist_distribution_image` and
`resolve_distribution_image(distribution_id, image_id)`. The CLI holds only an ephemeral
in-memory plan and terminal result for one invocation.

**Testing**: `cargo test -p taumaru-microvm-cli --all-targets --all-features` for focused
CLI tests first, then `cargo test -p taumaru-microvm --all-targets --all-features` for the
new SDK operation, followed by the workspace gates: `cargo fmt --all -- --check`,
`cargo check --all-targets --all-features`,
`cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-targets --all-features`. SDK tests use the existing registry fixture
(`crates/sdk/tests/fixtures/manifest.json`) and deterministic download fakes; CLI tests use
the existing SDK-shaped fake client and never touch the network or a real terminal.

**Target Platform**: Linux host CLI with interactive TTY and non-TTY/script execution.
Output must remain understandable with `NO_COLOR`, limited terminal width, or no terminal.

**Project Type**: Rust workspace CLI application consuming a publishable Rust SDK library.

**Performance Goals**: Show discovery status immediately; show an initial transfer state
within one second after confirmation under normal terminal conditions; stream progress
without buffering artifacts; make no second transfer for SDK-valid cache hits; keep plan
construction linear in listed artifacts plus selected images.

**Constraints**: `main.rs` stays thin. All registry, cache, integrity, persistence, and
Firecracker behavior stays behind the SDK. No transfer starts before review confirmation.
Runtime-bundle failure stops dependent downloads. Independent image groups continue after a
kernel/image failure, but success is reported only when every required member returns a
verified SDK result. Expected failures return formatted nonzero CLI results without panic,
stack trace, or unsolicited SDK output. All repository artifacts and user-facing product
strings remain in English.

**Scale/Scope**: One invocation may select many images across many distributions with
shared default kernels; kernel transfers are deduplicated within the plan. Concurrent
multi-plan transfers, resumable transfers, authentication, custom registry selection, custom
kernel selection, and MicroVM lifecycle actions remain out of scope.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Pre-Phase 0 Gate

| Principle | Status | Design response |
|---|---|---|
| I. SDK-First Shared Core | PASS | New acquisition lives in `MicroVmSdk`; the CLI only selects, sequences, and renders SDK results. Existing whole-distribution methods are preserved for current consumers. |
| II. Panic-Free, Silent SDK Boundary | PASS | New SDK methods return typed `Result` errors (`NotFound`, `IncompatibleArtifact`, `Cancelled`, integrity/persistence errors) with Rustdoc; no printing, logging, or process control. |
| III. Explicit Local State and Lifecycle | PASS | CLI passes the explicit resolved home to the SDK and never opens SQLite; single-image persistence reuses the per-image inventory rows. |
| IV. Closed for Modification, Open for Extension | PASS | Additive SDK methods plus internal `distribution_image_member`/`download_member` reuse; CLI changes go through the existing internal client/renderer boundary. No second engine. |
| V. Calm, Accessible CLI and Intentional Documentation | PASS | One image list, sorted deterministic review, text-plus-symbol status, non-color/narrow modes, three-part failures, `130` on interrupt. |

No constitution violations require a complexity exception.

### Post-Phase 1 Gate

| Check | Status | Evidence |
|---|---|---|
| Dependency direction | PASS | `crates/cli` depends on `crates/sdk`; no SDK-to-CLI dependency. Public SDK addition is `DownloadedDistributionImage` plus two methods, re-exported through `lib.rs`. |
| SDK ownership | PASS | `commands/download.rs` calls list/download methods and renders results; no hash, path, inventory, or Firecracker logic in the CLI. |
| Home and database ownership | PASS | `context.rs` resolves `TAUMARU_HOME` or the default; `MicroVmSdk` owns directory creation and migrations. No schema change. |
| Plan safety | PASS | Immutable sorted plan plus explicit confirmation gate precede all SDK calls. Runtime failure is terminal for dependents; kernel failure skips only dependent images. |
| Async boundary | PASS | `main` runs the async CLI through Tokio; SDK calls are awaited sequentially in deterministic order; callbacks update the renderer. |
| Output accessibility | PASS | Text labels accompany semantic emphasis; non-TTY, `NO_COLOR`, and narrow-terminal modes stay line-oriented and deterministic. |
| Testability | PASS | Pure planning plus SDK-shaped fake client cover selection, dedup, ordering, and failure continuation; SDK fixture tests cover the new operation without the public registry. |

No post-design gate is violated.

## Project Structure

### Documentation (this feature)

```text
specs/005-image-artifact-download/
├── plan.md                         # This file ($speckit-plan command output)
├── research.md                     # Phase 0 decisions and alternatives
├── data-model.md                   # Ephemeral plan/result model and state transitions
├── quickstart.md                   # Operator and verification guide
├── contracts/
│   ├── sdk-image-download.md       # New SDK operation and result contract
│   └── cli-artifacts-download.md   # Renamed command, input, output, exit-status contract
└── tasks.md                        # Phase 2 output ($speckit-tasks command)
```

### Source Code (repository root)

```text
Cargo.toml                            # unchanged (no new dependencies)
Cargo.lock                            # unchanged unless tooling refreshes it
crates/sdk/
├── src/
│   ├── lib.rs                       # re-export DownloadedDistributionImage
│   ├── manager.rs                   # download_distribution_image + _with_cancellation
│   ├── domain/
│   │   ├── mod.rs                   # re-export new result type
│   │   └── artifact.rs              # DownloadedDistributionImage struct
│   └── tests/
│       ├── registry_contract.rs     # extend: single-image manifest reads (if needed)
│       ├── sqlite_persistence.rs    # extend: per-image persistence equivalence
│       └── failure_paths.rs         # extend: not-found, incompat, cancelled paths
crates/cli/
├── src/
│   ├── cli.rs                       # Artifacts parent + DownloadArgs{images} nesting
│   ├── commands/
│   │   ├── mod.rs                   # dispatch Command::Artifacts(Download)
│   │   └── download.rs              # image catalog, ImageSelection, plan, prompts, executor
│   └── output/
│       └── human.rs                 # per-image review, progress labels, summary
└── tests/
    └── command_surface.rs           # renamed entry, help, invalid-input behavior
```

**Structure Decision**: Keep the existing two-crate layout and module ownership. No new
crates, top-level directories, or speculative layers. SDK addition goes through
`domain/artifact.rs` (type) → `manager.rs` (operations) → `lib.rs`/`domain/mod.rs`
(re-exports). CLI changes stay inside `cli.rs`, `commands/download.rs`, and
`output/human.rs`.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| — | — | — |

No violations. Table retained as empty by template convention.
