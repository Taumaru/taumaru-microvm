# Data Model: MicroVM Creation Progress Callback

**Feature**: `006-create-progress-callback` | **Date**: 2026-09-19

No storage changes. No SQLite migration. Events are transient and never persisted.
All types are SDK-public, `Clone + Debug + Eq + PartialEq` (plus `Copy`), documented
with Rustdoc, and re-exported from `crates/sdk/src/domain/mod.rs` and
`crates/sdk/src/lib.rs`.

## Entities

### `CreationStage` (new enum, `domain/microvm.rs`)

One named discrete phase of creation, in fixed emission order:

| Variant | Meaning | Emitted when |
|---|---|---|
| `Validation` | Request validation plus the existing-VM lookup decision | Fresh creation confirmed (1/6) |
| `PrerequisiteResolution` | Artifact/kernel/runtime resolution and volume availability checks | Prerequisites resolved and provisional row inserted (2/6) |
| `VolumePreparation` | VM-local `rootfs.ext4` copy (and growth) | Copy finished (3/6) |
| `CredentialSetup` | Ed25519 generation plus public-key injection | Key injected (4/6) |
| `NetworkConfiguration` | Network port configuration plus guest config writes | Network outcome persisted to journal (5/6) |
| `Finalization` | Runtime verification plus network/credential/runtime persistence and state commit | VM committed `Configured` (6/6) |

Public `Display` (lowercase snake strings: `validation`, `prerequisite_resolution`,
`volume_preparation`, `credential_setup`, `network_configuration`, `finalization`) for
CLI labels; `pub(crate) as_str` + `parse`, mirroring `MicroVmState`/`NetworkMode`.

Validation rules: exactly six variants; ordering above is normative; no `#[non_exhaustive]`
(the stage set is fixed by the spec's six-stage total).

### `CreationOutcome` (new enum, `domain/microvm.rs`)

The single closing state of an observed operation:

| Variant | Meaning | Step counters |
|---|---|---|
| `Completed` | New VM configured and stopped | 6/6, stage `Finalization` |
| `AlreadyConfigured` | Idempotent repeat; existing VM returned unchanged | 0/6, stage `Validation`, no preceding stage events |
| `Failed { stage }` | Operation failed at `stage`; typed error returned unchanged | N/6 for the N stages finished before the failure |

Validation rules: exactly one terminal event per observed operation; `Failed` always
names the stage that observed the error; `Completed` always follows the six stage events.

### `CreationProgress` (new struct, `domain/microvm.rs`)

```rust
pub struct CreationProgress {
    pub stage: CreationStage,
    pub completed_steps: u64,
    pub total_steps: u64,        // always 6 (TOTAL_CREATION_STEPS)
    pub bytes_completed: Option<u64>,
    pub expected_bytes: Option<u64>,
    pub outcome: Option<CreationOutcome>,  // None = stage event, Some = terminal event
}
```

Field rules:

- `total_steps` is always `TOTAL_CREATION_STEPS` (6).
- Stage events: `outcome` is `None`; `completed_steps` is 1–6 in emission order.
- Terminal events: `outcome` is `Some`; step counts per the table above.
- Byte counters: `Some` only on the `VolumePreparation` stage event, where both fields
  equal the requested `disk_size_bytes`; `None` everywhere else (steps-only stages and
  all terminals). Both fields are `Some` or both are `None` — never mixed.
- `Copy`: no heap fields; zero allocation at all 7 emission points.

### `Progress Observer` (call-site concept, no new type)

`Option<F> where F: FnMut(CreationProgress) + Send`, passed to `create_microvm`.
`None` receives nothing; `Some` receives the operation's full ordered stream and only
that stream. Caller-owned, infallible, non-blocking, invoked synchronously in the
caller's task. Cannot alter integrity, persistence, or error decisions.

## Invariants (all testable)

1. Step counters are monotonic within one operation and never exceed 6.
2. A successful observed creation emits exactly 7 events: stages 1/6–6/6 in order, then
   the `completed` terminal at 6/6.
3. Byte fields are `Some(disk, disk)` on exactly one event (`VolumePreparation`) and
   `None` on every other event.
4. Every observed operation ends with exactly one terminal event; failure terminals
   carry N/6 for the N already-emitted stage events plus the failed stage — no extra
   stage event for the failure itself.
5. Idempotent repeat: single `already-configured` terminal, 0/6, no stage events, no
   host changes. Conflict: single `failed` terminal at `Validation`, 0/6, existing
   typed error unchanged.
6. Nothing about the stream is persisted; returned VM metadata with an observer is
   field-identical to the same request without one.

## State transitions

No lifecycle-state change. The event stream is orthogonal to `MicroVmState`: the VM
row moves `Creating → Configured` exactly as today; progress events observe but never
drive the transition. Rollback on failure is unchanged and emits nothing itself.

## Emission map (code locations)

Current `manager.rs` references; implementers place one `emit` call at each row:

1. `Validation` 1/6 — `create_microvm`, after the existing-VM lookup returns `None`.
2. `PrerequisiteResolution` 2/6 — `create_microvm`, after `insert_creating` succeeds.
3. `VolumePreparation` 3/6 + bytes — `create_claimed_microvm`, after `prepare_rootfs`.
4. `CredentialSetup` 4/6 — after `inject_public_key`.
5. `NetworkConfiguration` 5/6 — after the guest-config write match.
6. `Finalization` 6/6 — after `update_state(Configured)`; then terminal `completed` 6/6.
7. Terminals — `failed` N/6 at each stage's error site; `already-configured` 0/6 on the
   idempotent branch; `failed` 0/6 at `Validation` for conflicts and pre-emission errors.
