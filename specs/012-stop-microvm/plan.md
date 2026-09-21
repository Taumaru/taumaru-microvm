# Implementation Plan: Stop Running MicroVM

**Branch**: `012-stop-microvm` | **Date**: 2026-09-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/012-stop-microvm/spec.md`

## Summary

Implement one SDK-only `stop_microvm(name)` operation that turns a running MicroVM into a stopped
one. The operation takes only the VM name (the SDK home comes from the existing `MicroVmSdk`
instance; the volume directory and runtime references are read from the inventory record), decides
running-versus-stopped from control-socket responsiveness alone, returns success with
`forced: false` when the socket is already silent, otherwise sends one graceful shutdown request
(`PUT /actions` with `SendCtrlAltDel`) through the volume-local control socket, waits 60 seconds
for the socket to go silent and the recorded process to stop referencing the VM, escalates to an
immediate SIGKILL of the re-verified recorded PID when the machine stays running, re-verifies the
stop, and persists the stopped runtime row. The result always reports `Stopped` plus a `forced`
flag. An undeliverable graceful request while the socket answers is a typed error with no forced
attempt. No CLI changes are part of this feature.

## Technical Context

**Language/Version**: Rust 2024 edition, using the repository's stable toolchain.

**Primary Dependencies**: Existing `tokio` (already enabled: `fs`, `io-util`, `macros`, `net`,
`process`, `rt`, `signal`, `sync`, `time`), `rusqlite` (bundled SQLite), `serde`/`serde_json`,
`thiserror`; no new crates. Host integration uses std `UnixStream` over UDS through the internal
runtime port plus the existing `kill -KILL` host-command convention; no shell is used. Process
reference checks read `/proc` via the standard library filesystem APIs only.

**Storage**: Existing SQLite inventory under the explicit SDK home; no schema migration is needed
(`vm_runtime` already stores `process_id` and `process_state`; stop rewrites the same stopped
shape start uses for stale cleanup). VM-local files stay in the persisted volume directory
(`firecracker.sock` control socket); stop creates no files and deletes only the owned stale socket
at exactly the persisted path after liveness proves no listener.

**Testing**: Existing SDK unit/integration suites (`crates/sdk/tests/`, manager tests with injected
ports) extended with deterministic stop tests using a fake runtime port (scripted socket answers,
delivery outcomes, process references, and exit behavior); capability-gated host tests for real
socket/process/signal behavior stay out of the default suite. Run the repository Cargo quality
gates.

**Target Platform**: Linux hosts with permission to connect to the volume-local control socket and
to signal the recorded machine process on the forced path. Hosts without socket or signal
permission return typed SDK errors.

**Project Type**: Reusable SDK library in `crates/sdk`; the CLI is not modified for this feature.

**Performance Goals**: Already-stopped returns after one socket probe with no wait. Graceful stop
returns as soon as the exit poll observes silence (200 ms granularity), well under the spec's
3-minute SC-005 budget. The forced path adds at most the 60-second exit wait plus a 10-second
post-kill re-verify. No background worker or unbounded wait is introduced.

**Constraints**: The SDK is silent and returns typed `Result` errors. It must not read home paths
from environment variables, accept any input beyond the VM name, signal any process except the
re-verified recorded PID of this VM, attempt forced termination when the graceful request was
undeliverable, claim stopped for a still-running machine, or touch other VMs, network attachments,
artifacts, or credentials. Multiple VMs on one host are first-class.

**Scale/Scope**: Multiple independent VMs per host; concurrent same-name stops serialize to one
shutdown sequence. This feature owns the SDK stop operation only; start, reboot, delete, list,
inspect, status, guest-readiness, and CLI presentation remain future work.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle behavior; CLI remains thin | PASS: design is SDK-only | PASS: no CLI source changes or duplicate orchestration |
| Public SDK is typed, silent, non-panicking, and side-effect explicit | PASS: typed errors and port boundaries required | PASS: no output/logging/global state; shutdown, wait, and SIGKILL are explicit |
| Domain is independent from infrastructure | PASS: domain/ports/adapters separation selected | PASS: socket message, wait loop, and signal stay behind the runtime port |
| SQLite is local source of truth | PASS: record and runtime refs reconciled with live socket evidence | PASS: no migration needed; stopped commit reuses existing helpers |
| Firecracker/firectl remain replaceable implementation details | PASS: runtime port extension selected | PASS: public API exposes no HTTP body, signal number, or socket type |
| Registry and artifact boundaries are explicit | PASS: no registry or artifact work in stop | PASS: stop touches neither inventory artifacts nor downloads |
| Multiple MicroVMs are supported | PASS: per-VM locks, paths, and process identity | PASS: per-name plus volume locks; no shared mutable stop state |
| Public contracts and compatibility changes are documented | PASS: contract/data-model artifacts planned | PASS: Rustdoc, contract, data-model, and quickstart are included |
| Project text is English | PASS | PASS |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/012-stop-microvm/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── sdk-stop.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── src/
│   │   ├── lib.rs              # Re-export MicroVmStopResult
│   │   ├── manager.rs          # stop_microvm coordinator
│   │   ├── error.rs            # Reuse typed variants (no new public variants expected)
│   │   ├── domain/
│   │   │   ├── microvm.rs      # MicroVmStopResult
│   │   │   └── lifecycle.rs    # Stopped already exists; reused unchanged
│   │   ├── ports/
│   │   │   ├── runtime.rs      # Extended RuntimeController: shutdown request, exit wait
│   │   │   └── repository.rs   # Reuse find/persist methods
│   │   └── adapters/
│   │       └── runtime/
│   │           └── firecracker.rs  # SendCtrlAltDel delivery, exit poll, SIGKILL reuse
│   └── tests/
│       ├── lifecycle.rs        # Stop lifecycle, idempotency, forced escalation
│       └── failure_paths.rs    # Stop failure paths
└── cli/
    └── (untouched by this feature)
```

