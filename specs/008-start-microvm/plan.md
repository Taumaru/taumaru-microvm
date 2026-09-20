# Implementation Plan: Start Configured MicroVM

**Branch**: `008-start-microvm` | **Date**: 2026-09-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/008-start-microvm/spec.md`

## Summary

Implement one SDK-only `start_microvm(name)` operation that turns a `Configured` (stopped) MicroVM
into a durably `Running` one. The operation takes only the VM name (the SDK home comes from the
existing `MicroVmSdk` instance; the volume directory is read from the inventory record), rejects
`Creating` rows, decides running-versus-stopped from live host evidence (recorded process liveness
plus an active control-socket connection, never stored state alone), reuses the existing
`NetworkController::configure` reconciliation to repair only missing or stale host network items,
launches the full persisted configuration through the runtime adapter as a detached background
process with the socket inside the VM volume, and persists `Running` state with the process ID and
socket path only after the control channel answers. Repeated start while genuinely running returns
the current running identity without a second process; any process/socket mismatch is treated as
stale and started fresh. No CLI changes are part of this feature.

## Technical Context

**Language/Version**: Rust 2024 edition, using the repository's stable toolchain.

**Primary Dependencies**: Existing `tokio` (already enabled: `fs`, `io-util`, `macros`, `net`,
`process`, `rt`, `signal`, `sync`, `time`), `rusqlite` (bundled SQLite), `serde`/`serde_json`,
`thiserror`; no new crates. Host integration uses typed `tokio::process::Command` argument vectors
and `tokio::net::UnixStream` through the internal runtime port; no shell is used. Process liveness
is read from `/proc` via the standard library filesystem APIs only.

**Storage**: Existing SQLite inventory under the explicit SDK home; no schema migration is needed
(`microvms.state` already parses `"running"`; `vm_runtime` already stores `process_id` and
`process_state`). VM-local files stay in the persisted volume directory (`rootfs.ext4`,
`ssh/` keys, `firecracker.sock`); launch output is captured to a VM-local log file inside the same
directory.

**Testing**: Existing SDK unit/integration suites (`crates/sdk/tests/`, manager tests with injected
ports) extended with deterministic start tests using fake network/runtime ports; capability-gated
host tests for real KVM/socket/process behavior stay out of the default suite. Run the repository
Cargo quality gates.

**Target Platform**: Linux hosts with KVM, readable/writable `/dev/kvm`, permission to reconcile
TAP/routes/forwarding/nftables state, and a verified artifact inventory (kernel, Firecracker,
`firectl` binaries) from creation time. Unsupported or insufficiently privileged hosts return typed
SDK errors.

**Project Type**: Reusable SDK library in `crates/sdk`; the CLI is not modified for this feature.

**Performance Goals**: Repeated start against a running VM performs only live checks and returns
without relaunch; network reconciliation skips already-correct resources; post-launch socket
readiness is a bounded poll (sub-second interval, well under the spec's 2-minute SC-001 budget).
No background worker or unbounded wait is introduced.

**Constraints**: The SDK is silent and returns typed `Result` errors. It must not read home paths
from environment variables, accept or re-derive a volume path, move or reinterpret the persisted
volume, place an active socket outside the volume, launch a second process over a live machine,
kill an unrelated process on PID reuse, or leave an orphan process or false `Running` claim after a
failed launch. Multiple VMs on one host are first-class.

**Scale/Scope**: Multiple independent VMs per host; concurrent same-name starts serialize to one
process. This feature owns the SDK start operation only; stop, reboot, delete, list, inspect,
status, guest-readiness, and CLI presentation remain future work.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle behavior; CLI remains thin | PASS: design is SDK-only | PASS: no CLI source changes or duplicate orchestration |
| Public SDK is typed, silent, non-panicking, and side-effect explicit | PASS: typed errors and port boundaries required | PASS: no output/logging/global state; launch, probe, and cleanup are explicit |
| Domain is independent from infrastructure | PASS: domain/ports/adapters separation selected | PASS: process, socket, SQLite, and network stay behind ports |
| SQLite is local source of truth | PASS: record, network, runtime refs persisted and reconciled with live state | PASS: no migration needed; atomic persist-runtime plus state transition after readiness |
| Firecracker/firectl remain replaceable implementation details | PASS: runtime port extension selected | PASS: public API exposes no command, PID type beyond `u32`, or socket type |
| Registry and artifact boundaries are explicit | PASS: boot-artifact readiness reverified from inventory, never downloaded | PASS: kernel/binary resolution reuses existing inventory records |
| Multiple MicroVMs are supported | PASS: per-VM locks, paths, and network identity | PASS: per-name plus volume locks; no shared mutable start state |
| Public contracts and compatibility changes are documented | PASS: contract/data-model artifacts planned | PASS: Rustdoc, contract, data-model, and quickstart are included |
| Project text is English | PASS | PASS |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/008-start-microvm/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── sdk-start.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── src/
│   │   ├── lib.rs              # Re-export MicroVmStartResult
│   │   ├── manager.rs          # start_microvm coordinator
│   │   ├── error.rs            # Reuse typed variants (no new public variants expected)
│   │   ├── domain/
│   │   │   ├── microvm.rs      # MicroVmStartResult
│   │   │   └── lifecycle.rs    # Running becomes a durable post-start state
│   │   ├── ports/
│   │   │   ├── runtime.rs      # Extended RuntimeController: liveness, launch, readiness
│   │   │   └── repository.rs   # Reuse find/update/persist methods
│   │   └── adapters/
│   │       └── runtime/
│   │           └── firecracker.rs  # firectl argv, detached spawn, socket probe
│   └── tests/
│       ├── lifecycle.rs        # Start lifecycle, idempotency, stale recovery
│       └── failure_paths.rs    # Start failure paths
└── cli/
    └── (untouched by this feature)
```

