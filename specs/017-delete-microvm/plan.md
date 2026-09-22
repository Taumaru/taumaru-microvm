# Implementation Plan: Delete MicroVM

**Branch**: `017-delete-microvm` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/017-delete-microvm/spec.md`

## Summary

Implement one SDK-only `delete_microvm(name)` operation that permanently removes a stopped
MicroVM. The operation takes only the VM name (the SDK home comes from the existing `MicroVmSdk`
instance; every path and reference is read from the inventory record), refuses a running machine
with a typed stop-first lifecycle conflict, and otherwise deletes in the order host network
release → whole volume-directory removal → inventory-record deletion, returning a small result
carrying the deleted name. Shared kernels, images, tool binaries, and other VMs are never
touched. No CLI changes are part of this feature.

## Technical Context

**Language/Version**: Rust 2024 edition, using the repository's stable toolchain.

**Primary Dependencies**: Existing `tokio` (`fs`, `sync`), `rusqlite` (bundled SQLite),
`thiserror`; no new crates. File removal uses `std::fs::remove_dir_all` (never follows
symlinks); liveness uses the existing `socket_answers` probe through the internal runtime port;
network release goes through the network port. No shell is used.

**Storage**: Existing SQLite inventory under the explicit SDK home; no schema migration is
needed. Child rows (`vm_networks`, `vm_network_resources`, `vm_credentials`, `vm_runtime`) are
removed by the existing `ON DELETE CASCADE` foreign keys through the existing `delete_microvm`
repository method, which also recomputes legacy bridge reference counts. VM-owned files live in
the persisted volume directory (`rootfs.ext4`, `firecracker.sock`, `ssh/`, logs); delete removes
the whole directory, never individual shared paths.

**Testing**: Existing SDK unit/integration suites (`crates/sdk/tests/`, manager tests with
injected ports and fixture VMs) extended with deterministic delete tests using the fake runtime
port (scripted socket answers plus probe errors) and a fake network port (scripted cleanup
outcomes); capability-gated host tests for real socket/network/filesystem behavior stay out of
the default suite. Run the repository Cargo quality gates.

**Target Platform**: Linux hosts with permission to remove the VM volume directory and to
release the VM's owned host network items. Hosts without that permission return typed SDK
errors with the record kept for retry.

**Project Type**: Reusable SDK library in `crates/sdk`; the CLI is not modified for this feature.

**Performance Goals**: Delete performs one socket probe, one network-cleanup pass, one recursive
directory removal, and one record deletion — no waits, polls, or retries. Well under the spec's
2-minute SC-006 budget on a capable host.

**Constraints**: The SDK is silent and returns typed `Result` errors. It must not read home paths
from environment variables, accept any input beyond the VM name, treat an unprobable socket as
stopped, delete a running machine, recursively remove any directory outside the SDK home, remove
shared artifacts, report success while the record or owned resources remain, or touch other VMs.
Incomplete creation records are deletable (no completeness gate). Multiple VMs on one host are
first-class.

**Scale/Scope**: Multiple independent VMs per host; concurrent same-name deletes serialize to one
deletion sequence. This feature owns the SDK delete operation only; create, start, stop, reboot,
list, inspect, status, prune, and CLI presentation remain separate work.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle behavior; CLI remains thin | PASS: design is SDK-only | PASS: no CLI source changes or duplicate orchestration |
| Public SDK is typed, silent, non-panicking, and side-effect explicit | PASS: typed errors and port boundaries required | PASS: no output/logging/global state; network release, recursive removal, and record deletion are explicit |
| Domain is independent from infrastructure | PASS: domain/ports/adapters separation selected | PASS: liveness, network release, and filesystem mechanics stay behind ports |
| SQLite is local source of truth | PASS: record read reconciled with live socket evidence; record deleted last | PASS: no migration needed; existing cascades plus bridge refcount reused |
| Firecracker/firectl remain replaceable implementation details | PASS: runtime port reuse selected | PASS: public API exposes no socket type or host mechanism |
| Registry and artifact boundaries are explicit | PASS: no registry or artifact work in delete | PASS: shared kernels/images/tools are never deletion targets |
| Multiple MicroVMs are supported | PASS: per-VM locks, paths, and scoped network release | PASS: per-name plus volume locks; release limited to this VM's owned items |
| Public contracts and compatibility changes are documented | PASS: contract/data-model artifacts planned | PASS: Rustdoc, contract, data-model, and quickstart are included |
| Project text is English | PASS | PASS |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/017-delete-microvm/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── sdk-delete.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── src/
│   │   ├── lib.rs              # Re-export MicroVmDeleteResult
│   │   ├── manager.rs          # delete_microvm coordinator + volume-removal helper
│   │   ├── error.rs            # Reuse typed variants (no new public variants)
│   │   ├── domain/
│   │   │   ├── microvm.rs      # MicroVmDeleteResult
│   │   │   └── lifecycle.rs    # Unchanged
│   │   ├── ports/
│   │   │   ├── network.rs      # New NetworkController::cleanup_for_delete
│   │   │   ├── runtime.rs      # Reuse socket_answers
│   │   │   └── repository.rs   # Reuse find/delete_microvm
│   │   └── adapters/
│   │       └── network/
│   │           └── linux.rs    # Absent-tolerant cleanup_for_delete implementation
│   └── tests/
│       ├── lifecycle.rs        # Delete lifecycle, refusal, retry, scoping
│       └── failure_paths.rs    # Delete failure paths
└── cli/
    └── (untouched by this feature)
```

