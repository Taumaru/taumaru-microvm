# Research: MicroVM Creation Progress Callback

**Feature**: `006-create-progress-callback` | **Date**: 2026-09-19

All findings below are grounded in the current tree (`crates/sdk/src/manager.rs`,
`crates/sdk/src/domain/artifact.rs`, `crates/sdk/src/domain/microvm.rs`,
`crates/sdk/src/domain/lifecycle.rs`, `crates/sdk/tests/`).

## 1. Callback signature shape

- **Decision**: Change the signature to
  `pub async fn create_microvm<F>(&self, request: CreateMicroVmRequest, on_progress: Option<F>)
  -> Result<MicroVmCreationResult, SdkError> where F: FnMut(CreationProgress) + Send`.
  Helpers that need to emit take `Option<&mut F>`; `create_microvm` passes
  `on_progress.as_mut()`. When `None`, each emission site is one skipped branch.
- **Rationale**: This is exactly the convention the four download operations already use
  (`download_kernel`, `download_binary`, `download_distribution`,
  `download_distribution_image` all take `on_progress: F where F: FnMut(DownloadProgress) + Send`
  and invoke it synchronously, e.g. `on_progress(tracker.event(...))` inside
  `download_member` and the streaming loop). Reusing it keeps per-call routing for free
  (the generic parameter carries no shared state, so concurrent creations cannot
  interleave streams), adds zero cost when `None`, and satisfies FR-001's "callers that
  pass none" with an `Option` rather than a second method. The spec clarification
  explicitly authorizes changing the signature outright (SDK at `0.1.0`, never published).
- **Alternatives considered**:
  - Separate `create_microvm_with_progress` variant (download-style `_with_cancellation`
    pattern): rejected — it would permanently double the public creation surface for zero
    existing consumers, and every future creation flag would multiply the overloads.
  - Required (non-optional) callback: rejected — contradicts FR-001/FR-002 and forces
    no-op closures (`|_| {}`) on every caller for no benefit.
  - `&mut dyn FnMut(CreationProgress)` trait object: rejected — adds a lifetime and
    indirection while the generic form is already proven by the download path and keeps
    the `Send` bound uniform.

## 2. Event type shape (new, not reused)

- **Decision**: New `Copy` struct `CreationProgress { stage: CreationStage, completed_steps: u64,
  total_steps: u64 (= 6), bytes_completed: Option<u64>, expected_bytes: Option<u64>,
  outcome: Option<CreationOutcome> }`, with `CreationStage` (6 variants) and
  `CreationOutcome` (`Completed` / `AlreadyConfigured` / `Failed { stage }`) enums.
  Derives `Clone, Copy, Debug, Eq, PartialEq`; public `Display` plus `pub(crate) as_str`,
  mirroring `MicroVmState`/`NetworkMode` in `domain/lifecycle.rs`.
- **Rationale**: `DownloadProgress` is transfer-shaped (`artifact_kind`, `artifact_id`,
  `member_name`, `phase: DownloadPhase`); shoehorning six creation stages into
  `DownloadPhase::{Downloading, Verifying, ...}` would corrupt download semantics and any
  CLI rendering built on them. A dedicated type keeps both contracts truthful. `Copy` is
  possible (no `String` fields — stages are enums, counters are `u64`) and avoids
  allocation at all 7 emission points. `Option<u64>` byte fields encode FR-005's
  "step counters only" rule in the type system instead of sentinel `0/0` values that a
  CLI could mistake for a real total.
- **Alternatives considered**:
  - Reusing `DownloadProgress` with a synthetic `ArtifactKind`: rejected — leaks artifact
    concepts into lifecycle progress and breaks the download renderer's assumptions.
  - Separate `StageEvent` and `TerminalEvent` structs: rejected — one struct with
    `outcome: None` (stage) vs `Some` (terminal) is simpler to record, assert, and render,
    and matches the single-callback convention.

## 3. Emission ordering vs the existing-check branch

- **Decision**: The `Validation` stage event (1/6) is emitted only after both
  `request.validate(&home)` succeeds **and** the existing-VM lookup finds no record.
  The idempotent-repeat and conflict branches emit only their terminal event
  (`already-configured` / `failed` at `Validation`, 0/6) with no preceding stage events.
- **Rationale**: The current flow is validate → locks → existing-check → prerequisites.
  FR-006 requires the `already-configured` terminal to have "no preceding stage events",
  and clarification Q2 requires conflicts to produce "a single terminal `failed` event".
  Emitting `Validation` 1/6 immediately after `request.validate` would violate both, so
  the validation stage is defined as complete only once the operation is known to be a
  fresh creation. Any error before the first stage event (invalid request, lock failure,
  repository lookup failure, conflict, stale persisted state) is attributed to
  `Validation` at 0/6 — a single deterministic rule.
- **Alternatives considered**:
  - Emitting `Validation` 1/6 right after `request.validate` and letting conflict paths
    carry a stage event plus terminal: rejected — directly contradicts FR-006 and the Q2
    clarification.

## 4. Byte-counter source