**Structure Decision**: All behavior lives in the SDK crate behind the existing
domain/ports/adapters layout. The manager orchestrates; the runtime adapter owns process and
socket mechanics; the network adapter is reused unchanged; SQLite access stays behind the existing
repository trait. No new top-level modules or CLI changes.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Liveness means both a verified-live recorded process (`/proc/<pid>` exists and its command line
   still references this VM's socket or runtime binary) and an answering control socket (UDS
   connect plus a minimal Firecracker API read); any disagreement is stale.
2. Launch builds the same `firectl` argument set creation validated (Firecracker binary, kernel,
   VM-local rootfs, vCPUs, effective MiB, TAP/MAC, boot parameters, volume-local socket) and
   spawns it detached with stdio captured to a VM-local log file, recording the child PID and
   forgetting the handle with `kill_on_drop(false)`.
3. Readiness is a bounded poll of the volume-local socket; only a responsive socket commits the
   durable `Running` state. Launch failure kills only the just-spawned child, keeps repaired
   network in place, and leaves the VM `Configured`.
4. Network repair reuses `NetworkController::configure` with the persisted network as `existing`,
   so correct items are skipped and only missing or SDK-owned stale items are recreated with the
   persisted identity unchanged.
5. Concurrency reuses the existing per-name plus volume `target_lock` pair, so concurrent starts
   for one VM produce at most one process.
6. Persistence needs no migration: one repository closure writes `persist_runtime` (PID,
   `"running"`) and `update_state(Running)` after readiness; failure paths reset runtime refs to
   stopped without claiming `Running`.
7. Errors reuse existing `SdkError` variants (`NotFound`, `InvalidRequest`, `LifecycleConflict`,
   `ArtifactPrerequisite`, `Network`, `TemporaryRuntime`, `HostCommand`, `RuntimeIncompatible`,
   `Filesystem`, storage/credential/guest variants); no new public error variant is expected.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): request shape, result record, state machine, ownership, and
  validation rules.
- [contracts/sdk-start.md](./contracts/sdk-start.md): public Rust types, operation, errors,
  side-effect contract, and usage.
- [quickstart.md](./quickstart.md): SDK usage, host prerequisites, failure behavior, and gates.

### Public SDK boundary

Extend the `MicroVmSdk` facade with:

- `start_microvm(&str) -> Result<MicroVmStartResult, SdkError>`.

Re-export `MicroVmStartResult` deliberately from `crates/sdk/src/lib.rs` with Rustdoc describing
the running-state invariants (socket inside the volume, socket-preferred control, PID as forced
termination fallback). The operation takes only the VM name; the SDK home comes from the
constructed `MicroVmSdk`, and the volume directory is read from the inventory record. Do not add
CLI commands or any other lifecycle operation in this feature.

The result exposes the stable VM name, `Running` state, volume directory, VM-local rootfs path,
volume-local socket path, background process ID (`u32`), persisted network metadata, and
`root:22` SSH metadata with the private-key path only. It never exposes key contents or an active
process handle.

### Start coordinator

Implement the start flow in `manager.rs` (or a focused private manager module) in this order:

1. Validate the VM name with the existing `validate_vm_name` helper before touching host state.
2. Acquire the per-name target lock (`{home}/vms/{name}`), load the record with `find_microvm`,
   return `NotFound` for unknown names, then acquire the volume lock when the persisted volume
   differs from the name-lock path (same pattern as `configure_network`).
