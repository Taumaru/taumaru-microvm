# Feature Specification: MicroVM Boot Autostart

**Feature Branch**: `023-microvm-boot-autostart`

**Created**: 2026-09-26

**Status**: Implemented

**Input**: User description: "Let the user configure the CLI so that selected MicroVMs start
automatically whenever the host boots. Commands follow the existing pattern: a selector when no
name is given and no prompts with `--non-interactive`. The SDK must not know the CLI. Provide
create, update, delete, and list for every configured machine."

## Clarifications

### Session 2026-09-26

- Q: Where is the autostart configuration stored? → A: The SDK persists a per-MicroVM policy in
  the local inventory and exposes generic operations; the CLI owns the boot hook (systemd).
- Q: Which fields can be updated? → A: Enabled/paused and the number of start attempts on
  failure.
- Q: Command shape? → A: `microvm autostart add|edit|rm|ls`.

## User Scenarios & Testing

### User Story 1 - Start selected machines at boot (Priority: P1)

An operator marks one or more MicroVMs for autostart. After the host boots, every enabled machine
is running without manual action.

**Acceptance Scenarios**:

1. **Given** a created MicroVM, **When** the operator runs `microvm autostart add web-01`,
   **Then** the policy is stored and the boot service for this home is enabled.
2. **Given** enabled policies, **When** the host boots, **Then** each enabled machine is started
   in name order, a machine that is already running counts as started, and one failing machine
   does not prevent the others from starting.
3. **Given** a machine whose start fails, **When** the boot run executes, **Then** it retries up
   to the configured attempts and reports the last error.

### User Story 2 - Manage policies per machine (Priority: P1)

1. `edit` changes enabled/paused and max attempts; interactive mode pre-fills current values.
2. `rm` removes the policy only; the machine is untouched; repeating it is harmless.
3. `ls` shows NAME, AUTOSTART (`enabled`/`paused`), ATTEMPTS, and verified STATE.
4. Deleting a MicroVM removes its policy.
5. When no enabled policy remains, the boot service instance is disabled.

### Edge Cases

- Host without systemd: commands fail before any change with a what/why/next message.
- `add` for a machine with a different existing policy: conflict pointing to `edit`.
- Non-interactive `add|edit|rm` without a name, or `edit` without a change: missing-value error,
  no prompt.
- Snapshots and restores do not carry the policy; it is host-local.

## Requirements

- **FR-001**: The SDK MUST persist autostart policies without knowledge of the CLI, executables,
  or service managers.
- **FR-002**: Attempts MUST be between 1 and 10 (default 3); invalid values are typed errors.
- **FR-003**: SDK operations: create (idempotent for identical settings), update, delete
  (idempotent), get, list, and run.
- **FR-004**: The CLI MUST follow the selector / `--non-interactive` / privilege-escalation
  pattern of `start`, `stop`, and `delete`.
- **FR-005**: The boot hook MUST be scoped per Taumaru home so separate homes never overwrite
  each other.
