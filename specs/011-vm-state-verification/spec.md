# Feature Specification: VM State Verification

**Feature Branch**: `011-vm-state-verification`

**Created**: 2026-09-20

**Status**: Draft

**Input**: User description: "Machine state (running, stopped, configured) is stored in the database, but the database is static while the real state is not: a machine can die from a boot failure, a manual kill, or any cause outside the CLI, leaving the CLI state permanently stale. Everywhere the machine state is displayed must now verify the real state against the `.sock` file: whether any process is bound to the socket, combined with the recorded PID — never trusting the PID alone, since a PID can be recycled or the machine restarted manually. The verification must be smart and performant: listing many machines (e.g. 100) must check states concurrently, never one after another."

## Clarifications

### Session 2026-09-20

- Q: When the control socket answers but the recorded process ID no longer references this machine (e.g. manual restart outside the CLI), what is the verified state? → A: Running — socket governs.
- Q: When live verification finds a machine is not running, what state should state surfaces display for it? → A: Preserve persisted non-running state (superseded by later answers — no persisted state column remains; a silent socket always reports stopped).
- Q: When one machine's liveness probe fails or times out during a bulk listing, how should that entry behave? → A: Custom — no persisted status column exists (migration removes it); state comes only from live verification, and a probe failure means stopped.
- Q: After the persisted state column is removed, which states can a machine report and how is creation progress expressed? → A: Running or stopped only — creation progress is not a state; incomplete creations fail or are retried.
- Q: What bound should bulk state verification use when checking many machines at once? → A: CPU-scaled cap — parallelism scales with available CPU cores up to a ceiling.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Truthful single-machine state (Priority: P1)

An operator checks one machine (status view, inspection, or as a pre-step of start/ssh) after the machine died outside the CLI (manual kill, boot failure, host reboot). The displayed state reflects reality — stopped — instead of the stale persisted `running` marker, and follow-up operations (e.g. start) behave for a stopped machine rather than failing against a phantom running one.

**Why this priority**: This is the reported bug. Every other improvement is worthless while a single-machine answer can be wrong.

**Independent Test**: Kill a running machine's process outside the CLI, then check its state through the CLI/SDK. It reports stopped, and `start` proceeds as for a stopped machine.

**Acceptance Scenarios**:
1. **Given** a machine whose process is gone (killed outside the CLI, boot failure, host reboot), **When** the operator checks its state, **Then** the reported state is stopped, not running.
2. **Given** a machine whose control socket no longer answers, **When** the operator checks its state, **Then** the reported state is stopped even if a process with the recorded PID still exists.
3. **Given** a machine that is genuinely running (its volume-local control socket answers), **When** the operator checks its state, **Then** the reported state is running.
4. **Given** a machine whose socket does not answer, **When** the operator checks its state, **Then** the reported state is stopped — there is no persisted label to preserve, collapse, or mark stale.

---

### User Story 2 - Fast bulk listing over many machines (Priority: P2)

An operator lists the inventory with many machines (tens to ~100). Each entry shows a live-verified state, and the whole listing still completes quickly because per-machine checks run concurrently rather than sequentially.

**Why this priority**: Verifying liveness per machine adds host probes (process inspection, socket connection) to what used to be a pure database read. Without concurrency, bulk listing regresses linearly and becomes unusable at scale.

**Independent Test**: Seed an inventory with many machines in mixed real states and time a full listing; it completes within the success-criteria bound with correct per-machine states.

**Acceptance Scenarios**:

1. **Given** an inventory of ~100 machines in mixed states, **When** the operator lists them, **Then** every entry shows the live-verified state and the listing completes within the time bound in SC-002.
2. **Given** a bulk listing where one machine's probe is slow or fails, **When** the listing runs, **Then** that entry reports stopped while the other machines still report their verified states correctly (never a panic or aborted listing).

---

### User Story 3 - Safe start over a stale runtime reference (Priority: P2)

An operator starts a machine whose runtime reference is stale (recorded PID/socket with nothing live behind it). The start flow detects the staleness through the same verification, treats the machine as stopped, cleans up the stale references, and launches normally — instead of refusing as "already running" or crashing into a phantom socket.

**Why this priority**: This is the most damaging consequence of stale state: it blocks recovery. The start path already does liveness probing; this story makes that behavior uniform and specified.

**Independent Test**: With a stale runtime reference and no live process, run start and observe a normal launch with fresh process identity.

**Acceptance Scenarios**:

1. **Given** a stale runtime reference with no live process/socket, **When** the operator starts the machine, **Then** the operation proceeds as a fresh start and ends with a genuinely running machine.
2. **Given** a genuinely running machine, **When** the operator starts it again, **Then** the operation is idempotent and returns the existing live machine without launching a duplicate.

---

### Edge Cases