**Structure Decision**: All behavior lives in the SDK crate behind the existing
domain/ports/adapters layout. The manager orchestrates; the network adapter owns host-release
mechanics with absent-tolerance; SQLite access stays behind the existing repository trait. No new
top-level modules or CLI changes.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Deletion order is host network release → whole volume-directory removal → inventory-record
   deletion last, so any failure keeps the record and a retry resumes from the remaining owned
   resources (clarified Q3/Q5).
2. No migration: the existing `delete_microvm` repository method removes the row, cascades to
   all child rows, and recomputes legacy bridge reference counts.
3. Absent-tolerance splits by layer: already-missing files are skipped with `NotFound`-tolerant
   removal in the manager; already-absent network items are skipped inside a new
   `cleanup_for_delete` port method (existing `cleanup` stays strict for the creation-rollback
   path). Only a fully unknown machine name errors as not-found (clarified Q3).
4. Recursive removal is guarded by containment invariants (absolute path, strictly below the SDK
   home, expected volume-internal shapes for rootfs/socket/credential paths); violations are
   typed `StorageConflict` errors with the record kept.
5. Liveness uses `socket_answers` exactly: `Ok(true)` refuses with stop-first
   `LifecycleConflict`, `Ok(false)` proceeds, `Err` propagates without deleting.
6. No completeness gate: incomplete creation leftovers are deletable; absent child rows simply
   skip their step.
7. Success returns `MicroVmDeleteResult { name }` (clarified Q1); repeat delete of the same name
   returns `NotFound` (clarified Q2).
8. Errors reuse existing `SdkError` variants with no new public variant.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): request shape, records read, deletion set, result record,
  state machine, and validation rules.
- [contracts/sdk-delete.md](./contracts/sdk-delete.md): public Rust types, operation, errors,
  side-effect contract, and usage.
- [quickstart.md](./quickstart.md): SDK usage, host prerequisites, failure behavior, and gates.

### Public SDK boundary

Extend the `MicroVmSdk` facade with:

- `delete_microvm(&str) -> Result<MicroVmDeleteResult, SdkError>`.

Add `MicroVmDeleteResult { name }` to `domain::microvm`, re-export it deliberately from
`crates/sdk/src/lib.rs` (and `domain::mod`) with Rustdoc stating the deletion invariants (record
gone, whole volume directory gone, owned host network items released, shared artifacts intact).
The operation takes only the VM name; the SDK home comes from the constructed `MicroVmSdk`. Do
not add CLI commands or any other lifecycle operation in this feature.

### Delete coordinator

Implement the delete flow in `manager.rs` (or a focused private manager module) in this order:

1. Validate the VM name with the existing `validate_vm_name` helper before touching host state.
2. Acquire the per-name target lock (`{home}/vms/{name}`), load the record with `find_microvm`,
   return `NotFound` for unknown names, then acquire the volume lock when the persisted volume
   differs from the name-lock path (same pattern as `start_microvm`/`stop_microvm`).
3. Probe liveness through the runtime port: `socket_answers` decides. `Ok(true)` means running:
   return `LifecycleConflict` directing the caller to stop first, with zero host changes. `Err`
   propagates as the typed probe error without deleting. Only `Ok(false)` proceeds.
4. Deliberately skip the `require_complete` gate: incomplete creation leftovers are deletable.
   Absent child rows skip their step (`None` network means no network release).
5. Release host networking: when a persisted network exists, call the new `cleanup_for_delete`
   port method with the in-memory clone (never hold a SQLite transaction open across it). A
   present-but-unreleasable item is a typed error with the record kept; already-absent items are
   skipped inside the adapter.
6. Verify volume containment invariants (absolute, strictly below the SDK home, expected
   volume-internal shapes for rootfs/socket/credential paths); violations are typed
   `StorageConflict` errors with the record kept. Then remove the entire volume directory with
   `remove_dir_all`, treating `NotFound` as already removed. Any other removal failure is a typed
   `Filesystem` error with the record kept.
