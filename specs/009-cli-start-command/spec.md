# Feature Specification: CLI `start` Command for Launching a MicroVM

**Feature Branch**: `009-cli-start-command`

**Created**: 2026-09-20

**Status**: Draft

**Input**: User description: "Develop the start command for the CLI: invoked as `microvm start` or `microvm start {machine_name}`; without a name it shows a selector with all machines created on the PC. It calls the privilege function with sudo (like the `new` command) when not run as root, then calls the SDK `start_microvm` with the selected VM. After starting, it shows how to connect (`microvm ssh {name}` or the full ssh command with sudo/root prefix), LAN access guidance when the machine has LAN access (copy the private ssh file plus the command), and how to stop (`microvm stop {name}`) — those commands are documentation-only hints, not implemented here. Also update the post-creation log of the `new` command (which already shows machine info) with a new line showing the command to start the machine. The command must be beautiful, modern, follow the visual standard of existing commands, and support a non-interactive mode where passing the machine name is mandatory and already running with root access is mandatory. All root-access behavior reuses the existing CLI privilege function — no reinvention."

## Clarifications

### Session 2026-09-20

- Q: In interactive mode, should `microvm start` launch the machine immediately once the name is known, or ask for a yes/no confirmation first? → A: Start immediately with no confirmation prompt; elevation (when needed) is the only gate before the SDK start call.
- Q: While the machine is starting, what should the command display before the running report appears? → A: Show a live spinner or status line while starting, then the running report.
- Q: When the named machine does not exist locally, should the command just report "not found", or also offer to create it now? → A: Report not found with a pointer to `microvm new`; start nothing and offer no creation shortcut.
- Q: When the typed or selected machine name is invalid (empty, malformed, not path-friendly), should the command abort immediately or re-prompt for a corrected name? → A: Abort immediately with the naming rule restated; no re-prompt and nothing started.
- Q: When started machine output should list ssh and stop hints, in which order should the next-step lines appear? → A: Connect hints first (`microvm ssh`, then direct ssh, then the LAN paragraph when present), with `microvm stop` last.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Interactive Start With Machine Selector (Priority: P1)

As a host operator, I want to run `microvm start` and pick one of my created machines from a selector (or pass its name directly) so that I can launch a configured MicroVM without memorizing exact names.

**Why this priority**: This is the core value of the feature: turning a configured-but-stopped machine into a running one through a guided, beautiful flow consistent with the `new` and download commands.

**Independent Test**: Run `microvm start` with no arguments in an interactive terminal against a host with several created VMs, select one with the keyboard, approve elevation when asked, and verify the machine reaches running state and the success output names the started VM.

**Acceptance Scenarios**:

1. **Given** the operator runs `microvm start "web-01"`, **When** the flow begins, **Then** it skips the machine selector and uses `web-01` directly.
2. **Given** the operator runs bare `microvm start`, **When** the flow begins, **Then** it presents a single-select list of all locally created machines with keyboard navigation, confirmation, and cancellation.
3. **Given** a machine is established (typed or selected), **When** the command proceeds, **Then** it requests elevated access through the existing privilege flow when not already running as root, and starts the machine immediately through the SDK start operation with the selected name only and no intermediate confirmation prompt.
4. **Given** the operator cancels the selector or declines elevation, **When** the command exits, **Then** no machine is started and the command reports that the operation was cancelled.

---

### User Story 2 - Non-Interactive Start for Scripts (Priority: P1)

As an automation operator, I want to start a machine with a fully explicit command and no prompts so that scripts get deterministic behavior and fail fast when requirements are missing.

**Why this priority**: The CLI is an operational tool. Scripted start must be possible, and missing values or missing root access in scripts must fail fast with guidance instead of hanging on a prompt.

**Independent Test**: Invoke the command without a terminal as root with an explicit machine name, then repeat without a name and without root access. Verify the first run starts with no prompts and deterministic output, while the other runs error before any mutation and explain what is missing.

**Acceptance Scenarios**:

1. **Given** a complete explicit invocation (`microvm start "web-01" --non-interactive`) running with root access, **When** the command runs, **Then** it performs no prompts, starts the machine, and exits with a deterministic status and concise summary.
2. **Given** the name is missing in non-interactive mode, **When** the command validates input, **Then** it reports the missing name with usage guidance and starts nothing.
3. **Given** the command runs in non-interactive mode without root access, **When** it starts, **Then** it errors before any mutation and explains how to rerun with root access instead of prompting for elevation.