3. Reject `Creating` rows with `LifecycleConflict`; only `Configured` and `Running` proceed.
4. Validate persisted machine data with a start-specific check (volume is a real directory;
   rootfs and socket paths remain inside the volume; rootfs present with the recorded size;
   credential paths, modes, and SSH metadata match the contract; runtime/kernel/binary paths
   resolve to verified inventory files). This check must not reuse the creation assertion that the
   runtime is stopped and no socket file exists, because stale leftovers are handled by liveness
   instead.
5. Run live checks through the runtime port: verified process liveness plus an active socket
   connection. If both agree the VM is running, return the current running identity without
   launching anything. If either signal disagrees, treat the VM as stopped, clear or replace the
   stale runtime references, and continue. Never decide from stored state alone.
6. Run `runtime.validate_host()` (KVM) and reconcile the persisted network via
   `network.configure` with the stored network as `existing`, passing the same used-address lists
   as creation; persist the reconciled network with `update_network`. Keep correct items, repair
   only missing or stale ones, keep the persisted identity unchanged.
7. Remove a stale leftover socket file at exactly `{volume}/firecracker.sock` only after the live
   checks proved no listener; never delete any other file.
8. Launch through the runtime port with the full persisted configuration and wait for readiness
   with a bounded poll. On readiness, atomically persist the runtime record (PID, `"running"`,
   volume-local socket) and transition the VM row to `Running` in one repository closure, then
   return the running identity.
9. On launch failure, kill only the just-spawned child, remove the owned socket only if this
   attempt created it, reset runtime refs to stopped without claiming `Running`, keep repaired
   network in place, and return the typed launch error.

The coordinator must never use in-memory state as the source of truth. A fresh SDK instance must
recover the volume, configuration, network identity, and runtime refs from SQLite, then re-derive
running-versus-stopped from live host evidence.

### Liveness and probe design

Extend `ports::runtime::RuntimeController` (crate-internal, so no public breakage) with:

- a process check that returns live only when `/proc/<pid>` exists and the recorded command line
  still references this VM (socket path or runtime binary path); an unreadable or mismatched
  command line is not-live, and the operation never kills the process on this evidence alone;
- a socket check that connects to the volume-local socket over UDS and performs a minimal
  Firecracker API read (for example `GET /machine-config`); a refused or timed-out connection is
  not-answering, while file existence alone never counts;
- a detached launch that returns the child PID and a readiness wait bounded in time.

Running requires both signals to agree. Every mismatch (dead process, silent socket, PID reuse,
orphaned socket file) resolves to stopped-plus-stale-refs, and the next step is a fresh launch,
never a duplicate launch beside a live machine.

### Launch and readiness design

The `FirecrackerRuntime` adapter builds `firectl` argv from already-validated persisted values:
the persisted Firecracker and `firectl` paths, the kernel path resolved from the record's
`kernel_id` through the artifact inventory, the VM-local `rootfs.ext4` as the writable root
drive, vCPU count, checked effective memory MiB, the persisted TAP name and guest MAC, the
persisted `desired_boot_parameters` combined with the distribution boot arguments, and
`{volume}/firecracker.sock` as the socket path.

The child is spawned detached: stdin is null, stdout/stderr append to a VM-local log file inside
the volume directory, the handle uses `kill_on_drop(false)` and is forgotten after the PID is
recorded, so the caller regains control immediately with no session held open. Readiness polls the
socket until it answers or the bound expires. Only the just-spawned PID may be terminated on
failure; a recorded PID from a previous run is never signalled, which closes the PID-reuse kill
hazard.

### Network repair reuse

No network adapter changes are needed. Start calls `configure` exactly like `configure_network`
does: the persisted network is passed as `existing`, host-only and LAN used-address lists are
loaded from the repository, correct resources return as `skipped`, and missing or SDK-owned stale
resources return as `applied`. Mode switches are impossible because the request mode is derived
from the record's `expose_on_lan` and validated against the persisted mode first. Unrepairable
network returns the adapter's typed `Network` error with the VM left `Configured` and no process
launched; repaired items stay persisted so the next retry resumes from correct state.

### Persistence and transaction boundaries

No migration is added. The existing repository methods cover every write:

- `find_microvm` for lookup by name;
- `update_network` for the reconciled attachment;
- one closure running `persist_runtime` plus `update_state` for the `Running` commit;
- one closure resetting runtime refs plus `update_state(Configured)` when recovering stale state
  or cleaning up a failed launch.

SQLite transactions cover each durable transition but are never held open while running external
host commands or polling the socket. The manager writes nothing claiming `Running` before the
socket answers, and `configure_network`'s `Configured`-only gate keeps working because start is
the sole writer of the durable `Running` state.

### Error and side-effect design

Reuse the existing `SdkError` surface with no new public variant expected:

