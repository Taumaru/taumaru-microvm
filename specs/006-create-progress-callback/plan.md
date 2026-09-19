# Implementation Plan: MicroVM Creation Progress Callback

**Branch**: `006-create-progress-callback` | **Date**: 2026-09-19 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/006-create-progress-callback/spec.md`

## Summary

Change `MicroVmSdk::create_microvm` to accept an optional per-call progress observer, following the existing download-callback convention (`FnMut(...) + Send`, invoked synchronously in the caller's task). With an observer attached, creation emits exactly one event per finished stage in fixed order — request validation, prerequisite resolution, volume/rootfs preparation, credential setup, network configuration, finalization — each carrying N/6 step counters, plus exactly one terminal event (`completed` 6/6, `already-configured` 0/6, or `failed` N/6 naming the failed stage). Only the volume/rootfs stage event carries byte counters (final `disk_size_bytes` for both fields); all other stage events carry step counters only. Without an observer, results, typed errors, idempotency, conflicts, and rollback are unchanged. The previous one-argument signature is changed outright (no backward compatibility; SDK unpublished, version 0.1.0). No CLI, persistence, registry, or Firecracker-adapter changes.

## Technical Context

**Language/Version**: Rust 2024 edition, repository stable toolchain (unchanged).

**Primary Dependencies**: No new dependencies. Reuses `tokio` async and the established `FnMut(Event) + Send` callback convention from `download_kernel` / `download_binary` / `download_distribution` / `download_distribution_image` in `crates/sdk/src/manager.rs`.

**Storage**: Unchanged. No SQLite migration; progress events are transient and never persisted. VM, network, credential, and runtime rows keep their current shape.

**Testing**: `cargo test` with the existing deterministic manager-test harness (injected `TestStorage` / `TestCredentials` / `TestNetwork` / `TestRuntime` / `TestArtifactSource` in `crates/sdk/src/manager.rs` tests module) for stream-shape assertions, plus `crates/sdk/tests/public_api.rs` export assertions and a new `crates/sdk/tests/creation_progress.rs` integration test using the fixture registry server. Capability-gated Linux integration (KVM, TAP/bridge, DHCP) stays out of the default suite, as today.

**Target Platform**: Linux hosts, same as existing creation (unchanged host prerequisites).

**Project Type**: SDK library feature in `crates/sdk`; the CLI is not modified (no `create` command exists yet; verified `crates/cli/src/commands/` contains only `download.rs`).

**Performance Goals**: Observer overhead is one branch per emission point when `None` (7 branches on success) and one synchronous callback invocation per event when `Some`; no allocation, buffering, thread spawn, or timer is introduced. Creation duration is otherwise unchanged.

**Constraints**: Silent SDK (no stdout/stderr, logs, tracing, process exit, global state); panic-free typed `Result` errors; no environment-variable reads; observer is caller-owned, infallible (`FnMut` returning `()`), non-blocking, invoked synchronously in the caller's task, and cannot alter integrity, persistence, or error decisions; per-call routing under concurrency falls out of the generic parameter (no shared state); breaking signature change is permitted (SDK unpublished, `0.1.0`, no crates.io consumers) with all in-repo call sites migrated in the same change.

**Scale/Scope**: Multiple independent VMs per host, unchanged. Per-call observer routing supports concurrent creations with no cross-operation aggregation. Out of scope: creation cancellation, `configure_network` progress, CLI rendering.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle behavior; CLI remains thin | PASS: SDK-only change, no CLI source touched | PASS: design adds no CLI code and no second orchestration path |
| Public SDK is typed, silent, non-panicking, side-effect explicit | PASS: infallible `FnMut` observer, typed errors unchanged, no output/logging/global state | PASS: contract bans all progress side channels; observer cannot change decisions; events transient |
| Domain is independent from infrastructure | PASS: new progress types live in `domain/microvm.rs` with no fs/db/net/process imports | PASS: emission points in `manager.rs` only read already-computed values; ports/adapters untouched |
| SQLite is local source of truth | PASS: no persistence change | PASS: no migration; events never stored; rollback and idempotency semantics preserved |
| Firecracker/firectl remain replaceable details | PASS: runtime port untouched | PASS: no command/process type leaks into progress events |
| Registry and artifact boundaries explicit | PASS: prerequisites resolution unchanged | PASS: byte counters reuse already-verified sizes; creation still never downloads |
| Multiple MicroVMs supported | PASS: per-call observer, per-VM locks unchanged | PASS: no shared progress state; concurrent streams cannot interleave |
| Public contracts and compatibility documented | PASS (with note): breaking `create_microvm` signature change is permitted because the SDK is unpublished (`0.1.0`, never published to crates.io); all in-repo call sites migrate in the same change; Rustdoc + contract + quickstart updated | PASS: `contracts/sdk-create-progress.md` records the breaking change explicitly with before/after signatures and the no-migration rationale |
| Project text is English | PASS | PASS |

Breaking-change note (Complexity Tracking justification, not a violation): constitution principle IV requires explicit versioning/migration guidance for breaking public changes. The SDK crate is at `0.1.0` and has never been published to crates.io, so there are no external consumers to migrate; the "migration guidance" is the contract's before/after signature record plus the mechanical in-repo call-site update (`None::<fn(CreationProgress)>` or a recording closure). No version bump beyond the existing `0.1.0` pre-publish state is required.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Signature: `pub async fn create_microvm<F>(&self, request: CreateMicroVmRequest, on_progress: Option<F>) where F: FnMut(CreationProgress) + Send` — generic `Option<F>` mirrors the download `FnMut + Send` convention, keeps per-call routing, and costs one branch per emission point when `None`.
2. Event shape: single `Copy` struct `CreationProgress { stage, completed_steps, total_steps (=6 const), bytes_completed: Option<u64>, expected_bytes: Option<u64>, outcome: Option<CreationOutcome> }`; `CreationStage` (6 variants) and `CreationOutcome` (`Completed` / `AlreadyConfigured` / `Failed { stage }`) enums with `as_str`/`Display` for CLI labels.
3. Emission map: 7 emission points on success (6 stage finishes + terminal), terminal-only on early paths (validation failure, conflict, idempotent repeat), N stage events + terminal-`failed` on mid-flow failure; rollback and typed errors untouched.
4. Byte counters: only the volume/rootfs stage event sets them, to final `disk_size_bytes` for both fields (requested size, already validated `>=` verified source size); every other event sets both to `None`.
5. Observer discipline: synchronous invocation in the caller's task at each emission point via a small `emit` helper taking `Option<&mut F>`; no spawn, no buffering, no logging; a panicking observer unwinds as caller fault (same as downloads) and is documented as caller-owned.
6. Tests: deterministic manager unit tests (recording closure over injected fakes) for the full 7-event stream, counters, byte rule, failure terminals, idempotent/conflict paths, and observer/result equivalence; `public_api.rs` export assertions; new `creation_progress.rs` integration test for the silent/typed-error surface with an observer.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): `CreationStage`, `CreationOutcome`, `CreationProgress` fields, step/byte invariants, terminal rules, and the stage-to-code-location emission map.
- [contracts/sdk-create-progress.md](./contracts/sdk-create-progress.md): new public types, changed `create_microvm` signature (before/after), required emission behavior, preserved behavior, error contract (unchanged), usage examples.
- [quickstart.md](./quickstart.md): observer usage, stream-shape assertions, no-observer equivalence, failure/idempotent checks, and gates.

### Public SDK boundary

Change in `crates/sdk/src/domain/microvm.rs` (new types) and `crates/sdk/src/manager.rs` (`create_microvm` signature + emission points); re-export the three new types from `crates/sdk/src/domain/mod.rs` and `crates/sdk/src/lib.rs` with Rustdoc:

- `CreationStage`, `CreationOutcome`, `CreationProgress` (new, `Clone, Debug, Eq, PartialEq`, plus `Copy`);
- `create_microvm(CreateMicroVmRequest, Option<F>) -> Result<MicroVmCreationResult, SdkError> where F: FnMut(CreationProgress) + Send` (changed; old one-argument form removed);
- `configure_network(&str)` untouched; all other public operations untouched.

Migrate every in-repo caller in the same change: the `manager.rs` tests-module `create_microvm(...)` sites (~9) and `crates/sdk/tests/public_api.rs` (1 site), plus any contract/quickstart snippets that name the old signature (docs only; `specs/003-create-microvm/*` are historical records and are not rewritten — the new contract references them).

### Emission points (normative order)

| # | Location in current `manager.rs` flow | Event on success | Failure terminal |
|---|---|---|---|
| 1 | `create_microvm`, after `request.validate(&home)` | `Validation` 1/6, no bytes | `failed`, stage `Validation`, 0/6, then existing typed error |
| 2 | `create_microvm`, existing-VM branch (`return_or_reject_existing_creation`) | idempotent: terminal `already-configured`, stage `Validation`, 0/6, no stage events | conflict/non-configured/invalid persisted state: terminal `failed`, stage `Validation`, 0/6, then existing typed error |
| 3 | `create_microvm`, after `resolve_creation_prerequisites` + `runtime.validate_host` + `ensure_volume_available` + `insert_creating` | `PrerequisiteResolution` 2/6, no bytes | `failed` naming `PrerequisiteResolution`, 1/6 |
| 4 | `create_claimed_microvm`, after `storage.prepare_rootfs` | `VolumePreparation` 3/6, bytes `disk_size_bytes`/`disk_size_bytes` | `failed` naming `VolumePreparation`, 2/6 (rollback unchanged) |
| 5 | `create_claimed_microvm`, after `credentials.generate` + `storage.inject_public_key` | `CredentialSetup` 4/6, no bytes | `failed` naming `CredentialSetup`, 3/6 (rollback unchanged) |
| 6 | `create_claimed_microvm`, after `network.configure` + guest config writes | `NetworkConfiguration` 5/6, no bytes | `failed` naming `NetworkConfiguration`, 4/6 (rollback unchanged) |
| 7 | `create_claimed_microvm`, after `persist_network`/`persist_credential`/`persist_runtime` + `verify_stopped` + `update_state` | `Finalization` 6/6, no bytes, then terminal `completed` 6/6 | `failed` naming `Finalization`, 5/6 (rollback unchanged) |

`S1` failure (invalid request, conflict, idempotent repeat) emits only its terminal event. `S2+` failures emit the finished stages' events already produced, then exactly one terminal `failed` — never an extra stage event for the failure itself.

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── src/
│   │   ├── lib.rs              # re-export CreationStage, CreationOutcome, CreationProgress
│   │   ├── manager.rs          # changed create_microvm signature + 7 emission points
│   │   └── domain/
│   │       ├── mod.rs          # re-export new types
│   │       └── microvm.rs      # CreationStage, CreationOutcome, CreationProgress + TOTAL_STEPS
│   └── tests/
│       ├── public_api.rs       # extended export assertions + None-form call-site update
│       └── creation_progress.rs # new: stream shape, equivalence, failure/idempotent terminals
└── cli/
    └── src/                    # unchanged (no create command exists yet)
```

**Structure Decision**: SDK-only change inside the existing ownership layout. New domain types co-locate with the existing creation types in `domain/microvm.rs` (no new module — first real capability does not need a new layer). Emission logic stays inline in `manager.rs` behind one small private `emit` helper; no new ports or adapters (the observer is a caller-owned sink, not infrastructure). No SQLite migration. CLI untouched.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Breaking `create_microvm` signature change without deprecation shim (principle IV prefers additive + migration guidance) | Spec clarification Q1 explicitly authorizes changing the signature outright since the SDK is unpublished (`0.1.0`, no crates.io consumers); a shim (`create_microvm` + `create_microvm_with_progress`) would leave two creation entry points and duplicate the emission plumbing | Keeping the old signature plus a variant doubles the public creation surface permanently for zero consumers; an `impl FnMut` required-callback overload without `Option` would not satisfy FR-001 ("callers that pass none") |
