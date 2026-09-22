# Phase 0 Research: Delete MicroVM

## Research goals

Resolve the implementation questions for one SDK-only `delete_microvm(name)` operation: safe
deletion ordering with retry, schema reuse without migration, absent-item tolerance split between
files and network, recursive-removal containment, liveness-probe error semantics, and error
surface reuse.

All findings below were verified against the current tree (`crates/sdk/src/manager.rs`,
`ports/`, `adapters/`, `domain/`, `migrations/`).

## Findings

### Deletion order: network → volume directory → inventory record last

- Decision: release owned host network items first, remove the whole volume directory second,
  delete the inventory row last.
- Rationale: the clarified spec (Q3/Q5) requires the record to survive any failure so a retry
  resumes from the remaining owned resources. The record is the only durable index of what is
  owned (volume path, network attachment); deleting it first would orphan host state with no
  way to rediscover it. The existing `rollback_creation` in `manager.rs` already uses exactly
  this order: `network.cleanup`, then owned file removal, then `delete_microvm(vm_id)` last.
- Alternatives considered: record-first with error-carried leftovers (clarified Q5 option B).
  Rejected by the user: the record is kept on every failure and retry finishes the job.

### Persistence: no migration; existing `delete_microvm` covers the row

- Decision: reuse the existing `MicroVmRepository::delete_microvm` for the final step; no
  schema migration.
- Rationale: the schema (`crates/sdk/migrations/0002_microvm_creation.sql`) declares
  `ON DELETE CASCADE` from `microvms(id)` to `vm_networks`, `vm_network_resources`,
  `vm_credentials`, and `vm_runtime`, so deleting the row removes every child row in one
  transaction. The existing SQLite implementation additionally recomputes legacy
  `network_bridges.reference_count` and drops zero-reference SDK-owned bridges. Artifact rows
  (`kernels`, `distribution_images`, downloads) have no foreign key from `microvms`, so shared
  kernels and images are structurally unreachable from this deletion.
- Alternatives considered: a new migration adding delete-specific bookkeeping. Rejected:
  nothing needs storing; the record itself is the journal.

### Absent-tolerance split: manager skips missing files, adapter skips missing network items

- Decision: treat already-absent owned files as removed in the manager (`NotFound`-tolerant
  removal, mirroring `rollback_creation` and `remove_stale_socket`); treat already-absent owned
  network items as removed inside a new `cleanup_for_delete` port method. Only a fully unknown
  machine name errors as not-found.
- Rationale: filesystem absence is cheap to detect at the call site (`symlink_metadata` /
  `remove_dir_all` returning `NotFound`), and the codebase already does this in rollback paths.
  Network absence is adapter-specific (link existence, iptables rule existence, route/neighbour
  presence) and cannot be decided in the manager without leaking `ip`/`iptables` mechanics
  across the port boundary. The existing `cleanup` is strict in places that matter for delete
  (e.g. it fails a routed network with no committed LAN address, and `ip route del` /
  `ip neigh del` in `cleanup_routed` fail when the entry is already gone), and it is also the
  creation-rollback path — changing its strictness would alter rollback semantics. A separate
  `cleanup_for_delete` keeps both contracts intact.
- Alternatives considered: one tolerant `cleanup` for both paths. Rejected: rollback of a failed
  creation wants strict accounting of what was actually released; delete of a stopped machine
  wants end-state convergence. Different callers, different contracts.

### Recursive removal: `remove_dir_all` under containment invariants

- Decision: remove the entire volume directory with `std::fs::remove_dir_all` after verifying
  containment; map `NotFound` to success and every other I/O failure to a typed `Filesystem`
  error with the record kept.
- Rationale: `remove_dir_all` never follows symlinks, so stray links inside the volume cannot
  escape the deletion scope. The volume directory is small and VM-private (disk copy, socket,
  `ssh/`, logs, temp parts), so whole-tree removal is bounded and fast, and it satisfies the
  clarified Q4 (no empty directory left behind). Pre-removal invariants — absolute path,
  strictly below the SDK home, expected volume-internal shapes for `rootfs.ext4`,
  `firecracker.sock`, and the `ssh/id_ed25519*` key paths — reuse the shape checks already
  enforced by `validate_persisted_files` / `validate_start_prerequisites`, converted from
  start-gates into delete-gates that keep the record on violation.
- Alternatives considered: enumerating and deleting known files individually. Rejected: any new
  volume-local file added by a future feature would leak; whole-tree removal converges by
  construction.

### Liveness: `socket_answers` tri-state maps directly onto the spec

- Decision: `Ok(true)` refuses with stop-first `LifecycleConflict`; `Ok(false)` proceeds;
  `Err` propagates without deleting.
- Rationale: the adapter's `socket_answers` (`adapters/runtime/firecracker.rs`) already
  implements the tri-state the spec needs: connect `NotFound`/`ConnectionRefused`/
  `ConnectionReset`, write failure, read failure, and non-200 status all report `Ok(false)`
  (clean silence); anything else (e.g. unexpected connect errors, socket configuration
  failures) reports `Err` (unprobable). This is the same probe start and stop use, so delete
  inherits their tested definition of running versus stopped with no new probe code.
- Alternatives considered: adding a dedicated delete liveness probe. Rejected: a second probe
  definition would drift from start/stop semantics; the spec explicitly requires the same
  definition.

### Concurrency: existing per-name plus volume lock pair

- Decision: hold the `{home}/vms/{name}` lock and, when the persisted volume differs, the
  volume lock across probe, network release, file removal, and record deletion — the same
  pattern as `start_microvm` / `stop_microvm`.
- Rationale: creation already guards volume assignment with `find_volume_owner`, so two live
  records never share a volume; the lock pair additionally serializes a concurrent creation
  racing the same name or volume. No new locking primitive is needed.
- Alternatives considered: a delete-specific global lock. Rejected: per-target locks already
  give exactly-once deletion per VM without serializing unrelated VMs.

### Result shape and repeat semantics (clarified Q1/Q2)

- Decision: return `MicroVmDeleteResult { name }`; deleting an already-deleted name returns
  `NotFound`.
- Rationale: a name-carrying result keeps the operation consistent with the sibling lifecycle
  results and gives callers a confirmable value; `NotFound` on repeat keeps unknown-name
  handling uniform across operations and prevents a typo'd name from looking like a success.

### Error surface reuse

- Decision: no new public `SdkError` variant. `InvalidRequest` (name), `NotFound` (unknown
  name), `LifecycleConflict` (running machine, stop-first), `StorageConflict` (escaping or
  misshapen persisted paths), `Filesystem` (volume removal I/O), plus the network adapter's
  existing `Network` / `HostCommand` / `Cleanup` errors for unreleasable present items.
- Rationale: every delete failure already has a precise existing variant; the probe error
  propagates unchanged. No new variant means no public API breakage.

No unresolved technical questions remain for Phase 1 design.
