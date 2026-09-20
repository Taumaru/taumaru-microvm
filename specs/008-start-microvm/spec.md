# Feature Specification: Start Configured MicroVM

**Feature Branch**: `008-start-microvm`

**Created**: 2026-09-20

**Status**: Draft

**Input**: User description: "Build the SDK operation that starts an already-created MicroVM. It receives the Taumaru home like the rest of the SDK, an optional machine volume path (defaulting to the standard folder resolution used during MicroVM creation), and the unique MicroVM name. The flow verifies the machine exists correctly, checks whether it is already running without trusting only static database info (the process may have been killed externally), verifies which host network items are still correctly configured (host settings can be lost on host reboot), starts the machine with its full configuration (network, size, and all machine settings), runs the launch process in the background so the caller does not need to keep the process open, records process info such as the PID for later termination while preferring the machine's control socket, and keeps the socket inside the machine data folder."
## Clarifications

### Session 2026-09-20

- Q: When the caller omits the volume path but the VM was created with a custom volume directory, which directory should start use? → A: The caller never passes a volume path; start receives only the SDK home and the VM name and always reads the volume directory from the inventory record.
- Q: Should starting a VM whose stored state is still Creating be rejected instead of booted? → A: Yes (Option A) — only Configured and Running states may start; a Creating-state VM is rejected with a typed lifecycle-conflict error.
- Q: When the VM is genuinely running, should a repeated start return success with the current running identity instead of an already-running error? → A: Yes (Option A) — repeated start returns success with the current running identity and launches nothing; "genuinely running" always requires live verification including an active control-socket connection, never the stored state alone, because the process may have been killed externally after the database was updated.
- Q: When only the recorded process is alive but the socket never answers (or vice versa), should start treat the machine as stopped? → A: Yes (Option A) — running requires both a live process and an answering control socket; any mismatch is stale, so the operation cleans up the stale references and starts fresh.
- Q: When the launch fails after host network items were already repaired, should start keep the repaired network and report the launch failure? → A: Yes (Option A) — repaired network items stay in place as correct state for the next retry; the operation only guarantees the VM stays stopped with no orphan machine process and no false running state.


## User Scenarios & Testing *(mandatory)*

### User Story 1 - Start a Stopped, Configured MicroVM (Priority: P1)

As an application developer, I want to start one already-created MicroVM by its unique name so that it becomes running with its full persisted configuration applied and I get back current runtime identity (control socket location and process reference) without keeping a foreground process open.

**Why this priority**: This is the core value of the feature: turning a configured-but-stopped machine into a running one with a single call, applying everything that was decided at creation time.

**Independent Test**: Create and configure one MicroVM (stopped, with persisted compute, disk, and network settings), call the start operation with the SDK home and the VM name only, and verify the VM reaches running state, its control socket answers inside the VM data folder, and the caller regains control immediately while the machine keeps running in the background.

**Acceptance Scenarios**:

1. **Given** a configured, stopped MicroVM with valid persisted settings, **When** the caller starts it by name, **Then** the operation reads the volume directory from the inventory record, applies the VM's full persisted configuration (compute size, memory size, writable root disk, boot image, and network attachment), launches the machine in the background, persists the running state with process and socket references, and returns running identity including the socket path inside the VM data folder.
2. **Given** a configured, stopped MicroVM created with a custom volume directory, **When** the caller starts it by name alone, **Then** the operation uses the stored volume directory from the inventory record and starts the machine without requiring any path input.
3. **Given** a configured, stopped MicroVM whose host network items are all still present and correct, **When** the caller starts it, **Then** the operation keeps the correct items untouched and starts the machine without recreating anything.
4. **Given** a start call that succeeds, **When** the caller continues execution immediately after return, **Then** the machine keeps running without requiring the caller to hold any process, window, or session open.

---

### User Story 2 - Repeated Start Is Safe and Sees Reality (Priority: P1)

As an application developer, I want a repeated start to be safe whether the machine is genuinely running or its process was killed behind the SDK's back, so that I never get duplicate machines and never get told "running" for a dead process.

**Why this priority**: The caller explicitly requires live verification instead of trusting stored state. Wrong answers here cause duplicate processes or phantom "running" machines, both of which corrupt later stop, reboot, and status behavior.

**Independent Test**: Start a VM, call start again while it is alive, then terminate the machine process outside the SDK and call start again; the second call returns the same running identity without a new process, and the third call detects the dead process and starts a fresh running machine.

**Acceptance Scenarios**:

1. **Given** a MicroVM that is genuinely running (live process and responsive control socket), **When** the caller starts it again, **Then** the operation returns the current running identity without launching a second machine process and without changing the VM's configuration.
2. **Given** a MicroVM whose stored state says running but whose process was terminated outside the SDK, **When** the caller starts it, **Then** the operation detects the stale state through live host checks (process liveness and control-socket responsiveness, not stored state alone), treats the VM as stopped, clears or replaces the stale runtime references, and starts a fresh running machine.
3. **Given** two concurrent start calls for the same stopped VM, **When** both execute, **Then** exactly one machine process results and both callers observe the same running VM identity (one starts it, the other observes the running result).

---

### User Story 3 - Start Repairs Host Network State Lost Since Configuration (Priority: P2)

As an application developer, I want start to repair host network setup that disappeared after configuration (for example after a host reboot) so that a configured VM remains startable without a separate manual repair step.

**Why this priority**: Host network items are host-owned and can vanish while the VM record stays valid. Without repair-on-start, every host reboot would strand all configured VMs.

**Independent Test**: Configure a VM, remove its host network items outside the SDK (simulating a host reboot), call start, and verify the operation restores only the missing or stale items, keeps correct items untouched, and then starts the machine with working host connectivity.

**Acceptance Scenarios**:

1. **Given** a configured, stopped VM whose host network items are partially missing or stale, **When** the caller starts it, **Then** the operation recreates only the missing or stale items, preserves every still-correct item, and starts the machine with the VM's persisted network identity (addresses and attachment) unchanged.
2. **Given** a configured, stopped VM whose host network items cannot be recreated (for example no usable uplink for LAN mode), **When** the caller starts it, **Then** the operation returns a typed, actionable error, leaves no duplicate or half-started machine process behind, and keeps the VM in the stopped state.
3. **Given** a VM configured for LAN exposure and one configured for host-only isolation, **When** each is started after its host items were wiped, **Then** each is repaired according to its own persisted network mode without changing the mode or addresses selected at creation.

---

### Edge Cases

- What happens when the VM's stored state is still Creating (interrupted setup)? The operation rejects the start with a typed lifecycle-conflict error and does not boot a half-configured machine.
- What happens when the volume directory, writable root disk, or boot artifacts referenced by the record are missing or unusable? The operation returns a typed prerequisite error identifying what is missing instead of starting a broken machine.
- What happens when a control socket file already exists in the VM data folder from a previous run? A live socket for the VM means already-running; a stale leftover socket must not be mistaken for a running machine and must be handled without deleting unrelated files.
- What happens when the recorded process identifier exists but belongs to an unrelated process, or the process is alive while the socket is unresponsive (and vice versa)? Live checks combine process and socket evidence so ambiguous half-states resolve to either clearly-running or clearly-stopped, never to a duplicate launch on top of a live machine.
- What happens when the launch itself fails after network repair succeeded? The VM stays stopped, no orphan machine process remains, persisted state does not claim running, repaired network items stay in place as correct state for the next retry, and the error explains the launch failure.
- What happens when the caller passes an invalid VM name? The operation rejects the request with a typed invalid-input error before touching host state.
- What happens when host virtualization support is unavailable? The operation fails with a typed host-compatibility error rather than attempting a launch that cannot run.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The start operation MUST accept the explicit SDK home directory and the unique MicroVM name from its caller; it MUST NOT accept a volume path input.
- **FR-002**: The operation MUST read the machine data folder from the persisted volume directory in the inventory record and use it for the volume directory, root disk, control socket, and other VM-exclusive files.
- **FR-003**: The operation MUST validate the VM name before changing host state, rejecting empty or malformed names with typed invalid-input errors.
- **FR-004**: The operation MUST verify the VM exists in the local inventory by name and return a typed not-found error without host changes when it does not.
- **FR-004a**: The operation MUST reject a VM whose stored lifecycle state is Creating with a typed lifecycle-conflict error; only VMs in Configured (stopped) or Running state may proceed to live running checks and launch.
- **FR-007**: The operation MUST determine whether the VM is already running by reconciling persisted lifecycle and runtime references against live host evidence; this MUST include both machine process liveness and an active control-socket connection attempt, and it MUST NOT decide from stored state alone.
- **FR-008**: When live checks show the VM is genuinely running — meaning both the recorded machine process is alive and the control socket answers — the operation MUST return the current running identity (including socket path and process reference) idempotently, without launching a second machine process and without altering the VM's persisted configuration.
- **FR-009**: When stored state claims running but live checks show any mismatch (process gone, socket silent, or either signal disagreeing), the operation MUST treat the VM as stopped, clear or replace the stale runtime references, and proceed with a fresh start.
- **FR-010**: The operation MUST inspect the VM's host network items before launching, keep every still-correct item untouched, recreate only missing or stale items, and apply the VM's persisted network identity (mode, addresses, attachment) unchanged.
- **FR-011**: When host network items cannot be inspected or repaired, the operation MUST return a typed network error, leave the VM stopped, and leave no duplicate or half-started machine process behind.
- **FR-012**: The launch MUST apply the VM's complete persisted configuration: virtual CPU count, memory size, writable root disk, boot image and kernel references, network attachment, and boot parameters.
- **FR-013**: The launch MUST run the machine process in the background (detached), so the caller regains control immediately after return and is not required to hold the process, session, or terminal open while the machine runs.
- **FR-014**: On success the operation MUST persist the running lifecycle state together with the runtime references needed for later control, including the process identifier and the control socket path; future control MUST prefer the control socket, with the process identifier available for forced termination.
- **FR-015**: The machine control socket MUST live inside the VM data (volume) folder alongside the VM's exclusive files; the operation MUST NOT place an active control socket anywhere else.
- **FR-016**: Concurrent start calls for the same VM MUST coordinate so that at most one machine process is launched and all callers observe the same running VM identity.
- **FR-017**: When the launch fails, the operation MUST leave the VM in the stopped state, MUST NOT leave an orphan machine process or claim running state, MUST keep already-repaired network items in place as correct state for the next retry, and MUST return a typed error explaining the failure.
- **FR-018**: All expected failure paths (unknown VM, invalid input, missing prerequisites, network repair failure, host incompatibility, launch failure) MUST be returned as typed errors; the operation MUST NOT panic, terminate the calling process, write to standard output or error, emit logs, or hide behavior in global side effects.