7. Delete the inventory row with the existing `delete_microvm(vm_id)` repository method (child
   rows cascade; legacy bridge refcounts recomputed). Failure keeps the error typed with the
   record still present for retry.
8. Return `MicroVmDeleteResult { name }`.

The coordinator holds both locks across the probe, network release, file removal, and record
deletion, so concurrent same-name deletes run one deletion sequence. It never uses in-memory
state as the source of truth: a fresh SDK instance recovers every path from SQLite.

### Network release design

Extend `ports::network::NetworkController` (crate-internal, so no public breakage) with:

- `cleanup_for_delete(&self, network: &PersistedNetwork) -> Result<(), SdkError>`: releases
  exactly the host items recorded as owned by this VM, skipping already-absent items as already
  removed. Failure to release a present item is a typed error.

The `LinuxNetworkController` implementation reuses the existing ownership-scoped removal core
(tap links via `delete_link_if_present`, iptables rules via existence-checked deletion) and adds
existence probes before host-route (`ip route show`) and proxy-neighbour (`ip neigh show`)
removals so absent items skip instead of failing; sysctl restorations keep their existing strict
behavior since global sysctls have no absent state. The existing strict `cleanup` method is left
unchanged for the creation-rollback path. The `TestNetwork` fake implements the new method with a
scripted outcome plus a call counter, mirroring the existing `cleanup` fake.

### File removal and containment design

A private manager helper removes the volume directory after checking, in order:

1. `volume_path` is absolute, starts with the SDK home, and is not the home itself; otherwise
   `StorageConflict` naming the persisted volume.
2. `rootfs_path` is `{volume}/rootfs.ext4` and `socket_path` is `{volume}/firecracker.sock`;
   present credential paths are `{volume}/ssh/id_ed25519` and `{volume}/ssh/id_ed25519.pub`;
   otherwise `StorageConflict`. No existence or size checks: absent files are already removed by
   definition.
3. `remove_dir_all(&volume_path)`; `NotFound` maps to success, anything else to
   `SdkError::filesystem("remove VM volume", ...)`.

`remove_dir_all` never follows symlinks, so stray links inside the volume cannot escape the
deletion scope. Nothing outside the volume directory is ever removed; shared artifact, tool,
cache, and tmp paths are never deletion targets.

### Persistence and transaction boundaries

No migration is added. The existing repository methods cover every durable step:

- `find_microvm` for lookup by name;
- `delete_microvm` for the final row deletion with cascading child rows and bridge refcount
  maintenance.

Each repository call covers one durable transition only and is never held open across network
commands or filesystem removal.

### Error and side-effect design

Reuse the existing `SdkError` surface with no new public variant:

| Situation | Variant |
|---|---|
| Empty or malformed VM name | `InvalidRequest` with field and reason; no host mutation |
| Unknown VM name (including already deleted) | `NotFound` for kind `MicroVM`; no host mutation |
| Running machine | `LifecycleConflict` with name, state `running`, and operation directing stop-before-delete; no host mutation |
| Unprobable control socket | The probe's typed error propagated unchanged; no host mutation |
| Persisted paths escape the volume or home | `StorageConflict` naming the volume; record kept for repair |
| Owned volume removal fails | `Filesystem` naming the operation and path; record kept for retry |
| Present owned network item unreleasable | The adapter's typed error (`Network`, `HostCommand`, or `Cleanup`); record kept for retry |
| Record deletion fails | The repository's typed error; record still present for retry |

The operation never signals any process, never removes any file outside the owned volume
directory, never releases unowned host network configuration, and never touches artifacts,
caches, tools, or other VM records.

### Test implementation

Add or update SDK tests for:

- stopped delete: answering-socket `false`, network released via the fake, whole volume gone,
  record unresolvable, result carries the name, shared kernel/image rows untouched;
- running refusal: answering-socket `true` returns stop-first `LifecycleConflict` with owned
  files, record, and network attachment byte-for-byte unchanged; stop-then-delete succeeds;
- repeat delete: second call returns `NotFound` with no host change;
- absent-owned-files success: volume partially or fully removed beforehand still deletes;
- absent-network-item skip: fake reports already-absent items, delete succeeds;
- network-release failure: typed error, record kept, retry after fixing succeeds;
- volume-removal failure: typed error, record kept, retry succeeds;
- containment refusal: persisted volume outside the home (or misshapen owned paths) returns
  `StorageConflict` with nothing removed;
- incomplete-record delete: rows missing child records still delete fully;
- unknown, malformed names: typed errors with no host mutation;
- multi-VM isolation: deleting one VM leaves the other's files, attachment, and lifecycle flows
  intact;
- concurrent same-name deletes: one deletion sequence, one consistent outcome;
- operation silence: no stdout/stderr, logging, tracing, or process exit through injected fakes.
