# Phase 1 Data Model: VM State Verification

## Vocabulary change

### `MicroVmState` (revised, still public)

| Variant | Meaning | Source |
|---|---|---|
| `Running` | The VM's volume-local control socket answered at call time | Live probe, never persisted |
| `Stopped` | Anything else: silent socket, failed probe, timeout, no runtime record | Live probe default, never persisted |

`Creating` and `Configured` are deleted. `as_str`/`parse`/`Display` shrink to
`running`/`stopped`; the `parse` failure path (`Migration` error) stays for
defensive use. No other display vocabulary is introduced.

## Read models

### `MicroVmSummary` (shape unchanged, semantics changed)

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | Stable VM identifier, ordered by name |
| `state` | `MicroVmState` | Call-time verified state (`Running` iff the socket answers, else `Stopped`) |

The "last persisted state" doc is replaced: there is no persisted state.

### `RunningMicroVm` (unchanged)

`{ name, ssh }` — membership itself proves liveness. Still returned only for
socket-answering VMs, now selected from all stored VMs instead of
persisted-`Running` rows.

### Result types (fields kept, docs corrected)

- `MicroVmStartResult.state`: always `Running` on success (socket verified
  answering at return).
- `MicroVmCreationResult.state`: always `Stopped` on success (socket verified
  inactive at return by the existing `verify_stopped` step).
- `NetworkConfigurationResult.state`: field removed or fixed to `Stopped`;
  implementation chooses, tasks pin the choice. It described a creation-time
  constant, never a live machine.

## Persisted records

### `MicroVmRecord` (`state` field deleted)

Keeps identity, artifact references, sizing, volume/rootfs/socket paths,
`expose_on_lan`, `created_at`. It never carries lifecycle state again.

### `PersistedRuntime` (retained as-is)

`{ firecracker_path, firectl_path, socket_path, process_id: Option<u32>,
process_state: String }` stays. It is internal runtime bookkeeping — what the
SDK itself last did with the process handle — and feeds
`clear_stale_runtime`'s no-op fast path and the creation stopped-runtime
assertion. `process_id`/`socket_path` double as probe inputs.

### Completeness (replaces the `Configured` gate)

A VM record is **complete** when all four rows exist: `microvms` +
`vm_networks` + `vm_credentials` + `vm_runtime`. Creation commits the VM row
first and rolls it back (`delete_microvm` via `rollback_creation`) on any
later failure, so an incomplete row means an interrupted creation, never a
third lifecycle state. Completeness gates:

| Operation | Complete | Incomplete |
|---|---|---|
| `create_microvm` (repeat) | Immutable-field comparison, return existing | `LifecycleConflict` ("creation incomplete") |
| `configure_network` | Proceed if socket silent | `LifecycleConflict` |
| `start_microvm` | Proceed to liveness decision | `LifecycleConflict` |

## Derived verification

### `verify_one` inputs and verdict

Inputs (all owned/cloned before probing): `socket_path`, `firecracker_path`,
recorded `process_id`. Both probes run in one blocking closure; the verdict
uses the socket alone, the process evidence steers write paths:

| Socket answers | Process refs VM | Probe error/timeout | List verdict | Start action |
|---|---|---|---|---|
| yes | yes | — | `Running` | Idempotent return |
| yes | no | — | `Running` | Refuse: `TemporaryRuntime`, already managed outside |
| no | either | — | `Stopped` | Stale recovery, fresh launch |
| — | — | yes | `Stopped` | `Stopped` (read); start treats as stale unless inventory access itself failed |

A probe `Err`, a `JoinError`, or a `tokio::time::timeout` expiry all resolve
to `Stopped` on read paths — never an error, never a panic.

### Bulk verification config

| Constant | Value | Rationale |
|---|---|---|
| Permits | `(available_parallelism * 4).clamp(8, 32)` | I/O-blocked probes; 100 VMs in a few waves |
| Per-probe timeout | 12s | Covers ~10s worst-case socket probe plus margin; caps the unbounded `connect` |
| Ordering | By name | Bulk load already returns name order; fan-out must preserve it |

Entries are independent: one entry's outcome never changes another's.

## Migration 0004

| Item | Change |
|---|---|
| New file | `crates/sdk/migrations/0004_drop_microvm_state.sql` |
| Body order | `DROP INDEX IF EXISTS microvms_state;` then `ALTER TABLE microvms DROP COLUMN state;` |
| Registration | Version 4 appended to `MIGRATIONS`, order preserved |
| Required schema | `"state"` removed from the `microvms` entry in `REQUIRED_TABLES` |
| Rollback | None (project has no down-migration support); downgrade is "restore backup" |

## Derived state machine

There are no persisted transitions. Observed state only:

```text
Stopped ── socket answers ──► Running (observed, never written)
Running ── socket silent / probe fails ──► Stopped (observed, never written)
```

Write operations rewrite runtime references (`persist_runtime`), never a
state column. `start_microvm` is the only path that moves a stopped machine
to live; `clear_stale_runtime`/`reset_runtime_to_stopped` reset refs without
claiming anything.

## Ownership and multi-VM rules

- Verification is keyed per VM (own socket path, own recorded PID); no
  single-VM assumption anywhere in the fan-out.
- Read paths take no `target_lock` and perform zero writes.
- Mutating paths keep their existing per-name plus volume locks; the
  `Creating` race the old column papered over is covered by completeness plus
  locks.
- Only the child spawned by the current `start_microvm` call may be
  terminated, only on that call's failure. A recorded or foreign PID is never
  signalled.
