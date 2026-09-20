# Implementation Plan: VM State Verification

**Branch**: `011-vm-state-verification` | **Date**: 2026-09-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/011-vm-state-verification/spec.md`

## Summary

Replace the persisted lifecycle-state source of truth with call-time,
socket-governed verification. A database migration (version 4) drops the
`microvms.state` column, eliminating stale `running` markers on existing
databases with it. The public `MicroVmState` vocabulary collapses to
`Running` / `Stopped`: a machine reports `Running` if and only if its
volume-local control socket answers, with the recorded-process probe kept
strictly as a false-positive guard (PID-reuse protection) and as ownership
evidence for write paths. Bulk listings verify all machines concurrently
under a CPU-scaled semaphore (`(available_parallelism * 4).clamp(8, 32)`)
with a 12s per-probe timeout, resolving any probe failure or timeout to
`Stopped` without aborting the listing. Former `record.state` lifecycle
gates become completeness checks (all four child rows present) plus liveness
plus the existing per-target locks. The CLI needs no flow restructuring —
every state read already funnels through `list_microvms` /
`list_running_microvms`, which become truthful underneath — plus one dropped
post-check in `new.rs`. This is a breaking SDK change (two enum variants
removed, one column dropped, result-type constants corrected) shipped with
explicit compatibility notes; no new dependency is added.

## Technical Context

**Language/Version**: Rust 2024 edition, repository stable toolchain
(`rustc 1.98.1`).

**Primary Dependencies**: No new crates. `tokio` workspace features already
enabled (`fs`, `io-util`, `macros`, `net`, `process`, `rt`, `signal`,
`sync`, `time`) cover `tokio::sync::Semaphore`, `spawn_blocking`,
`JoinSet`, `time::timeout`; `rusqlite 0.40.2` with bundled SQLite (via
`libsqlite3-sys 0.38.2`) supports `ALTER TABLE DROP COLUMN`; `thiserror`,
`serde`/`serde_json` unchanged. `futures`/`buffer_unordered` is deliberately
not introduced — `JoinSet` covers the fan-out with zero manifest changes.

**Storage**: Existing SQLite inventory under the explicit SDK home. One
forward-only migration `0004_drop_microvm_state.sql` (version 4):
`DROP INDEX IF EXISTS microvms_state;` then
`ALTER TABLE microvms DROP COLUMN state;`. The `microvms` entry in
`REQUIRED_TABLES` drops `"state"`; all `microvms.state` statements in
`sqlite.rs` are rewritten; `update_state` is deleted from the repository
trait. `vm_runtime.process_state` stays as internal runtime bookkeeping.

**Testing**: Existing suites extended — manager unit tests with the injected
`TestRuntime` (socket-governs verdict matrix, foreign-live refusal,
bulk mixed-state with injected failures), integration tests
(`microvm_listing.rs` rewritten, `sqlite_persistence.rs` migration v3→v4 on
a seeded database, `public_api.rs` export pins), CLI tests (selector labels,
`ssh` running-filter). Real Firecracker/KVM integration stays
capability-gated out of the default suite. Repository gates:
`cargo fmt --check`, `cargo check`, `cargo clippy -D warnings`,
`cargo test` (all-targets, all-features).

**Target Platform**: Linux hosts. Read-path verification needs no KVM —
only `/proc` readability and Unix-socket connect rights. Launch/start paths
keep their existing KVM/privilege requirements.

**Project Type**: Reusable SDK library in `crates/sdk` plus thin CLI updates
in `crates/cli`; no new crate, no new CLI command.

**Performance Goals**: SC-002 — 100 mixed-state machines list in under 10s
on a typical host. Fast path (connect-refused on missing/silent sockets)
dominates; the fan-out turns the ~10s worst-case single probe into a few
waves of ≤32. A pathological all-hung fleet is bounded per probe (12s) and
documented, not optimized further.

**Constraints**: SDK stays silent and panic-free (typed `Result`s only; no
`unwrap`/`expect`/`panic!` on user/host-controlled paths). Read paths
perform zero writes and take no locks. Probes never signal a process. Only
the child spawned by the current `start_microvm` call may be terminated,
only on that call's failure. Socket and rootfs paths must stay inside the
VM volume (existing invariants, unchanged). Multiple VMs per host are
first-class; verification is always keyed per VM.

**Scale/Scope**: Tens to ~100 VMs per host. In scope: SDK verification core,
migration 0004, gate rewrites, bulk fan-out, CLI label truthfulness, tests,
docs. Out of scope: stop/reboot/delete/inspect/status operations, new
lifecycle states, read-path DB healing, async runtime-port rewrite, lock
files or any new cross-process mechanism.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| I. SDK owns lifecycle; CLI stays thin | PASS: verification designed SDK-side, CLI renders only | PASS: `verify` core + fan-out + migration live in SDK; CLI changes are label/guard deletions, no orchestration |
| II. Panic-free, silent SDK | PASS: probe failures must resolve, never throw | PASS: per-VM `Err`/timeout/`JoinError` → `Stopped`; inventory failures stay typed; no output/logging/globals; no new `unwrap` paths |
| III. Explicit local state and lifecycle | PASS: reconcile persisted metadata with live process state | PASS: DB keeps identity/config/runtime refs (source of truth for those); liveness is derived explicitly via the two probes with a documented verdict table; no silent state |
| IV. Closed/open + breaking-change discipline | PASS: additive where possible; breaking parts flagged | PASS with declared breakage: enum variants and column removed with compatibility notes below; no new layers, no new dep, ports/adapters boundary intact |
| V. Calm CLI, intentional docs, English | PASS: existing labels reused, Rustdoc updated | PASS: `running`/`stopped` labels only, no color-only signaling; public items get corrected Rustdoc; all text English |
| Two-crate direction (SDK never depends on CLI) | PASS | PASS: no CLI import from SDK; test doubles stay in-tree |
| Registry boundary untouched | PASS | PASS: no registry code touched |
| Host data-directory ownership untouched | PASS | PASS: layout and env rules unchanged |
| Tests + quality gates | PASS: plan covers unit/integration/failure-path/CLI tests | PASS: test implementation section below; gates listed in quickstart |

### Declared breaking changes (constitution IV compliance)

1. `MicroVmState::{Creating, Configured}` removed. Code matching on them
   must handle `Running`/`Stopped` only. Migration: replace creation-progress
   checks with operation success (`Ok` implies completeness); replace
   configured/creating gates with the completeness + liveness rules in the
   contract.
2. `microvms.state` column dropped by migration 4. Databases auto-migrate on
   next SDK open; raw-SQL readers of the column must stop. No downgrade path
   (project has no down-migration support); downgrade is "restore a
   pre-migration backup".
3. `MicroVmCreationResult.state` is now always `Stopped`;
   `NetworkConfigurationResult.state` is removed. Consumers asserting
   `Configured` must update.
4. `list_microvms` now performs host I/O per VM (bounded, concurrent) instead
   of a pure DB read; callers needing identity-only data should use the
   entries' names. Latency characteristics are documented in the contract.

No complexity exception is required: no new crate, module layer, or
dependency. One migration file, one private verify helper, one repository
method replacing two.

## Project Structure

### Documentation (this feature)

```text
specs/011-vm-state-verification/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── sdk-state-verification.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── migrations/
│   │   └── 0004_drop_microvm_state.sql   # New: drop index + drop column
│   ├── src/
│   │   ├── lib.rs                        # Unchanged exports; corrected Rustdoc
│   │   ├── error.rs                      # Unchanged (LifecycleConflict retained)
│   │   ├── manager.rs                    # Verify core, fan-out, gate rewrites, runtime-only commits
│   │   ├── domain/
│   │   │   ├── lifecycle.rs              # MicroVmState shrinks to Running/Stopped
│   │   │   └── microvm.rs                # Record loses state; result docs corrected; NetworkConfigurationResult loses state
│   │   ├── ports/
│   │   │   ├── repository.rs             # Bulk-load method added; update_state deleted
│   │   │   └── runtime.rs                # Unchanged (probes reused as-is)
│   │   └── adapters/
│   │       ├── persistence/
│   │       │   ├── migrations.rs         # Register v4; drop "state" from required microvms columns
│   │       │   └── sqlite.rs             # Rewrite state SQL; implement bulk load; delete update_state
│   │       └── runtime/
│   │           └── firecracker.rs        # Unchanged (probe mechanics reused as-is)
│   └── tests/
│       ├── microvm_listing.rs            # Rewritten: verified states, socket fixtures, migration seed
│       ├── public_api.rs                 # Updated export/variant pins
│       └── sqlite_persistence.rs         # v3→v4 column-removal coverage
└── cli/
    ├── src/
    │   ├── commands/
    │   │   ├── new.rs                    # Drop Configured post-check
    │   │   ├── start.rs                  # Unchanged flow (labels now verified)
    │   │   └── ssh.rs                    # Unchanged flow (filter now truthful)
    │   └── output/
    │       └── human.rs                  # Test fixtures updated; no render change
    └── tests/
        └── command_surface.rs            # Untouched (no lifecycle flags involved)
