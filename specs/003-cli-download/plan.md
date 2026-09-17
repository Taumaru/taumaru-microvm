# Implementation Plan: CLI Artifact Download

**Branch**: `003-cli-download` | **Date**: 2026-09-17 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-cli-download/spec.md`

## Summary

Add a `microvm download` command as a thin CLI workflow over the existing `MicroVmSdk`. The
command will load the current distributions, kernels, and binary packages; select one or more
host-compatible distributions and one compatible kernel per distribution; choose the highest
host-compatible package for each required runtime component (`firecracker` and `firectl`), reusing
one package when it provides both; show a review; and begin transfers only after confirmation.

Interactive selection uses `inquire`, while `indicatif` renders truthful aggregate and
current-member progress from SDK callbacks. Explicit repeatable `--distribution` and
`--kernel distribution=kernel` options provide deterministic non-interactive use. The CLI will
execute the SDK operations in runtime-binary, unique-kernel, and distribution-image order, render
the SDK's downloaded/adopted/skipped dispositions, and return a nonzero result for incomplete
plans.

No SQLite schema or cache policy is added to the CLI. The CLI resolves the existing home policy,
passes the path to `MicroVmSdk::new`, and leaves registry validation, checksum and size
verification, invalid-file replacement, inventory persistence, and binary mapping to the SDK.

## Technical Context

**Language/Version**: Rust edition 2024; repository toolchain is Rust 1.98.1. No new MSRV is
declared by this feature.

**Primary Dependencies**: Existing `clap` 4.x for the command surface and the local
`taumaru-microvm` SDK. Add `inquire` 0.9.x for keyboard-friendly selectors and confirmation,
`indicatif` 0.18.x for terminal progress, `semver` 1.x for deterministic runtime package
version ordering, `thiserror` 2.x for CLI error composition, and `tokio` 1.x with the existing
workspace dependency plus `macros`, `rt-multi-thread`, and `signal` features for the asynchronous
command entrypoint. Add `tokio-util` 0.7.x for the SDK-owned cooperative cancellation token. The
SDK will expose cancellation-aware download variants while preserving its existing methods. Do
not add a direct `crossterm`, HTTP, SQLite, hashing, or Firecracker dependency to the CLI.

**Storage**: No new CLI storage. `MicroVmSdk::new` receives the resolved home, creates or
validates the SDK-managed layout, runs existing pending SQLite migrations, and owns all artifact
inventory updates. The CLI holds only an in-memory plan and terminal result for one invocation.

**Testing**: `cargo test -p taumaru-microvm-cli --all-targets --all-features` for focused CLI
tests, followed by the workspace `fmt`, `check`, `clippy`, and `test` gates. Unit tests will cover
catalog filtering, explicit mapping validation, runtime-package selection, plan deduplication and
ordering, confirmation gating, progress normalization, non-color/narrow output, and failure
continuation. A fake SDK-shaped client will avoid network and terminal requirements; existing SDK
tests remain responsible for actual registry, cache, integrity, and SQLite behavior.

**Target Platform**: Linux host CLI with interactive TTY and non-TTY/script execution. The
command must also remain understandable with `NO_COLOR`, limited terminal width, or no terminal.

**Project Type**: Rust workspace CLI application consuming a publishable Rust SDK library.

**Performance Goals**: Start visible discovery feedback immediately; show an initial transfer
state within one second after confirmation under normal terminal conditions; stream progress
without buffering artifacts; make no second transfer for SDK-valid cache hits; and keep plan
construction linear in the number of listed artifacts plus selected images.

**Constraints**: `main.rs` remains thin; all registry, cache, integrity, persistence, and
Firecracker behavior stays behind the SDK. No transfer may start before review confirmation.
Runtime-bundle failure stops dependent downloads. Independent groups may continue after failure,
but success is reported only when every required artifact group returns verified SDK results.
Expected failures return formatted nonzero CLI results without panic, stack trace, or unsolicited
SDK output. All repository artifacts and user-facing product strings remain in English.

**Scale/Scope**: One command invocation may select multiple distributions, many images, and
shared kernels, while downloading one or more runtime binary packages. Kernel transfers are
deduplicated within the plan. Concurrent multi-plan transfers, resumable transfers, authentication,
custom registry selection, and MicroVM lifecycle actions remain out of scope.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Pre-Phase 0 Gate

| Principle | Status | Design response |
|---|---|---|
| I. SDK-First Shared Core | PASS | The CLI only selects, presents, and sequences calls to the existing SDK; it does not implement another artifact or lifecycle engine. |
| II. Panic-Free, Silent SDK Boundary | PASS | SDK failures remain typed and silent. CLI formatting owns diagnostics and does not ask the SDK to print, log, or terminate. |
| III. Explicit Local State and Lifecycle | PASS | The CLI passes the explicit resolved home to the SDK and never opens SQLite or maintains a second artifact inventory. |
| IV. Closed for Modification, Open for Extension | PASS | A small internal client and renderer boundary allow deterministic tests and future output/registry adapters without changing SDK semantics. |
| V. Calm, Accessible CLI and Intentional Documentation | PASS | The planned selectors, review, progress, labels, non-color mode, and actionable failures follow the Design System and constitution. |

No constitution violations require a complexity exception.

### Post-Phase 1 Gate

| Check | Status | Evidence |
|---|---|---|
| Dependency direction | PASS | `crates/cli` depends on `crates/sdk`; no SDK-to-CLI dependency is introduced. |
| SDK ownership | PASS | `commands/download.rs` calls list/download methods and renders results; it does not calculate hashes, inspect paths, mutate inventory, or invoke Firecracker. |
| Home and database ownership | PASS | `context.rs` resolves `TAUMARU_HOME` or the existing default; `MicroVmSdk::new` owns directory creation and migration execution. |
| Plan safety | PASS | A complete immutable plan and explicit confirmation gate precede all SDK download calls. Runtime failure is terminal for dependent stages. |
| Async boundary | PASS | `main` runs the async CLI through Tokio; SDK calls are awaited sequentially in deterministic order and callbacks update the renderer. |
| Output accessibility | PASS | Text labels accompany semantic emphasis; non-TTY, `NO_COLOR`, and narrow-terminal modes remain line-oriented and deterministic. |
| Testability | PASS | Pure planning and an SDK-shaped fake client cover command semantics without the public registry or a real terminal. |

No post-design gate is violated.

## Project Structure

### Documentation (this feature)

```text
specs/003-cli-download/
├── plan.md                         # This file ($speckit-plan command output)
├── research.md                     # Phase 0 decisions and alternatives
├── data-model.md                   # Ephemeral plan/result model and state transitions
├── quickstart.md                   # Operator and verification guide
├── contracts/
│   └── cli-download.md              # Command, input, output, and exit-status contract
└── tasks.md                        # Phase 2 output ($speckit-tasks command)
```

### Source Code (repository root)

```text
Cargo.toml                           # add shared CLI dependency versions
Cargo.lock                           # resolve the CLI dependency graph
crates/sdk/
├── Cargo.toml                        # add the cancellation support dependency
└── src/
    ├── error.rs                      # typed cancellation error
    ├── manager.rs                    # cancellation-aware download variants and cleanup
    └── domain/
        └── artifact.rs               # cancellation control and progress phase
