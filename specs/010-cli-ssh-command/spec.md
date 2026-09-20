# Feature Specification: CLI `ssh` Command for Connecting to a MicroVM

**Feature Branch**: `010-cli-ssh-command`

**Created**: 2026-09-20

**Status**: Draft

**Input**: User description: "Develop the SSH command for the CLI. It runs as `microvm ssh {machine_name}` or `microvm ssh`; without a machine name it lets the user select a machine from all of their machines that are running. Check whether the SDK already has a function for listing running machines; if not, create a minimal function in the SDK that lists running machines and returns each one's SSH file path, and change nothing else in the SDK. Use that function for the selection. After selection, invoke the privilege request in the cases where it already applies. Then, already privileged, run as a child process the command to enter the machine via SSH, something like `ssh -i {host key path} root@{private ip}`, and that child process must inherit all stdio and everything else so the user has exactly the same terminal experience as a manual SSH session — inputs, outputs, and everything else must work correctly."

## Clarifications

### Session 2026-09-20

- Q: Should `microvm ssh` accept an optional remote command to run inside the guest instead of always opening an interactive shell? → A: Yes — an optional remote command after the machine name or after `--` runs inside the guest with its exit status propagated.
- Q: Should `microvm ssh` support scripted use with no prompts, failing fast when the name is missing or rights are insufficient? → A: Yes — scripted use (explicit name plus optional remote command, non-interactive) performs no prompts and fails fast like the start command.
- Q: When connecting to a machine whose host key is not yet known, how should `microvm ssh` handle host-key verification? → A: Default OpenSSH behavior — first connect prompts to accept the host key and remembers it, exactly like running ssh by hand.
- Q: Should `microvm ssh` mirror the start command's name and mode flags (`--name` alias plus `--non-interactive`)? → A: Yes — mirror the start flags exactly: positional name, `--name` alias with must-agree rule, and `--non-interactive` flag.
## User Scenarios & Testing *(mandatory)*

### User Story 1 - Connect to a Named Running Machine (Priority: P1)

As a host operator, I want to run `microvm ssh web-01` and land directly in that machine's shell so that reaching a known running machine takes one command with no menus.

**Why this priority**: The named form is the fastest path and the scriptable form (with an optional remote command and a non-interactive mode that performs no prompts). It delivers the full value of the feature on its own: from an established machine name to a working guest shell or a single remote command result.

**Independent Test**: With a machine running, run `microvm ssh web-01`, approve elevation when asked, and verify a guest shell opens for that exact machine and closes cleanly on exit; repeat scripted with an explicit name plus a remote command and no terminal, and verify no prompts occur and the remote exit status is returned.

**Acceptance Scenarios**:

1. **Given** machine `web-01` is running and the operator runs `microvm ssh web-01`, **When** the flow proceeds, **Then** it skips any machine selector, resolves `web-01` through the running-machines source, requests elevation through the existing privilege flow when not already privileged, and opens the guest session for `web-01`.
2. **Given** the named machine does not exist locally, **When** the command validates the name, **Then** it reports the machine as not found with the affected name, points to creation, and opens no session.
3. **Given** the named machine exists but is not running, **When** the command validates it, **Then** it reports the machine as not running with its current state, points to the start command for that machine, and opens no session.
4. **Given** scripted use with an explicit name plus an optional remote command and no interactive terminal, **When** the command runs, **Then** it performs no prompts of any kind, rights are required up front instead of prompting for elevation, and the remote exit status propagates on success while any missing requirement fails before any session with usage guidance.

---

### User Story 2 - Pick a Running Machine Interactively (Priority: P1)

As a host operator, I want to run bare `microvm ssh` and pick one of my running machines from a selector so that I do not need to memorize exact names.

**Why this priority**: Operators routinely run several machines. A selector restricted to running machines prevents the wasted round trip of picking a stopped machine and then being told it is not running.

**Independent Test**: Run bare `microvm ssh` with several machines in mixed states (running, stopped, being created), verify only running machines are offered, select one with the keyboard, approve elevation, and verify the guest session opens for the selected machine.

**Acceptance Scenarios**:

1. **Given** the operator runs bare `microvm ssh` in an interactive terminal, **When** the flow begins, **Then** it presents a single-select list containing only running machines, with keyboard navigation, confirmation, and cancellation.
2. **Given** no machine is currently running, **When** the selector source returns empty, **Then** the command reports that there is nothing to connect to, points to the start command, and opens no session.
3. **Given** the operator cancels the selector or declines elevation, **When** the command exits, **Then** no session is opened and the command reports that the operation was cancelled.

---

### User Story 3 - Native-Feeling Privileged Terminal Session (Priority: P2)

As a host operator inside the guest session, I want typing, output, terminal resizing, signals, and exit behavior to feel exactly like a manually invoked SSH session so that editors, pagers, and remote commands all work without surprises.

**Why this priority**: A session that mangles input, drops output, or swallows the remote exit status is worse than no command at all. Fidelity is the acceptance bar for the whole feature once machine resolution works.