**Structure Decision**: All behavior lives in the SDK crate behind the existing
domain/ports/adapters layout. The manager orchestrates; the runtime adapter owns socket and
process mechanics; SQLite access stays behind the existing repository trait. No new top-level
modules or CLI changes.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Graceful shutdown is `PUT /actions` with `{ "action_type": "SendCtrlAltDel" }` over the
   volume-local control socket; HTTP 204 means delivered. There is no halt/power-off action in the
   Firecracker API. Guest images are expected to carry the i8042/AT-keyboard drivers; without them
   the forced path is the backstop.
2. The 60-second exit wait polls the same probe pair start uses (socket silence plus recorded
   process no longer referencing the VM) on a 200 ms interval, implemented as `wait_for_stop in
   the `FirecrackerRuntime` adapter next to `wait_for_socket`.
3. Forced termination reuses the existing `terminate_spawned` SIGKILL implementation unchanged,
   fired only after `process_references_vm` re-confirms the recorded PID at escalation time,
   followed by a 10-second re-verify. Still running afterwards is a typed error.
4. An undeliverable graceful request while the socket answers is a typed error with no forced
   attempt; an already-silent socket at delivery time re-verifies to success with `forced: false`.
5. Concurrency reuses the existing per-name plus volume `target_lock` pair, so concurrent stops for
   one VM run exactly one shutdown sequence.
6. Persistence needs no migration: success reuses `reset_runtime_to_stopped` / `clear_stale_runtime`
   plus `remove_stale_socket` at exactly the persisted socket path.
7. Errors reuse existing `SdkError` variants with no new public variant expected.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): request shape, result record, state machine, ownership, and
  validation rules.
- [contracts/sdk-stop.md](./contracts/sdk-stop.md): public Rust types, operation, errors,
  side-effect contract, and usage.
- [quickstart.md](./quickstart.md): SDK usage, host prerequisites, failure behavior, and gates.

### Public SDK boundary

Extend the `MicroVmSdk` facade with:

- `stop_microvm(&str) -> Result<MicroVmStopResult, SdkError>`.

Add `MicroVmStopResult { name, state, socket_path, forced }` to `domain::microvm`, re-export it
deliberately from `crates/sdk/src/lib.rs` (and `domain::mod`) with Rustdoc describing the stopped
invariants (socket inside the volume and silent at return; `forced` true only when SIGKILL was
delivered). `state` is always `MicroVmState::Stopped` on success. The operation takes only the VM
name; the SDK home comes from the constructed `MicroVmSdk`. Do not add CLI commands or any other
lifecycle operation in this feature.

### Stop coordinator

Implement the stop flow in `manager.rs` (or a focused private manager module) in this order:

1. Validate the VM name with the existing `validate_vm_name` helper before touching host state.
2. Acquire the per-name target lock (`{home}/vms/{name}`), load the record with `find_microvm`,
   return `NotFound` for unknown names, then acquire the volume lock when the persisted volume
   differs from the name-lock path (same pattern as `start_microvm`).
3. Reject incomplete creations via `require_complete` with `LifecycleConflict` before any host
   work.
4. Probe liveness through the runtime port: `socket_answers` decides. Silent means already
   stopped: settle refs with `clear_stale_runtime` when needed and return
   `MicroVmStopResult { Stopped, forced: false }` with no shutdown and no signal.
5. When running, call the new `request_shutdown` port method once. If it reports already-stopped
   (socket went silent in the race window), re-verify silence and return success with
   `forced: false`. If delivery fails while the socket answers, return the typed error with no
   forced attempt and no state write.
6. On delivery, call the new `wait_for_stop` port method with the 60-second bound. If it reports
   exited, remove the owned stale socket only at exactly the persisted path after liveness proves
   no listener, commit the stopped runtime row with `reset_runtime_to_stopped`, and return
   `Stopped` with `forced: false`.
7. On wait expiry with the machine still running, require a usable recorded PID and re-verify with
   `process_references_vm` that it still references this VM (socket or runtime binary path).
   Without a usable or still-referencing PID, return a typed error and never signal. Otherwise
   call `terminate_spawned` (SIGKILL) on the recorded PID, re-verify with `wait_for_stop` up to
   10 seconds, and on success commit the stopped row and return `Stopped` with `forced: true`.
   Still running afterwards is a typed error that never claims stopped.
8. If the process exits naturally between wait expiry and SIGKILL, the re-verification observes
   the stopped machine and returns success with `forced: false`.

The coordinator must never use in-memory state as the source of truth. A fresh SDK instance must
recover the volume, socket path, and runtime refs from SQLite, then re-derive stopped from live
socket evidence.

### Shutdown and wait design

Extend `ports::runtime::RuntimeController` (crate-internal, so no public breakage) with:

- `request_shutdown(&self, socket_path: &Path) -> Result<bool, SdkError>`: connects over UDS and
  sends `PUT /actions` with `{ "action_type": "SendCtrlAltDel" }` using bounded read/write
  timeouts in the existing `socket_answers` style. Returns `Ok(true)` on HTTP 204, `Ok(false)`
  when the socket is already silent (connect reports not-found/refused/reset), and `Err` for any
  other delivery failure (write failure, read timeout, non-204 response).
- `wait_for_stop(&self, socket_path: &Path, process_id: Option<u32>, firecracker_path: &Path, deadline: Duration) -> Result<bool, SdkError>`:
  polls every 200 ms until the socket is silent AND the recorded process (when present) no longer
  references the VM, or the deadline expires. Returns `Ok(true)` on exit, `Ok(false)` on expiry.
  A `None` process identity waits on socket silence alone.

Update the `terminate_spawned` port documentation: it remains the single SIGKILL path, now also
used for the re-verified recorded PID on the stop escalation path (previously documented as
current-call spawns only). No behavior change to the function itself.

The `FirecrackerRuntime` adapter implements both new methods with std blocking I/O, matching the
existing `wait_for_socket` style; SQLite transactions are never held open across the shutdown
request or the wait loops.

### Persistence and transaction boundaries

No migration is added. The existing repository methods cover every write:

- `find_microvm` for lookup by name;
- `reset_runtime_to_stopped` (one closure running `persist_runtime` with `process_id: None`,
  `process_state: "stopped"`) for the stopped commit on every success path;
- `clear_stale_runtime` settle check for the already-stopped path (no write when refs are already
  settled);
- `remove_stale_socket` for the owned socket file only at exactly the persisted path after
  liveness proves no listener.

Each closure covers one durable transition only and is never held open across host commands,
signal delivery, or socket polling.

### Error and side-effect design

Reuse the existing `SdkError` surface with no new public variant expected:

| Situation | Variant |
|---|---|
| Empty or malformed VM name | `InvalidRequest` with field and reason; no host mutation |
| Unknown VM name | `NotFound` for kind `MicroVM`; no host mutation |
| Incomplete creation record | `LifecycleConflict` with name, state, and operation; no host mutation |
| Graceful request undeliverable while socket answers | `TemporaryRuntime` with component, reason, and stopped flag; no forced attempt |
| Still running with no usable or still-referencing PID | `TemporaryRuntime`; never signals an unrelated process, never reports success |
| SIGKILL delivery failure | `HostCommand` naming `kill`; never claims stopped for a live machine |
| Machine still running after SIGKILL re-verify | `TemporaryRuntime`; never claims stopped for a live machine |

The operation never signals a recycled PID belonging to an unrelated process, never removes any
file except the owned stale socket at exactly the persisted path, and never touches network,
artifact, credential, volume layout, or other VM records.

### Test implementation

Add or update SDK tests for:

- graceful stop: answering socket, delivered shutdown, exit within bound, `forced: false`,
  silent socket and settled refs afterwards;
- already-stopped idempotency across stopped, never-started, and externally-killed rows:
  success with `forced: false` and at most a ref-settling write;
- forced escalation: guest ignores request, 60-second expiry, SIGKILL to the verified PID,
  success with `forced: true`;
- undeliverable graceful request while answering: typed error, no signal, no state write;
- unforceable still-running machine (no usable PID): typed error, never success;
- PID-reuse safety: recorded PID referencing an unrelated process is never signaled;
- escalation race: natural exit between wait expiry and SIGKILL returns `forced: false`;
- still-running after SIGKILL: typed error, never claims stopped;
- unknown, malformed, and incomplete-record names: typed errors with no host mutation;
- concurrent same-name stops: one shutdown sequence, one consistent stopped result;
- operation silence: no stdout/stderr, logging, tracing, or process exit through injected fake
  runtime ports.
