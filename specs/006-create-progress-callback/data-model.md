# Data Model: MicroVM Creation Progress Callback (realtime)

**Feature**: `006-create-progress-callback` | **Date**: 2026-09-19 (realtime revision)

No storage changes. No SQLite migration. Events are transient and never persisted.
All types are SDK-public, `Clone + Copy + Debug + Eq + PartialEq`, documented
with Rustdoc, and re-exported from `crates/sdk/src/domain/mod.rs` and
`crates/sdk/src/lib.rs`.

## Entities

### `CreationStage` (enum, `domain/microvm.rs`)

One named discrete phase of creation, in fixed emission order (unchanged):

`Validation`, `PrerequisiteResolution`, `VolumePreparation`, `CredentialSetup`,
`NetworkConfiguration`, `Finalization` — with `Display` snake-case labels,
`pub(crate) as_str` + `parse`, mirroring `MicroVmState`/`NetworkMode`.

### `CreationEventPhase` (new enum, `domain/microvm.rs`)

What happened inside a stage for one event: `Started` (stage began),
`InProgress` (byte work advanced), `Finished` (stage completed; also used by
all terminal events).

### `CreationOutcome` (enum, `domain/microvm.rs`)

The single closing state of an observed operation (unchanged):
`Completed` (6/6, 100%), `AlreadyConfigured` (0/6, preceded only by a `Started`
event), `Failed { stage }` (N/6 for the N finished stages).

### `CreationProgress` (struct, `domain/microvm.rs`)

```rust
pub struct CreationProgress {
    pub stage: CreationStage,
    pub completed_steps: u64,
    pub total_steps: u64,        // always 6 (TOTAL_CREATION_STEPS)
    pub overall_percent: u64,    // 0-100, monotonic; drives a progress bar alone
    pub phase: CreationEventPhase,
    pub bytes_completed: Option<u64>,
    pub expected_bytes: Option<u64>,
    pub outcome: Option<CreationOutcome>,  // None = stage event, Some = terminal event
}
```

Field rules:

- `total_steps` is always `TOTAL_CREATION_STEPS` (6).
- `overall_percent` = completed stages' full share + fractional byte progress of
  the current stage's share; first event reports 0, terminal `completed` reports 100.
- `InProgress` ticks carry `Some`/`Some` byte fields; `Started`/`Finished` events
  (including all terminals) carry `None`/`None`. Both fields are `Some` or both
  are `None` — never mixed.
- `Copy`: no heap fields; zero allocation per emission.

### `Progress Observer` (call-site concept, no new type)

`Option<F> where F: FnMut(CreationProgress) + Send`, passed to `create_microvm`
(unchanged). Invoked synchronously in the caller's task, in real time, at stage
starts, byte ticks, stage finishes, and the terminal event.

## Invariants (all testable)

1. `overall_percent` and step counters are monotonic within one operation and never
   exceed 100 / 6.
2. Every stage emits exactly one `Started` and one `Finished` event in stage order;
   byte-moving work (artifact verification reads, rootfs copy) emits `InProgress`
   ticks between its stage's start and finish.
3. Byte fields are `Some` only on `InProgress` ticks, with `done <= total`.
4. Every observed operation ends with exactly one terminal event (`Finished` phase).
5. Idempotent repeat: `Started` + single `already-configured` terminal, 0/6, no
   finishes, no host changes. Conflict: `Started` + single `failed` terminal at
   `Validation`, 0/6, existing typed error unchanged.
6. Nothing about the stream is persisted; returned VM metadata with an observer is
   field-identical to the same request without one.

## State transitions

Unchanged: the event stream is orthogonal to `MicroVmState`
(`Creating → Configured`); rollback on failure is unchanged and emits nothing itself.

## Emission map (code locations)

1. `Validation` start 0/0% — `create_microvm` entry; finish 1/6 — after the
   existing-VM lookup returns `None`.
2. `PrerequisiteResolution` start 1/6 — before `resolve_creation_prerequisites`;
   `InProgress` ticks during image/kernel verification reads; finish 2/6 — after
   `insert_creating` succeeds. Failure → terminal `failed` at 1/6.
3. `VolumePreparation` start 2/6 — before `prepare_rootfs`; `InProgress` ticks
   from the storage copy loop (`prepare_rootfs` `on_copy_progress` callback);
   finish 3/6 — after `prepare_rootfs` returns. Failure → terminal `failed` at 2/6.
4. `CredentialSetup` start 3/6 → finish 4/6 around key generation + injection.
5. `NetworkConfiguration` start 4/6 → finish 5/6 around port configuration +
   guest writes.
6. `Finalization` start 5/6 → finish 6/6 after `update_state(Configured)`; then
   terminal `completed` 6/6 at 100%.
7. Terminals — `failed` N/6 at each stage's error site; `already-configured` 0/6
   on the idempotent branch (preceded only by the validation `Started` event).