**Independent Test**: Open a session via the command and via the equivalent manual SSH invocation, exercise interactive input, full-screen output, interrupt keys, and a remote command with a known exit status, and verify both behave identically.

**Acceptance Scenarios**:

1. **Given** a machine is resolved and elevation is satisfied, **When** the session launches, **Then** the child session inherits the full terminal input/output/error streams so keystrokes, echoed output, full-screen programs, and terminal resizing behave exactly as in a manual SSH session.
2. **Given** the guest command or shell exits with a status, **When** the session ends, **Then** the CLI exits with the corresponding session status rather than always reporting success.
3. **Given** the command is not running with sufficient rights, **When** the flow reaches the privilege step, **Then** it uses the existing CLI privilege flow (same cases and behavior as the established lifecycle commands) and implements no new escalation mechanism of its own.

---

### Edge Cases

- The name is empty, malformed, or not path-friendly; the command aborts immediately before any session with the naming rule restated.
- A positional name and `--name` disagree; the command reports the mismatch and opens nothing.
- The named machine does not exist; the command reports it as not found with the affected name and points to creation without offering a creation shortcut.
- In `--non-interactive` mode the machine name is missing or sufficient rights are absent; the command fails before any session with usage guidance and never prompts.
- The named or selected machine exists but is stopped or still being created; the command reports the not-running state and points to the start command for that machine.
- No machines are running when the selector opens; the command reports that there is nothing to connect to and suggests starting one first.
- The operator interrupts during the selector, elevation, or session establishment; establishment reports cancellation with no session opened and the standard interrupt exit status, while an interrupt inside an established session is delivered to the remote side.
- The SSH key file or guest address backing the session is missing or unreadable; the command reports the typed failure calmly and opens no session.
- The SSH client program is unavailable on the host; the command explains what is missing and how to fix it instead of failing silently.
- The terminal has no color support, is narrow, or cannot receive interactive input; selector focus, selection, and failure status remain distinguishable through text and symbols. Opening an interactive shell requires an interactive terminal, while scripted use (explicit name plus remote command) performs no prompts and may run without one.
- The host-local base directory is unavailable or unwritable; the command explains the affected path and stops safely.
- The session itself never mutates VM lifecycle state: connecting, disconnecting, or failing to connect leaves creation, start, stop, and inventory state unchanged.

## Requirements *(mandatory)*

### Functional Requirements
- **FR-001**: The CLI MUST expose the command as `microvm ssh` with an optional positional machine name, an equivalent `--name` flag, a `--non-interactive` flag, and zero or more trailing remote-command words after the name (a `--` separator MUST be supported to disambiguate the remote command from flags). A bare `microvm ssh` MUST open the running-machine selector; `microvm ssh web-01` MUST skip the selector and use `web-01` directly. When both the positional name and `--name` are supplied they MUST agree, otherwise the command MUST report the mismatch and open nothing. Trailing words supplied without a machine name (after `--`) MUST be treated as the remote command to run on the interactively selected machine.
- **FR-002**: In an interactive terminal without a name, the command MUST present a single-select step listing only running machines with keyboard navigation, confirmation, and cancellation. Cancellation or an empty running set MUST exit without opening any session and MUST explain the outcome (cancelled, or nothing running with a pointer to the start command).
- **FR-003**: The SDK MUST provide a minimal running-machines listing used by the selector. It MUST return one entry per running machine carrying at least the machine name and the SSH connection material needed to connect (host private-key path, guest account, guest address, and port). It MUST report actually-running machines under the same liveness notion the start operation uses; the persisted inventory snapshot alone, which is never live-verified, does not satisfy this requirement. Which existing liveness checks are reused is an implementation decision for planning.
- **FR-004**: The SDK change for this feature MUST be strictly additive and minimal: one running-machines listing capability plus its supporting types and errors. Existing SDK operations, signatures, behaviors, persistence layout, and public contracts MUST remain unchanged.
- **FR-005**: An explicitly named machine MUST be resolved through the running-machines source, not through the all-machines inventory. An unknown name MUST be reported as not found with a pointer to creation; a known-but-not-running machine MUST be reported as not running with its current state and a pointer to the start command for that machine. An invalid name MUST abort immediately with the naming rule restated and no re-prompt.
- **FR-006**: When not already running with sufficient rights, the command MUST request elevation through the existing CLI privilege flow with the same cases and behavior as the established lifecycle commands. The command MUST NOT implement its own escalation mechanism or rewrite privilege behavior.
- **FR-006a**: In scripted use (explicit machine name with no interactive terminal), the command MUST perform no prompts of any kind: the machine name is required, sufficient rights are required up front, and any missing requirement MUST fail before any session with usage guidance. Elevation prompting MUST NOT occur in scripted use.
- **FR-007**: After machine resolution and elevation, the command MUST launch the session as a child SSH invocation built from the returned connection details in the form `ssh -i {private-key-path} {user}@{address}`, carrying the returned port when it differs from the SSH default and appending the trailing remote-command words verbatim when supplied (no shell opens in that case; the remote command runs instead). The command MUST reference the private-key file by path and MUST never print key contents. Host-key verification MUST keep default OpenSSH behavior (prompt on first connect and remember accepted keys); the command MUST NOT add options that weaken or bypass it.
- **FR-008**: The child session MUST inherit the full terminal input, output, and error streams (plus terminal size and signal delivery) so the operator experience — keystrokes, echoed and full-screen output, interrupt and resize behavior — is indistinguishable from running the equivalent SSH invocation manually.
- **FR-009**: The remote session status MUST propagate: when the guest shell or remote command ends, the CLI MUST exit with the corresponding status. Interrupts during selector, elevation, or session establishment MUST report cancellation with no session opened and the standard interrupt exit status; once the session is established, terminal control belongs to the session.
- **FR-010**: Expected failures (unknown machine, invalid name, machine not running, missing or unreadable key material, unavailable SSH client, failed connection) MUST explain what happened, why no session was opened, and what the operator can do next, with a nonzero exit status and no unexplained output.
- **FR-011**: The interactive presentation MUST follow the restrained neutral visual language used by the established flows: compact information density, clear hierarchy, semantic status emphasis, visible keyboard focus, progressive disclosure, immediate feedback, and calm failure states. State MUST NOT be communicated by color alone.
- **FR-012**: Opening a session MUST NOT change lifecycle or inventory state: it creates, configures, starts, stops, or deletes nothing and performs no network repair.