### Key Entities

- **Start request**: The caller's intent to run one machine: the unique VM name resolved against the explicit SDK home. The machine data folder always comes from the inventory record's persisted volume directory; the caller never supplies it.
- **Stored machine record**: The durable inventory entry for the VM: identity, persisted configuration (compute, memory, disk, boot references, network identity), volume and socket paths, and lifecycle state. It is the source of what "correct" means for existence and configuration checks.
- **Live running evidence**: Host-observed facts used to confirm a running machine: machine process liveness and control-socket responsiveness. Combined with the stored references, this evidence decides running versus stopped versus stale.
- **Host network attachment**: The VM's persisted network identity plus the host-side items that realize it (interface attachment, addresses, routes, forwarding entries as applicable to the VM's mode). Items are inspected per start and repaired only when missing or stale.
- **Running attachment**: The result of a successful start: running lifecycle state with the control socket path inside the VM data folder and the background process reference. The socket is the preferred control channel; the process reference supports forced termination.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A caller can start a correctly configured, stopped VM by name alone and receive running identity (control socket plus process reference) with the machine reachable through its control channel within 2 minutes on a capable host.
- **SC-002**: Every successful start places the active control socket inside that VM's data folder and records a process reference usable for later termination — 100% of successful starts, verified from returned identity and host state.
- **SC-003**: A second start call against a genuinely running VM returns the same running identity without launching an additional machine process — 100% of repeated-start trials show exactly one machine process.
- **SC-004**: A start call after the machine process was terminated outside the SDK detects the stale state and reaches running with a fresh process instead of reporting the dead machine as running — 100% of externally-killed trials recover on the next start.
- **SC-005**: A start call after the VM's host network items were removed (simulated host reboot) restores connectivity and reaches running without manual repair and without changing the VM's persisted network identity — 100% of wiped-network trials for each supported network mode.
- **SC-006**: Failed starts (unknown VM, missing machine data, unrepairable network, failed launch) return actionable typed errors with no duplicate or orphan machine processes and no false running state — 100% of failure trials leave the VM stopped and accurately reported.
- **SC-007**: Callers regain control immediately after a successful start while the machine keeps running with no open session required — 100% of successful trials show the machine still running after the calling context ends.

## Assumptions

- The VM was previously created and configured by the SDK, so a complete persisted record (configuration, volume association, network identity, boot references) already exists; initial creation and configuration are out of scope.
- The volume directory always comes from the inventory record's persisted volume, whether the VM was created with the managed default or a custom directory; start takes no volume-path input and never re-derives a default at start time.
- An already-running VM makes start idempotent (return current running identity) rather than an error, consistent with the project's rule that operations are idempotent whenever their semantics allow repetition.
- A successful start returns once the machine process is running in the background and its control channel inside the VM data folder is responsive; full guest-OS boot and guest login readiness are out of scope and remain the concern of status and connection operations.
- Host reboots can delete host-side network items (interfaces, routes, forwarding entries) while VM data folders and the local inventory survive; repair recreates host items only and never changes the VM's persisted network identity or mode.
- The control socket is the preferred channel for later stop, reboot, and status control; the persisted process identifier exists as a fallback for forced termination of an unresponsive machine.
- Stopping, rebooting, deleting, listing, inspecting, guest readiness checks, and any command-line presentation of this operation are out of scope; this feature covers the SDK start operation only.
- The SDK home is provided explicitly by the caller; the SDK does not discover it from environment variables. The machine data folder is read from the inventory record, not resolved from the home at start time.