```

**Structure Decision**: All behavior lives in the SDK behind the existing
domain/ports/adapters layout. The manager orchestrates verification; the
runtime adapter is reused unchanged; SQLite access stays behind the
repository trait with one added bulk method and one deleted method. The CLI
is touched only where it asserted the removed vocabulary. No new top-level
module, no new crate, no new dependency.

## Complexity Tracking

No constitution violations or additional project layers require
justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). Resolved decisions:

1. **Socket governs**: `Running` iff the volume-local socket answers; the
   process probe is a false-positive guard and write-path ownership evidence,
   never a veto on a live socket nor a promotion of a silent one.
2. **Two-state vocabulary**: `MicroVmState` keeps `Running`/`Stopped`;
   result types keep truthful constant `state` fields (`StartResult:
   Running`, `CreationResult: Stopped`); `NetworkConfigurationResult.state`
   is removed.
3. **Migration 0004 drops the column forward-only**: `DROP INDEX` then
   `DROP COLUMN` as version 4 (editing `0002` would trip the checksum drift
   gate); bundled SQLite supports it; `REQUIRED_TABLES` drops `"state"`;
   no down-migration support exists.
4. **Gates become completeness + liveness + locks**: child-row presence
   replaces `Configured`, locks plus completeness cover the `Creating` race,
   `LifecycleConflict` is retained with verified-state payloads.
5. **Fan-out is semaphore + spawn_blocking + timeout**: permits
   `(available_parallelism * 4).clamp(8, 32)`, 12s per-probe bound capping
   the unbounded `connect`, `JoinSet` collection — no Cargo changes
   (`futures` absent from manifests; `wait_for_socket`'s 60s loop stays out
   of reads).
6. **One bulk-load repository method, no listing locks**: single connection
   replacing 1+N round trips; reads take no `target_lock`.
7. **`vm_runtime.process_state` stays**: internal bookkeeping only, never
   displayed; only `microvms.state` goes.
8. **Foreign-live start refuses safely**: `TemporaryRuntime`
   (`stopped: false`), no duplicate launch, no adoption, no signalling.
9. **CLI renders verified state**: `start`/`ssh` flows unchanged, `new.rs`
   guard dropped.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): vocabulary, read models, persisted
  records, completeness, verdict table, fan-out constants, migration shape.
- [contracts/sdk-state-verification.md](./contracts/sdk-state-verification.md):
  public types, operations, errors, side effects, migration, CLI
  presentation, usage.
- [quickstart.md](./quickstart.md): manual validation scenarios and gates.

### Public SDK boundary

`MicroVmState` (`domain/lifecycle.rs`) keeps its name with two variants,
`Running` and `Stopped`, plus `as_str`/`parse`/`Display` shrunk to match.
Module and crate re-exports (`domain/mod.rs`, `lib.rs`) are unchanged; the
Rustdoc on the enum, on `MicroVmSummary.state` ("call-time verified state"),
and on both result types is rewritten to state the new invariants.
`MicroVmSummary { name, state }` keeps its shape with changed semantics;
`RunningMicroVm { name, ssh }` is untouched. `list_microvms` and
`list_running_microvms` keep their signatures with rewritten contracts
(verified per entry, zero writes, inventory errors still typed).
`LifecycleConflict` is retained; its `state` string now carries the verified
state or the condition (`creation incomplete`).

### Verification core

One private verify path in `manager.rs` (a focused private submodule is
acceptable if the manager grows past cohesion; no new public module):

- `verify_one(socket_path, firecracker_path, process_id) -> Verdict` runs the
  existing probe pair (`process_references_vm`, then `socket_answers`) inside
  a single blocking closure. Verdict rule: socket answers → `Running`
  (regardless of process evidence); anything else — silent socket, probe
  `Err`, timeout, join failure — → `Stopped`.
- `is_vm_live` is refactored into two pieces: the socket verdict above for
  reads, and process-evidence branching for `start_microvm` (live +
  referencing → idempotent; live + missing/foreign → refuse with
  `TemporaryRuntime`; silent → stale recovery). `build_start_result`'s
  PID-equality check stays as the enforcement point for the idempotent path.
- Constants (private, beside the helper): permits
  `(available_parallelism * 4).clamp(8, 32)` with a hardcoded fallback when
  parallelism is unavailable, and `PROBE_TIMEOUT_SECS = 12`.
- `Arc<dyn RuntimeController>` is cloned per VM with owned `PathBuf`s into
  `spawn_blocking`; each handle is wrapped in `time::timeout` and collected
  via `JoinSet`, preserving name order.

### Bulk fan-out

New `MicroVmRepository` bulk method returning all stored VMs ordered by name
(one connection, children loaded on that same connection), replacing the
1+N `list_microvm_names` + per-row `find_microvm` pattern.
`list_microvm_names` is retired if no caller remains; otherwise narrowed to
names-only (tasks verify). Both public listings fan out over the bulk load;
`list_running_microvms` additionally filters to socket-answering VMs and
builds the SSH payload from the already-loaded credential/network rows. Read
paths take no `target_lock` and perform no writes.

### Persistence and migration

- New `crates/sdk/migrations/0004_drop_microvm_state.sql` with exactly the
  two statements in order, registered as version 4; `"state"` removed from
  the `microvms` required-columns list.
- `sqlite.rs`: `find_microvm` SELECT, list query, `insert_creating` (renamed
  to a state-free `insert_microvm`), `update_state` (deleted), and
  `record_from_row` parse all drop the column; new bulk-load implementation
  added.
- `MicroVmRecord.state` deleted; the four `update_state` commit sites
  (configure, start success, stale reset, creation finalization) become
  runtime-only persists. Creation still seeds the VM row first and rolls it
  back on failure, preserving the completeness signal.

### Lifecycle gate rewrites

| Operation | Old gate | New gate |
|---|---|---|
| `create_microvm` repeat | `state != Configured` → conflict | Incomplete record → `LifecycleConflict`; complete → immutable-field comparison |
| `configure_network` | `state != Configured` → conflict | Incomplete → conflict; live socket → `TemporaryRuntime` (`stopped: false`); silent → proceed (`verify_stopped` stays) |
| `start_microvm` | `Creating` / non-`Configured`/`Running` → conflict | Incomplete → conflict; then socket-governed branch (idempotent / refuse-foreign / fresh launch) |
| `build_start_result` | PID equality + path checks | Unchanged |

`clear_stale_runtime`, `reset_runtime_to_stopped`, `remove_stale_socket`,
`cleanup_failed_launch`, and `validate_persisted_files` keep their logic
minus the state column.

### CLI presentation

- `start.rs`: unchanged flow; selector labels render verified states.
- `ssh.rs`: unchanged flow; `Running` pre-filter and
  `resolve_running_entry` become truthful with no code change required
  beyond type updates if signatures shift.
- `new.rs`: the `result.state != Configured` post-check is deleted
  (creation `Ok` implies completeness).
- `human.rs`: fixtures updated to the new vocabulary; render functions need
  no change (they never branched on state).

### Error and side-effect design

No new public error variant. Per-VM probe failures resolve to `Stopped` on
read paths; inventory access failures (`Sqlite`, `Migration`) remain typed
errors; start/configure refusals use `LifecycleConflict` and
`TemporaryRuntime` as tabulated in the contract. Reads are silent, lock-free,
and write-free; the SDK gains no dependency and no global state.

### Test implementation

- Manager unit tests (injected `TestRuntime`): full verdict matrix (live,
  silent, recycled-PID, foreign-live, missing record), foreign-live start
  refusal (no launch, no signalling), stale-recovery launch, bulk
  mixed-state correctness with injected per-VM failures and a slow probe
  (parallelism smoke: bounded time, ordered output, isolated failures),
  completeness gates for all three operations, silence under failure
  injection.
- Integration tests: `microvm_listing` rewritten (raw-SQL seeder drops the
  `state` column; real `UnixListener` fixtures answering `200` vs silent
  for true/false entries; empty inventory; ordering); `sqlite_persistence`
  seeds a v3-schema database and asserts the column is gone and data
  survives after open; `public_api` pins updated (`Stopped` presence,
  `Creating`/`Configured` absence).
- CLI tests: selector label rendering with verified states, `ssh`
  running-filter behavior; `command_surface` untouched.
- Existing start/create/network suites updated where fixtures set or assert
  the removed vocabulary (`start_fixture_vm` drops its `state` param,
  `update_state` callers removed, `human.rs` fixtures fixed).