### Key Entities *(include if feature involves data)*

- **SSH Request**: The single machine to connect to, supplied either as the positional name or as the interactive selector choice, plus zero or more trailing remote-command words to run inside the guest instead of opening a shell.
- **Running Machine Entry**: One running machine from the minimal SDK listing: its stable name plus the SSH connection material (host private-key path, guest account, guest address, port) needed to build the session invocation.
- **Session Handoff**: The privileged child SSH invocation (`ssh -i {private-key-path} {user}@{address}`, plus port when non-default, plus the trailing remote-command words verbatim when supplied) executed with fully inherited terminal streams and propagated exit status.
- **Connection Guidance**: The failure and empty-state pointers shown instead of a session: creation hint for unknown machines, start hint for known-but-stopped machines, start hint when nothing is running.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In usability tests with at least three machines in mixed states, an operator can run bare `microvm ssh`, see only the running machines, select one using only the keyboard, approve elevation, and reach a working guest shell in under 60 seconds.
- **SC-002**: In 100% of named-connect tests against a running machine, the session reaches that machine's guest shell using the key path, account, address, and port from the running-machines entry, and key contents never appear in output.
- **SC-003**: In 100% of selector tests with stopped or being-created machines present, those machines are absent from the offered list; in 100% of empty-running-set tests, the command reports nothing to connect to and opens no session.
- **SC-004**: In 100% of session-fidelity comparisons against the equivalent manual SSH invocation, interactive typing, full-screen output, terminal resizing, interrupt handling, and remote exit-status propagation behave identically.
- **SC-005**: In 100% of failure-path tests (unknown machine, invalid name, machine not running, missing key material, unavailable SSH client, refused connection), the command returns a nonzero status, identifies the failure, opens no session, leaves lifecycle and inventory state unchanged, and gives a concrete retry or repair action.
- **SC-006**: In 100% of cancellation tests (selector cancelled, elevation declined, interrupt during establishment), no session is opened and the command reports cancellation with the standard interrupt exit status where applicable.
- **SC-007**: In 100% of regression checks, previously available behaviors (machine creation, start flow, and the connection details they report) behave exactly as before this feature; the only new observable capabilities are the running-machines selection source and the `ssh` command itself.

## Assumptions

- The guest account and port come from the returned connection details; today the account is the fixed guest root account and the port is the SSH default, matching the requested `root@{private address}` form, so the port flag appears only when the returned port differs from the default.
- The guest address used for the session is the machine's private guest address from the returned connection details, consistent with the connection hints shown by creation and start.
- Opening an interactive shell requires an interactive terminal; without one and without a remote command, the command fails fast with usage guidance instead of launching a broken session. Scripted use (explicit name plus remote command) performs no prompts and may run without an interactive terminal.
- The SSH client program is expected to be present on the host; its absence is a calm, guided failure, not a silent one.
- Elevation cases mirror the established lifecycle commands: prompt-driven escalation in interactive terminals when not already privileged; the exact backend ordering and re-execution mechanics are owned by the existing privilege flow and unchanged here.
- The minimal SDK listing reuses the existing local inventory as its data source and adds only the running filter plus the SSH connection material; storage of keys, addresses, and lifecycle state is unchanged.
- Stopping, rebooting, deleting, listing, inspecting, guest readiness, and connection-hint rendering beyond the required failure pointers are out of scope.
- All operator-facing text for this feature is English; localization is out of scope.
