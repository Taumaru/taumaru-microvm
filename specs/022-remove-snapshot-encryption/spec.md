# Feature Specification: Unencrypted MicroVM Snapshots and Restore

**Feature Branch**: 022-remove-snapshot-encryption

**Created**: 2026-09-24

**Status**: Draft

**Input**: User description: "Remove password-based encryption from VM snapshots. Neither snapshot creation nor machine restoration will require a password; snapshot archives will have no encryption."

## Clarifications

### Session 2026-09-24

- Q: What access permissions should be applied when an unencrypted snapshot is created? → A: Use the operating system defaults and the destination directory's access policy.

## User Scenarios & Testing

### User Story 1 - Create an unencrypted snapshot (Priority: P1)

As a MicroVM operator, I want to create a snapshot without choosing or entering a password, so that snapshot creation has no encryption step.

**Why this priority**: Removing the password and encryption from snapshot creation is the central requested change.

**Independent Test**: Create snapshots from running and stopped VMs through every supported entry point, then confirm each archive can be read without a secret and the source VM retains its expected lifecycle state.

**Acceptance Scenarios**:

1. **Given** a VM that can be snapshotted, **When** the operator creates a snapshot, **Then** one portable archive is produced with no password and no encrypted contents.
2. **Given** an interactive terminal or a non-interactive script, **When** the operator creates a snapshot, **Then** creation completes without a password prompt or password value provided through any supported entry point.
3. **Given** a snapshot is successfully created, **When** the CLI reports its location, **Then** it clearly states that the archive is unencrypted and contains the VM's disk and SSH credentials.
4. **Given** a running VM is snapshotted, **When** the operation completes, **Then** the VM remains running and the archive represents a coherent disk capture; a stopped VM remains stopped.
5. **Given** the output path is in a directory governed by system defaults and directory access rules, **When** a snapshot is created, **Then** the archive follows those access permissions.

### User Story 2 - Restore a VM without a password (Priority: P1)

As an operator moving a MicroVM to another compatible host, I want to restore a supported snapshot without entering a password, so that recovery has no decryption step.

**Why this priority**: Snapshot portability is only useful if the resulting archive can be restored through the same passwordless workflow.

**Independent Test**: Restore a supported unencrypted archive both interactively and in a non-interactive run, then confirm the VM is discoverable, stopped, and has the archived portable configuration.

**Acceptance Scenarios**:

1. **Given** a supported unencrypted archive and a compatible destination, **When** the operator restores it, **Then** the VM is reconstructed without a password and is left stopped.
2. **Given** an interactive terminal or a non-interactive script, **When** the operator restores a supported archive, **Then** restore completes without a password prompt or password value provided through any supported entry point.
3. **Given** an archive created under the previous encrypted format, **When** the operator attempts to restore it, **Then** restore reports that the format is unsupported and that the VM must be snapshotted again without encryption, without asking for a password or changing destination state.
4. **Given** a supported archive is truncated or corrupted, **When** restore is attempted, **Then** it rejects the archive and leaves no discoverable VM or partial destination files.

### Edge Cases

- The CLI is run without an interactive terminal; snapshot creation and restore must not wait for password input.
- A caller supplies the removed --password option; the command reports that the option is unsupported and does not echo the supplied value.
- The input is an older password-encrypted snapshot; restore rejects it without prompting or modifying local VM state.
- An unencrypted archive is unreadable, truncated, malformed, or fails its recorded integrity checks; restore fails without publishing a partial VM.
- A snapshot archive contains guest data or private SSH credentials that the operator did not realize would be readable without decryption.
- The destination already contains a VM with the archived name or cannot satisfy the existing restore prerequisites; failure leaves existing VMs unchanged.

## Requirements

### Functional Requirements

- **FR-001**: Snapshot creation MUST produce a single portable archive whose archive layer does not encrypt or conceal its payloads and can be read without an archive password or decryption key.
- **FR-002**: Snapshot creation MUST require no password. No supported way to request a snapshot may prompt for or accept an archive password.
- **FR-003**: Restore of a supported unencrypted archive MUST require no password. No supported way to request a restore may prompt for or accept an archive password.
- **FR-004**: The snapshot archive MUST retain the portable VM contents and metadata required by the existing snapshot and restore behavior, including the root disk, guest kernel, portable configuration, and associated SSH credentials.
- **FR-005**: Snapshot and restore MUST retain existing disk consistency, address policy, portability, integrity validation, and atomic failure behavior defined for VM snapshots and restore.
- **FR-006**: Integrity checks MAY detect accidental corruption or truncation, but MUST NOT be described as providing confidentiality or authenticity.
- **FR-007**: When reporting snapshot completion, the CLI MUST disclose that the archive is unencrypted and contains the VM disk and SSH credentials. The disclosure MUST NOT add a password prompt or require confirmation.
- **FR-008**: Restore MUST reject password-encrypted archives created under the previous format with a clear compatibility error. It MUST NOT request a password, attempt to decrypt them, or change existing VMs or destination files.
- **FR-009**: Restore MUST reject damaged or unsupported unencrypted archives before making a VM discoverable, and MUST remove operation-owned partial files on failure.
- **FR-010**: The supported snapshot and restore command help MUST contain no --password option, and attempts to use --password MUST fail without echoing its value.
- **FR-011**: A successful restore MUST leave the recovered VM stopped and preserve the existing destination-local reconstruction behavior.
- **FR-012**: Snapshot creation MUST follow the operating system's default access permissions and the access policy of the destination directory for the resulting archive.

### Key Entities

- **Snapshot Archive**: A single portable file containing the VM disk, guest kernel, portable metadata, and SSH credentials in readable, unencrypted form, with integrity information where supported.
- **Archive Protection**: The snapshot format has no password or cryptographic confidentiality; access to its readable contents follows operating system defaults and the destination directory's access policy.
- **Restored MicroVM**: A VM reconstructed from a supported unencrypted archive, recorded using destination-local paths and left stopped after restore.

## Success Criteria

### Measurable Outcomes

- **SC-001**: In 100% of successful snapshot operations, the archive can be read without a password or decryption key, and the VM's expected running or stopped state is preserved.
- **SC-002**: In 100% of interactive and non-interactive snapshot and restore operations, no password prompt or password input is required.
- **SC-003**: In 100% of successful restores from supported unencrypted archives, the VM is discoverable under its archived name, has its portable configuration, and is stopped.
- **SC-004**: In 100% of restore attempts using previous password-encrypted archives, the user receives a compatibility error without a password prompt, destination VM record, or operation-owned partial files.
- **SC-005**: In 100% of truncated, malformed, or integrity-invalid archive cases, restore rejects the input before publishing a VM and leaves pre-existing VM state unchanged.
- **SC-006**: Every successful snapshot completion message identifies the archive as unencrypted and discloses that it contains the VM disk and SSH credentials.
- **SC-007**: In 100% of snapshot operations, the output archive follows the access permissions determined by operating system defaults and the destination directory's policy.

## Assumptions

- This feature supersedes the password and encryption requirements in the VM snapshot and VM restore specifications. Their other requirements remain in effect, including payload composition, point-in-time disk capture, address handling, portability, and safe failure.
- Archives created with the previous password-encrypted format are unsupported by passwordless restore; users must create a new unencrypted snapshot.
- The password change concerns snapshot archive protection only. Guest accounts and SSH credentials remain part of the VM data and are not changed.
- Because the archive is unencrypted, anyone with read access to the file can inspect its contents. Archive permissions follow operating system defaults and the destination directory's access policy; recorded integrity checks do not provide confidentiality. Encryption already present inside guest disk data remains part of that guest data and is outside this change.