| Situation | Variant |
|---|---|
| Empty or malformed VM name | `InvalidRequest` with field and reason; no host mutation |
| Unknown VM name | `NotFound` for kind `MicroVM`; no host mutation |
| Stored state is `Creating` | `LifecycleConflict` with name, state, and operation |
| Volume, rootfs, key, or boot-artifact problem | `StorageConflict`, `Filesystem`, `GuestFilesystem`, `Credential`, or `ArtifactPrerequisite` naming the path |
| Network cannot be inspected or repaired | `Network` with mode, operation, resource, and reason; VM stays stopped |
| Host cannot run VMs | `RuntimeIncompatible` or `HostCommand`; no launch attempted |
| Launch, readiness, or stale-socket handling fails | `TemporaryRuntime` with component, reason, and stopped flag; no orphan process |
| Launch fails after repair | Primary launch error; repaired network stays; VM stays `Configured` |

A stale leftover socket is removed only at the exact persisted socket path after liveness proves
no listener. Caller-owned data, source artifacts, foreign network resources, and other VM records
are preserved. The operation never kills a previously recorded PID.

### Test implementation

Add or update SDK tests for:

- starting a `Configured` VM to `Running` with the socket inside the volume and a recorded PID,
  applying the full persisted configuration;
- repeated start while live returning the same identity with exactly one process;
- externally killed process detected as stale and started fresh;
- process/socket mismatch (each direction) and PID-reuse treated as stale, never duplicated;
- wiped host network repaired per mode with identity unchanged, correct items skipped;
- `Creating` rejected, unknown names not found, invalid names rejected before mutation;
- launch failure leaving the VM stopped with no orphan process and repaired network kept;
- concurrent same-name starts producing one process and one observed identity;
- silent operation: no stdout/stderr, no panic, typed errors on every expected path.

Use injected fake network and runtime ports to make liveness, repair, launch, and readiness
deterministic. Keep real KVM, process, socket, and network-administration tests capability-gated
so ordinary `cargo test` stays useful without privileges.

## Implementation Sequence

The dependency-ordered implementation sequence for `$speckit-tasks` is:

1. Add the `MicroVmStartResult` domain type with Rustdoc and the `lib.rs` re-export.
2. Extend the crate-internal `RuntimeController` with liveness, socket-probe, detached-launch,
   and readiness operations; update existing test doubles.
3. Implement the `FirecrackerRuntime` liveness check, socket probe, `firectl` argv builder,
   detached spawn with VM-local logs, and bounded readiness wait.
4. Implement the manager's start coordinator: name validation, locks, lookup, `Creating` gate,
   start-specific persisted-data validation, and boot-artifact reverification.
5. Wire live-check-first idempotency, stale-ref recovery, and stale-socket removal.
6. Wire network reconciliation reuse plus `update_network` persistence.
7. Wire launch, readiness, and the atomic `persist_runtime` plus `update_state(Running)` commit;
   wire launch-failure cleanup that kills only the spawned child and keeps repaired network.
8. Add lifecycle, idempotency, stale-recovery, repair, concurrency, failure-path, and silence
   tests; add the contract, data-model, and quickstart artifacts.
9. Run the required quality gates and review the diff for the SDK/CLI boundary, secret handling,
   PID safety, socket placement, and English-only repository text.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| A half-alive VM (live process, dead socket or vice versa) is mistaken for running | Require both signals to agree; every mismatch is stale and starts fresh without duplicating. |
| PID reuse makes an unrelated process look like the VM | Verify the command line references this VM and never signal a previously recorded PID; only the just-spawned child may be terminated. |
| A stale socket file blocks launch or fakes liveness | Treat file existence as nothing; only an answering connection counts, and remove the exact stale path only after proving no listener. |
| Launch fails after network was repaired | Keep repaired items persisted, kill only the spawned child, stay `Configured`, and report the launch error for a clean retry. |
| Two concurrent starts launch two processes | Hold the per-name plus volume lock pair across check and launch so at most one child is spawned. |
| Claiming `Running` before the VM answers | Persist `Running` plus runtime refs only after the readiness poll succeeds; failures reset to stopped refs. |
| Booting a half-configured VM | Reject `Creating` rows before any host work; start-specific validation rechecks volume, rootfs, keys, and boot artifacts. |

## Post-Design Constitution Check

All gates remain PASS after Phase 1:

- The feature is implemented once in the SDK and exposed through one typed operation; the CLI is
  untouched.
- Domain records and orchestration do not depend on processes, sockets, SQLite, Linux commands,
  or terminal output; each dependency sits behind a port/adapter.
- SQLite remains the durable host-local source of truth, reconciled with live process and socket
  evidence on every call; no migration is needed.
- The design supports independent VMs, repair-only network reconciliation, single-process
  concurrency, PID-reuse safety, and failure cleanup without global state or destructive behavior.
- Public contracts, Rustdoc, tests, and quickstart documentation are planned in English and
  preserve the existing project organization.