crates/cli/
├── Cargo.toml                        # inquire, indicatif, semver, thiserror, tokio
├── src/
│   ├── main.rs                       # startup, async run, help handling, exit mapping
│   ├── cli.rs                        # Clap root, subcommands, and download options
│   ├── context.rs                    # TAUMARU_HOME/default resolution and SDK wiring
│   ├── error.rs                      # typed CLI errors and what/why/next formatting
│   ├── commands/
│   │   ├── mod.rs
│   │   └── download.rs               # catalog, selectors, plan, execution, failure policy
│   └── output/
│       ├── mod.rs
│       └── human.rs                  # review, progress, summary, and terminal capability
└── tests/
    ├── command_surface.rs             # help, version, options, and non-TTY guard behavior
    └── package_boundary.rs             # existing publishable-SDK dependency check
```

No SQLite schema or migration file is changed by this feature. A focused SDK change in
`manager.rs`, `domain/artifact.rs`, and `error.rs` adds cooperative cancellation and temporary-file
cleanup while preserving the existing artifact ownership rules. Existing SDK adapters and tests
remain the only owners of registry transport, artifact file layout, checksum/size verification,
cache reconciliation, and SQLite persistence.

**Structure Decision**: Extend the existing minimal CLI into the constitution's ownership-oriented
layout. `cli.rs` defines only input syntax; `context.rs` owns caller-side home resolution and
dependency wiring; `commands/download.rs` owns the cohesive download use case and pure plan
validation; `output/human.rs` owns interactive rendering and progress presentation. The command
module uses an internal SDK-shaped client boundary for tests, but production behavior is always
delegated to `MicroVmSdk`.

## Design Decisions

### Command lifecycle

`main.rs` will retain top-level help behavior and become an async entrypoint that maps a returned
`CliError` to `ExitCode`: `0` for a fully verified plan, `1` for validation/registry/filesystem/
partial-download failure, and `130` for interactive or transfer cancellation. It will not use
`unwrap`, `expect`, or process termination inside command logic.

`commands/download.rs` will expose a narrow flow with explicit stages:

1. resolve context and detect prompt/output capabilities;
2. load the three SDK lists into one catalog;
3. choose explicit selections or run the interactive selectors;
4. validate compatibility and choose the runtime packages;
5. render review and require confirmation;
6. execute runtime, unique kernels, and distributions in plan order;
7. render verified, retained, failed, and final outcome states.

The plan builder is pure over catalog data and selection input. The executor receives the plan,
an SDK-shaped client, and a renderer. This keeps user interaction from being entangled with
registry metadata decisions and makes the no-transfer-before-confirmation invariant directly
testable.

### Registry and compatibility selection

The catalog loader calls `list_distributions`, `list_kernels`, and `list_binaries` once before
selection. Any SDK error stops the command before a prompt. Host architecture is mapped from
`std::env::consts::ARCH`; unsupported host values are a typed CLI validation failure.

The distribution selector shows only distributions matching the host architecture. For each one,
the kernel options are the intersection of its `supported_kernels` IDs, the listed kernels, and
the host architecture. The distribution's `default_kernel` is marked when it is available in that
intersection. Explicit mappings run the same validation and cannot bypass compatibility rules.

Runtime selection filters packages to the host architecture and selects the highest valid semantic
version for each required file component, `firecracker` and `firectl`. A package containing both
components is selected once and reused; split registry packages are both selected. Equal-version
ties are broken by package ID. Missing or invalid candidates produce a pre-transfer failure and
never fall back to an arbitrary package.

### Plan size, deduplication, and order

The plan sums runtime file sizes, one size per unique kernel ID, and every image size in selected
distributions with checked `u64` arithmetic. Selected distributions are sorted by registry ID for
execution; unique kernels are sorted by registry ID. Files within a binary package or distribution
remain in the order controlled by the SDK/registry operation.

The plan includes one distribution relationship for every selected distribution and one kernel
relationship for each distribution, even when multiple relationships point to the same kernel.
Only physical kernel execution members are deduplicated.

### SDK delegation and result handling

The production client adapter invokes only the existing SDK APIs. The callback maps each
`DownloadProgress` into the renderer with the operation's completed-byte base and the plan total.
The returned `DownloadedBinary`, `DownloadedKernel`, and `DownloadedDistribution` values provide
the terminal verified members and `DownloadDisposition` values.

The CLI does not inspect the filesystem before invoking an SDK operation. It does not delete a
corrupt file, calculate a digest, update a table, or call `resolve_binary` to infer success. The
SDK's `SkippedExisting` and `AdoptedExisting` events are displayed distinctly, and an SDK error
after earlier verified members is reported as retained work.

Every selected runtime package is executed first and the runtime stage is the only hard prerequisite
for the rest of the plan. After all runtime packages succeed, each unique kernel and selected
distribution group is attempted in order even if an earlier independent group fails. If the operator
cancels during any operation, the executor
stops the current and subsequent groups, asks the SDK's cancellation-safe path to remove the
partial temporary file, and reports retained verified work. The command returns a
partial-failure outcome and exit code `1` if any required group fails, or cancellation and exit
code `130` when interrupted.

### Interaction and output

`inquire::MultiSelect` provides distribution selection, `inquire::Select` provides one kernel
choice per selected distribution, and `inquire::Confirm` guards the transfer. Prompt item text
will stay compact and text-readable, with explicit selection/focus/default markers and no
decorative saturated colors.

`indicatif` renders one aggregate bar plus a current-member line on an interactive terminal.
Non-TTY and `NO_COLOR` modes use deterministic plain lines and retain stage, byte, identity, and
disposition labels. Progress goes to stderr; the final summary goes to stdout. Output helpers will
avoid wide tables and preserve stable IDs when terminal width is limited.

### Error presentation

`error.rs` will wrap SDK, prompt, input, home-resolution, output, plan-validation, and controlled
cancellation failures in a typed CLI error. The formatter will provide what happened, why the
plan is incomplete or cancelled, and the next corrective action. Registry and SDK error text is
used as context, not printed as a raw backtrace. Prompt and transfer cancellation remain distinct
so they can map to exit code `130` while preserving the final group summary. The SDK's typed
cancellation result is not rendered as an unexpected download failure.

## Implementation Sequence

1. Add the CLI dependency declarations in the workspace and `crates/cli/Cargo.toml`; keep SDK
   dependencies unchanged.
2. Split the current Clap bootstrap into `cli.rs`, `context.rs`, `error.rs`, `commands/mod.rs`,
   and `output/mod.rs` while preserving existing help, version, no-argument, and invalid-input
   behavior.
3. Add the `download` subcommand and explicit repeatable distribution/kernel options, including
   non-interactive validation and stable exit-code mapping.
4. Implement home resolution and production SDK wiring without moving environment lookup into the
   SDK.
5. Extend the SDK download path with an explicit cancellation signal and cancellation-safe
   temporary-file cleanup, preserving existing uncancelled download APIs and SQLite semantics.
6. Implement catalog loading, host compatibility filtering, explicit mapping validation, semantic
   runtime-package selection, size accounting, shared-kernel deduplication, and deterministic plan
   construction.
7. Implement interactive distribution/kernel selection, review, confirmation, cancellation, and
   narrow/non-color-safe human output according to the Design System reference.
8. Implement sequential execution through the SDK-shaped client, callback-to-progress mapping,
   runtime prerequisite handling, independent failure retention, and final summaries.
9. Add unit tests close to planning, execution, context, error, and output modules, including fake
   client tests for no transfer before confirmation, ordering, deduplication, cache dispositions,
   partial failures, runtime failure short-circuiting, controlled cancellation, temporary-file
   cleanup, and truthful progress.
10. Extend CLI command-surface tests for the new help/options and non-TTY selection guard; keep
   network and SQLite behavior covered by the existing SDK fixture and persistence suites.
11. Run focused and full workspace quality gates, then verify the operator and automation flows in
    [quickstart.md](./quickstart.md).

## Complexity Tracking

No constitution violations are expected. The internal SDK-shaped client boundary and the separate
human output module are required to keep the command testable and the CLI/SDK ownership boundary
explicit; neither introduces a second artifact or persistence implementation.
