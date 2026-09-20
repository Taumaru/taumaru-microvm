# Research: VM State Verification

**Feature**: `specs/011-vm-state-verification/spec.md` | **Date**: 2026-09-20

All unknowns resolved. No `NEEDS CLARIFICATION` remains. Evidence paths are
`file:line` in the current tree.

## Decision 1: Liveness semantics are socket-governed

- **Decision**: A machine reports `Running` if and only if its volume-local
  control socket answers. The recorded-process probe
  (`process_references_vm`, `crates/sdk/src/adapters/runtime/firecracker.rs:114-147`)
  never vetoes an answering socket and never promotes a silent one. Its roles
  are ownership evidence for write paths (never signal a foreign PID, detect
  adoption) and diagnostics.
- **Rationale**: Clarification Q1 fixed this explicitly, and it is the only
  rule that covers machines restarted manually outside the CLI: an answering
  socket proves a live server for that VM's control channel regardless of what
  the recorded PID says. The existing `is_vm_live`
  (`crates/sdk/src/manager.rs:1205-1216`) already sequences the two probes; the
  change is making the socket decisive instead of requiring both.
- **Alternatives considered**:
  - Both-required (socket AND recorded process must agree): rejected — a live
    manually-restarted VM would report stopped, contradicting the feature
    request and clarification Q1.
  - PID-primary: rejected — recycled PIDs and stale records are the original
    bug; a live PID alone proves nothing about this VM.

## Decision 2: State vocabulary collapses to Running / Stopped

- **Decision**: `MicroVmState` (`crates/sdk/src/domain/lifecycle.rs:5-12`)
  loses `Creating` and `Configured`, keeping `Running` and `Stopped`.
  `MicroVmSummary.state` becomes the call-time verified value. Result types
  keep their `state` field with corrected docs: `MicroVmCreationResult.state`
  is `Stopped` (socket verified inactive at return via the existing
  `verify_stopped` step), `MicroVmStartResult.state` is `Running` (socket
  verified answering at return).
- **Rationale**: Clarification Q4 mandates running-or-stopped only; creation
  progress is not a state. Keeping the field with a truthful constant
  preserves struct shape for crates.io consumers while removing the dead
  vocabulary. Creation success implies completeness by construction (all child
  records committed), so no `Configured` marker is needed to express it.
- **Alternatives considered**:
  - Removing `state` from all result types: rejected — gratuitous breakage
    for zero information gain; the constants are truthful point-in-time
    values, not persisted state.
  - Keeping `Creating`/`Configured` as deprecated variants: rejected — the
    spec mandates the vocabulary removal, and deprecated variants invite new
    matches on meaningless states.

## Decision 3: Migration 0004 drops the column forward-only

- **Decision**: New `crates/sdk/migrations/0004_drop_microvm_state.sql`,
  registered as version 4 in `MIGRATIONS`
  (`crates/sdk/src/adapters/persistence/migrations.rs:15-31`), containing in
  this order:
  ```sql
  DROP INDEX IF EXISTS microvms_state;
  ALTER TABLE microvms DROP COLUMN state;
  ```
  Same change removes `"state"` from the `microvms` entry in
  `REQUIRED_TABLES`, and rewrites every `microvms.state` statement in
  `crates/sdk/src/adapters/persistence/sqlite.rs` (find ~`sqlite.rs:743-749`,
  list ~`:792`, insert ~`:832-840`, `update_state` ~`:930-937`,
  `record_from_row` parse ~`:1049-1050`).
- **Rationale**: The runner (`migrations.rs:52-88`) is forward-only,
  checksum-gated, one transaction per migration, with a fail-closed drift
  gate — editing `0002_microvm_creation.sql` would trip
  `migration 2 drifted` on every existing database, so a new version is the
  only legal path. Bundled SQLite supports `DROP COLUMN`: pinned
  `rusqlite 0.40.2` / `libsqlite3-sys 0.38.2` (Cargo.lock) build a SQLite far
  newer than 3.35.0, which introduced it. `state` is not PK/UNIQUE/FK, so its
  only DROP blockers are the `microvms_state` index
  (`0002_microvm_creation.sql:94`, dropped first) and its own inline
  `CHECK`, which leaves with the column definition. `verify_table_columns`
  (`migrations.rs:288-310`) is a subset check, so dropping `"state"` from the
  required list is sufficient — no verifier logic change.