- **Decision**: Only the `VolumePreparation` stage event sets byte counters, to
  `bytes_completed = expected_bytes = validated.request.disk_size_bytes` (the requested
  root-disk size). All other stage events and all terminal events set both to `None`.
- **Rationale**: FR-005 requires byte counters "consistent with verified artifact sizes".
  The requested size is already validated `>=` the registry-reported source `size_bytes`
  during prerequisite resolution, and `prepare_rootfs` grows the VM-local copy to exactly
  the requested size — so the finished stage's truthful final total is the requested
  size, known at the emission site without touching the storage port. No other stage
  moves a caller-meaningful byte total (key generation, network configuration, and
  persistence are discrete), so they report steps only. Terminals repeat step counts,
  never bytes, keeping one rule with no exceptions.
- **Alternatives considered**:
  - Reading the copied file's length after `prepare_rootfs`: rejected — adds a
    filesystem round-trip to report a value that is definitionally the requested size,
    and duplicates what the storage adapter already guarantees.
  - Byte ticks inside the copy (start/progress/finish): rejected — clarification Q3
    fixed one event per finished stage; the storage port has no progress hook and adding
    one would expand the port surface for negligible CLI benefit on a local copy.

## 5. Failure-terminal threading

- **Decision**: Emit the terminal `failed` event at the stage site that observes the
  error (via `match` + emit + `return Err`, replacing bare `?` at stage boundaries),
  threading `Option<&mut F>` into `create_claimed_microvm` for stages 3–6.
  `create_microvm` emits stage events 1–2, the `already-configured`/`failed` terminals
  for the existing-VM branch, and the `completed` terminal on success. The existing
  rollback wrapper around `create_claimed_microvm` is untouched and emits nothing itself
  (the failed terminal was already emitted by the failing stage).
- **Rationale**: Only the failing stage knows its own identity, so the emission must
  happen where the error surfaces — an error-to-stage remap at the top level would be
  fragile. Threading `&mut` (not moving `F`) lets both `create_microvm` and the helper
  emit on the same observer. Keeping rollback emission-free preserves its single
  responsibility and avoids double terminals. Invariant for implementers: every `Err`
  return after the observer is in scope is preceded by exactly one terminal emission;
  `?` may remain only where no emission is owed (pre-validation code has no observer
  yet — but those errors still need the `Validation` 0/6 terminal, so they also use
  match-emit-return).
- **Alternatives considered**:
  - A `current_stage` mutable variable mapped at a single top-level `map_err`:
    rejected — implicit, easy to desynchronize from the actual flow, harder to review.
  - Emitting `failed` inside `rollback_creation`: rejected — rollback also runs on paths
    whose terminal was already emitted, and rollback failures (`SdkError::Cleanup`) must
    not produce a second terminal.

## 6. Observer call discipline

- **Decision**: Synchronous invocation in the caller's task at each emission point
  (same as downloads); observer must be infallible (`FnMut(CreationProgress)` returning
  `()`) and non-blocking. A panicking observer unwinds as caller fault — identical to
  the download callbacks — and is documented as caller-owned in Rustdoc.
- **Rationale**: Matches the proven download discipline (no spawn, no channel, no
  buffering, no timer), keeps the silent-SDK boundary (FR-007), and makes event order
  deterministic for tests: a recording closure observes exactly the emission order.
  `Send` (not `Sync`) is sufficient since the observer is only ever called from the
  operation's own task.
- **Alternatives considered**:
  - `tokio::sync::mpsc` channel or spawned progress task: rejected — heavier, async,
    ordering-sensitive, and a new pattern no other SDK operation uses.

## 7. Test strategy

- **Decision**: Primary coverage in deterministic manager unit tests using the existing
  injected fakes (`TestStorage`, `TestCredentials`, `TestNetwork`, `TestRuntime`,
  `TestArtifactSource` with the fixture manifest in `crates/sdk/src/manager.rs` tests
  module): full 7-event stream with exact counters/stages/order, byte rule, failure
  terminals per stage, idempotent and conflict terminals, observer/no-observer result
  equivalence, and per-call isolation. Extend `crates/sdk/tests/public_api.rs` export
  assertions for the three new types; add `crates/sdk/tests/creation_progress.rs`
  integration test proving the silent/typed-error surface holds with an observer
  attached. Migrate all in-repo `create_microvm` call sites (~9 in the manager tests
  module, 1 in `public_api.rs`) to the new signature in the same change — verified by
  grep that no CLI or other crate calls it. Capability-gated Linux integration
  (KVM/TAP/DHCP) stays out of the default suite, as today.
- **Rationale**: The recording-closure pattern is already established in
  `crates/sdk/tests/download_flow.rs` (`|event| events.push(event)` + phase/order
  assertions), so the same assertions apply to creation streams. Unit tests through
  injected ports give exact stage control without host privileges; the integration test
  guards the public contract (exports, silence, typed errors).
- **Alternatives considered**:
  - Testing only through the integration suite: rejected — host-dependent and unable to
    inject per-stage failures deterministically.