---

### User Story 3 - Next-Step Connection Guidance (Priority: P2)

As a host operator who just started a machine, I want the success output to tell me exactly how to connect to it and how to stop it so that I do not need to look up connection details elsewhere.

**Why this priority**: The start result carries everything needed (account, port, address, key path, network mode). Surfacing it as copyable next steps closes the loop; the referenced `ssh` and `stop` commands themselves are out of scope and appear as documentation-only hints.

**Independent Test**: Start a host-only VM and a LAN-exposed VM, and verify each success output shows the documented connection hints appropriate to its network mode plus the stop hint, with no attempt to implement or invoke the hinted commands.

**Acceptance Scenarios**:

1. **Given** a machine started successfully, **When** the command reports success, **Then** it shows the future `microvm ssh {name}` hint, the direct ssh invocation derived from the returned connection details (account, port, address, private-key path — never key contents), and the `microvm stop {name}` hint.
2. **Given** the start ran with elevation, **When** the direct ssh hint is shown, **Then** it carries the same elevation prefix used for the start so the copied command works as shown.
3. **Given** the started machine is LAN-exposed, **When** the command reports success, **Then** it additionally explains that the machine is reachable from another machine by copying the private key file there and shows the corresponding command; host-only machines show no LAN paragraph.

---

### User Story 4 - `new` Success Output Points to `start` (Priority: P2)

As a host operator who just created a machine, I want the creation summary to tell me how to start it so that the path from creation to a running machine is obvious.

**Why this priority**: Creation ends with a stopped machine. A single start hint line completes the journey without changing any creation behavior.

**Independent Test**: Create a machine through `microvm new` and verify the success summary keeps all existing information and adds the command to start that machine.

**Acceptance Scenarios**:

1. **Given** a machine was just created, **When** the `new` command prints its success summary, **Then** the summary includes the command to start that exact machine alongside the existing identity, network, volume, resource, and connection details.

---

### Edge Cases

- The name is empty, malformed, or not path-friendly; the command aborts immediately before any mutation with the naming rule restated and no re-prompt.
- A positional name and `--name` disagree; the command reports the mismatch and starts nothing.
- No machines exist locally when the selector opens; the command reports that there is nothing to start and suggests creating one first.
- The named machine does not exist; the command reports it as not found with the affected name, points to `microvm new` without offering a creation shortcut, and starts nothing.
- The machine is already running; the command reports it as running with the same connection hints and launches no second machine.
- The machine is still being created (interrupted setup); the command rejects the start as a lifecycle conflict and boots nothing.
- The machine's stored files or boot artifacts are missing, or host network repair fails; the command reports the typed failure calmly and leaves the machine stopped.
- The operator interrupts during the selector or elevation; the command reports cancellation with no side effects and the standard interrupt exit status.
- The terminal has no color support, is narrow, or cannot receive interactive input; selector focus, selection, and success/failure status remain distinguishable through text and symbols.
- The host-local base directory is unavailable or unwritable; the command explains the affected path and stops safely.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The CLI MUST expose the command as `microvm start` with an optional positional machine name and an equivalent `--name` flag. A bare `microvm start` MUST open the machine selector; `microvm start "some-name"` MUST skip the selector. When both the positional name and `--name` are supplied they MUST agree, otherwise the command MUST report the mismatch and start nothing.
- **FR-003**: In an interactive terminal without a name, the command MUST present a single-select step listing every locally created machine with keyboard navigation, confirmation, and cancellation. Cancellation or an empty local inventory MUST exit without starting anything and MUST explain the outcome (cancelled, or nothing to start with a pointer to creation).
- **FR-004**: When not already running with root access, the command MUST request elevation through the existing CLI privilege flow (same behavior as the `new` command: prompt-driven escalation in interactive terminals). The command MUST NOT implement its own escalation mechanism or rewrite privilege behavior.
- **FR-005**: In non-interactive mode (`--non-interactive`) the command MUST perform no prompts of any kind: the machine name is required (positional or `--name`), root access is required up front, and any missing requirement MUST fail before any mutation with usage guidance. Elevation prompting MUST NOT occur in non-interactive mode.
- **FR-006**: After name resolution and elevation, the command MUST immediately invoke the single SDK start operation with the selected machine name and with no intermediate confirmation prompt, and MUST NOT implement lifecycle behavior, process management, or network repair itself.
- **FR-007**: On success the command MUST report the machine as running and show copyable next steps in this order: the `microvm ssh {name}` hint, then the direct ssh invocation derived from the returned connection details (account, port, address, private-key path — never key contents; prefixed with the elevation prefix when the start ran elevated), then the LAN remote-access paragraph when the returned network mode is LAN-exposed (copy the private key file to the other machine plus the corresponding command; host-only machines show no LAN paragraph), and finally the `microvm stop {name}` hint.
- **FR-009**: Expected failures (unknown machine, invalid name, lifecycle conflict, missing prerequisites, network or launch failure) MUST explain what happened, why the machine was not started, and what the operator can do next, with a nonzero exit status and no unexplained failure output. An unknown machine MUST point to `microvm new` without offering a creation shortcut. An invalid name MUST abort immediately with the naming rule restated and no re-prompt.
- **FR-010**: Starting an already-running machine MUST be reported as running with the same connection hints as a fresh start, without launching a second machine process.
- **FR-011**: The interactive presentation MUST follow the restrained neutral visual language used by the `new` and download flows: compact information density, clear hierarchy, semantic status emphasis, visible keyboard focus, progressive disclosure, immediate feedback, and calm failure states. State MUST NOT be communicated by color alone.
- **FR-012**: While the SDK start operation is in flight, the command MUST show a live spinner or status line consistent with the `new` and download flows, then replace it with the running report on success. The indicator MUST NOT invent progress values.
- **FR-013**: The `microvm ssh` and `microvm stop` commands referenced in hints are documentation-only and MUST NOT be implemented in this feature.