- **Alternatives considered**:
  - Table rebuild (`CREATE new / COPY / DROP / RENAME`): rejected —
    unnecessary machinery when `DROP COLUMN` is supported.
  - Editing `0002` in place: rejected — checksum drift gate forbids it.
  - Down migration: rejected — the project has no down-migration support
    (no `down` SQL, no downgrade path anywhere in `adapters/persistence`);
    downgrade guidance is "restore a pre-migration backup", documented in the
    contract.

## Decision 4: State gates become completeness + liveness + locks

- **Decision**: Every `record.state` gate is replaced, not removed blindly:
  - `configure_network` (`manager.rs:1020-1025`): gate on child-record
    completeness (network, credential, runtime rows all present) plus stopped
    (socket silent; existing `verify_stopped` call stays).
  - `start_microvm` (`manager.rs:1109-1124`): gate on completeness, then
    socket-governed liveness — live goes idempotent, silent goes fresh start.
    The `Creating` rejection is subsumed by completeness (an in-progress
    creation has no child rows yet) plus the existing per-volume lock
    (`target_lock`, `manager.rs:2228-2242`), which serializes same-target ops
    in-process.
  - `return_or_reject_existing_creation` (`manager.rs:1390-1406`): complete
    existing record goes through the immutable-field comparison as today;
    incomplete record fails with `LifecycleConflict` ("creation incomplete").
  - `SdkError::LifecycleConflict` is retained; its `state` string payload
    carries the verified state (`running`/`stopped`) or the condition
    (`creation incomplete`).
- **Rationale**: The column carried two independent meanings — "creation
  finished" and "machine live" — and each has a better source now:
  completeness from child-row presence (creation commits them atomically per
  stage and rolls back the VM row on failure via `rollback_creation`,
  `manager.rs:2140-2222`), liveness from probes, mutual exclusion from locks.
- **Alternatives considered**:
  - New claim/progress column: rejected — reintroduces persisted-state flavor
    through the back door; locks plus completeness suffice for the gates that
    remain.
  - Filesystem lock files in the volume dir: rejected — new mechanism with
    cross-process stale-cleanup problems; the in-process mutex map plus
    completeness checks cover the same races the old column covered.

## Decision 5: Bulk concurrency is semaphore + spawn_blocking + timeout

- **Decision**: `list_microvms` / `list_running_microvms` load all stored VMs,
  then verify under `tokio::sync::Semaphore` with
  `permits = (available_parallelism * 4).clamp(8, 32)`, one
  `tokio::task::spawn_blocking` per VM running the sync probe pair, each
  wrapped in `tokio::time::timeout(PROBE_TIMEOUT_SECS = 12)`, collected via
  `tokio::task::JoinSet`. Any per-VM outcome other than "socket answers" —
  silent, probe `Err`, timeout elapsed, join error — resolves that entry to
  `Stopped` (FR-007). No new dependencies.
- **Rationale**: Ground facts — `run_repository` already hops to the blocking
  pool per call (`manager.rs:2244-2251`); `SqliteRepository` holds only a path
  (`sqlite.rs:29-31`) and opens a fresh `Connection` per method with a 5s busy
  timeout (`sqlite.rs:25,1490-1495`), so concurrent reads are safe. Probes are
  fully blocking (`RuntimeController: Send + Sync`, `ports/runtime.rs:7`;
  stateless `FirecrackerRuntime`, `firecracker.rs:18-20`), hence
  `spawn_blocking` with a cloned `Arc<dyn RuntimeController>` and owned
  `PathBuf`s is sound. The socket `connect` has no timeout parameter
  (`firecracker.rs:152`) while read/write cap at 5s each (`:173,:178`), so a
  ~10s worst case plus margin gives the 12s outer bound; `wait_for_socket`'s
  60s loop (`:249-260`) is launch-readiness only and stays out of the fan-out.
  All primitives (`sync`, `rt`, `time`, `macros`) are already enabled in the
  workspace tokio features; `futures`/`buffer_unordered` appears nowhere in
  the manifests, so `JoinSet` avoids a Cargo change. The 4x-core multiplier
  fits I/O-blocked probes; the 8..32 clamp keeps 100-VM listings to a handful
  of waves dominated by fast connect-refusals, satisfying SC-002 (<10s for a
  typical mixed inventory). A pathological all-hung fleet can exceed 10s total
  — bounded per probe, documented, accepted.