- What happens when the recorded PID now belongs to an unrelated process (PID recycled)? The process check must confirm the process actually references this VM (command line referencing the VM's socket/binary), so a recycled PID alone never reports running.
- What happens when the socket file exists but nothing answers on it (stale socket after a crash)? File existence alone never counts as running; only an answering control connection does.
- What happens when the machine was started manually outside the CLI/SDK (live socket, unknown or different PID)? The socket probe governs: an answering socket for the VM's socket path means running, regardless of whether the PID matches the record.
- What happens when the process references the VM but the socket does not answer (mid-boot, hung)? The machine reports stopped; there is no separate starting or degraded state.
- What happens when probes lack permission (e.g. unreadable `/proc` entry, socket connect denied)? The entry resolves to stopped per FR-007 — never a panic, never unsolicited output from the SDK.
- What happens when the inventory holds no runtime record at all for a machine (never started)? The machine reports stopped without any host probing.
- How does the system behave when one probe in a bulk check hangs? Per-machine probing must be bounded so a single stuck machine cannot stall the whole listing indefinitely.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Every SDK operation that reports machine state (single-machine status/inspection and bulk listing alike) MUST derive state at call time from host verification alone. No persisted lifecycle-state value may serve as the source of displayed state.
- **FR-002**: A machine MUST be reported as running if and only if its volume-local control socket answers. The recorded-process check is a false-positive guard: a live recorded PID that does not reference this VM MUST NOT count as running, and a referencing process whose socket does not answer MUST NOT count as running either.
- **FR-003**: A database migration MUST remove the persisted lifecycle-state (`microvms.state`) column, eliminating stale `running` markers on existing databases with it. After migration, displayed state comes only from live verification; the recorded PID and socket paths remain persisted solely as probe inputs, never as state.
- **FR-004**: The check MUST NOT trust the recorded PID alone: a live PID that does not reference this VM's socket/binary MUST NOT count as running (PID-reuse protection).
- **FR-005**: The check MUST NOT trust socket file existence alone: only an answering control connection counts, never the mere presence of the `.sock` path.
- **FR-006**: Bulk state reporting (listing many machines) MUST verify machines concurrently with parallelism that scales with available CPU cores up to a fixed ceiling (exact core mapping and ceiling decided in planning), so total time does not grow linearly with one sequential probe per machine.
- **FR-007**: A single machine's probe failure or timeout (missing process access, socket errors, bounded per-machine hangs) MUST resolve that entry to stopped. It MUST NOT abort a bulk listing, panic, or corrupt other entries.
- **FR-008**: Read-only state reporting MUST NOT mutate inventory, launch processes, or destroy state as a side effect; verification derives state without persisting it. Lifecycle transitions (start/stop/reboot) keep their existing write behavior for runtime references, including stale-reference cleanup on the start path.
- **FR-009**: Probe failures that decide state resolve to stopped on read paths (FR-007) rather than surfacing as errors. Any other failure that prevents answering at all (e.g. inventory access failures) MUST surface as a typed SDK error with enough context for the caller to decide; the SDK MUST NOT panic, terminate the process, write output, or emit logs per the panic-free, silent SDK boundary.
- **FR-010**: CLI surfaces that display state MUST render the verified state with the existing calm status vocabulary; a machine verified as stopped MUST NOT be presented as running anywhere (list, inspect, status, start/ssh pre-steps).

### Key Entities

- **Verified machine state**: The call-time answer (running vs. stopped) derived from host probes. There is no persisted lifecycle state: machine identity comes from inventory, and the recorded PID plus socket paths serve only as probe inputs, never as state.
- **Liveness evidence**: The two combined signals — control-socket responsiveness (decides running) and recorded-process VM reference (false-positive guard). An answering socket decides running even when the recorded PID is stale or foreign; a silent socket decides stopped even when a process references the VM.
- **Bulk verification result**: The per-machine collection of verified states for a listing call, where each entry is independent: one entry's probe outcome never changes another's.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: After killing a running machine's process outside the CLI, every state surface reports that machine as stopped within one check (no stale `running` displayed anywhere).
- **SC-002**: A listing over 100 machines completes in under 10 seconds on a typical host, with every entry showing its live-verified state.
- **SC-003**: A machine whose recorded PID has been recycled by an unrelated process is never reported as running (verified by a test that plants a live non-VM process under the recorded PID).
- **SC-004**: A stale socket file with nothing answering on it is never reported as running (verified by a test with a planted non-responsive socket file).
- **SC-005**: Zero panics, process terminations, or unsolicited SDK output occur during verification, including bulk listings with injected per-machine probe failures.

## Assumptions

- "Running" means the machine's volume-local control socket answers (socket governs); "stopped" means anything else. These are the only two reportable states: the `configured` / `creating` labels disappear with the removed column, and creation progress is never a state — an incomplete creation fails or is retried.
- Bulk concurrency scales with available CPU cores up to a fixed ceiling (exact core mapping and ceiling decided in planning), never one sequential probe per machine and never unbounded one-task-per-machine.
- A database migration removes the `microvms.state` column (including stale `running` markers on existing databases); a migration that merely ignores the column without removing it does not satisfy this feature. Read-only state reporting performs no persistence writes at all.
- Per-machine probe timeouts are bounded so one hung machine cannot stall a listing; exact timeout values are a planning decision.
- Multiple independent MicroVMs per host remain first-class: verification is always keyed per machine (own socket path, own recorded PID) with no single-VM assumption.