### Key Entities *(include if feature involves data)*

- **Start Request**: The single machine name to launch, however supplied (positional argument, `--name` flag, or interactive selector choice).
- **Machine Choice**: The operator-selected created machine together with its running identity returned by the start operation (control socket, process reference, network mode, connection details).
- **Connection Hints**: The copyable next steps shown after success: the future `microvm ssh {name}` form, the direct ssh invocation with account, port, address, and private-key path, the LAN remote-access explanation for LAN-exposed machines, and the `microvm stop {name}` form.
- **Creation Summary Addition**: The new start-command line added to the `new` command's success output, naming the created machine.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In usability tests with at least three created machines, an operator can run bare `microvm start`, select a machine using only the keyboard, approve elevation, and reach the running report with connection hints in under 60 seconds.
- **SC-002**: In 100% of non-interactive tests with an explicit name and root access, the command performs no prompts and produces a deterministic exit status and summary; in 100% of tests with the name omitted or root access missing, it reports the missing requirement and starts nothing.
- **SC-003**: In 100% of successful start tests, the success output shows the `microvm ssh {name}` hint, a direct ssh invocation matching the returned account, port, address, and key path, and the `microvm stop {name}` hint; LAN-exposed machines additionally show the key-copy remote-access explanation, and host-only machines show none.
- **SC-004**: In 100% of successful `new` tests, the creation summary includes the command to start the created machine alongside all previously shown details.
- **SC-005**: In 100% of failure-path tests (unknown machine, invalid name, lifecycle conflict, unrepairable prerequisites), the command returns a nonzero status, identifies the failure, leaves the machine stopped, and gives a concrete retry or repair action.
- **SC-006**: In 100% of repeated-start tests against an already-running machine, the command reports running with connection hints and exactly one machine process exists afterward.
- **SC-007**: In 100% of non-color and narrow-terminal tests, selector focus, selected machine, and success/failure status remain distinguishable through text or symbols.

## Assumptions

- The SDK start operation already implements name validation, existence checks, live running detection, network repair, background launch, and typed errors; this command reuses all of it and adds no lifecycle semantics.
- The selector lists machines from the existing local inventory capability; which SDK surface serves the listing is an implementation decision for planning.
- An already-running machine makes start report running with connection hints rather than an error, consistent with the project's rule that operations are idempotent whenever their semantics allow repetition.
- A successful start returns once the machine process is running in the background with a responsive control channel; full guest-OS boot readiness is out of scope.
- The elevation prefix shown on the direct ssh hint mirrors the backend actually used for the start (sudo or equivalent); when already root, no prefix is shown.
- The direct ssh hint is derived from the returned connection details as `ssh -i {private-key-path} -p {port} {user}@{address}`; the private key file itself is copied for remote access, never its contents printed.
- Stopping, rebooting, deleting, listing, inspecting, guest readiness, and the actual `ssh`/`stop` commands are out of scope; they appear only as documentation hints.
- All operator-facing text for this feature is English; localization is out of scope.