- **Alternatives considered**:
  - `futures::buffer_unordered`: rejected — same shape, requires a new
    dependency for no gain.
  - Unbounded spawn per VM: rejected — 100+ concurrent blocking tasks plus
    socket pressure near the SC-002 scale point.
  - Sequential probing: rejected — N x ~10s worst case violates SC-002 by
    construction.
  - Async sockets via `tokio::net`: rejected — the runtime port is a sync
    trait; an async rewrite expands the blast radius to every caller and test
    double for no measurable gain on localhost Unix sockets.
  - `stat`-before-probe short-circuit for missing socket files: rejected —
    extra branch with TOCTOU flavor for negligible gain; connect-refused on a
    missing path is already the fast path.

## Decision 6: One bulk-load repository method, no listing locks

- **Decision**: Add one `MicroVmRepository` method returning all stored VMs
  ordered by name (one connection, replacing 1 + N round trips), narrow
  `list_microvm_names` to names-only or retire it if no caller remains, and
  delete `update_state`. Read paths take no `target_lock`.
- **Rationale**: 100 VMs currently cost 101 sequential connection setups;
  bulk verification deserves one read. Listing takes no locks because it
  performs no writes (FR-008); locks exist to serialize mutating same-target
  ops, and grabbing them for reads would serialize the fan-out against
  in-flight starts.
- **Alternatives considered**: keeping 1+N reads — rejected, needless latency
  multiplication at exactly the scale point the feature cares about.

## Decision 7: vm_runtime.process_state stays, update_state goes

- **Decision**: `vm_runtime.process_state` (`stopped`/`running`) is retained as
  internal runtime bookkeeping (it feeds `clear_stale_runtime`'s no-op fast
  path, `manager.rs:1221-1226`, and creation's stopped-runtime assertion,
  `manager.rs:2860-2870`). Only `microvms.state` — the displayed-state source
  — is removed, with `update_state` deleted from the trait and its four call
  sites (`manager.rs:1064,1186,1358,2094`) reduced to runtime-only commits.
  `insert_creating` is renamed to a state-free insert.
- **Rationale**: `process_state` is never displayed; it records what the SDK
  itself last did with the process handle. Removing it would force
  `Option<process_id>` to carry the stopped/starting distinction it cannot
  express, for no spec benefit.

## Decision 8: Foreign-live start never launches, never touches

- **Decision**: If the socket answers but the recorded PID is missing or does
  not reference this VM, `start_microvm` returns `TemporaryRuntime`
  (`stopped: false`, "machine is already running outside SDK management")
  instead of launching a duplicate or claiming the foreign process.
  `build_start_result`'s PID-equality check (`manager.rs:2743-2749`) stays as
  the enforcement point.
- **Rationale**: The idempotent fast path needs a truthful `process_id` for
  `MicroVmStartResult`; inventing one for an unmanaged process would be a lie
  the forced-termination fallback could later act on. Refusing to duplicate
  while refusing to adopt is the only safe combination.

## Decision 9: CLI renders verified state, drops one guard

- **Decision**: `start.rs` selector keeps listing all VMs with `name [state]`
  labels (now verified); `ssh.rs` keeps its `Running` pre-filter (now
  truthful) and `resolve_running_entry` flow unchanged; `new.rs:1042` drops
  the `Configured` post-check (creation `Ok` implies completeness; the
  constant `Stopped` field carries nothing to assert).
- **Rationale**: No CLI flow needs restructuring — every state read already
  funnels through `list_microvms` / `list_running_microvms`, which become
  truthful underneath. The `new.rs` check asserted a vocabulary that no
  longer exists.
