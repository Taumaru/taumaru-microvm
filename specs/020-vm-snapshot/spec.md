# Feature Specification: Online Encrypted MicroVM Snapshot

**Feature Branch**: 020-vm-snapshot  
**Created**: 2026-09-23  
**Status**: Draft  
**Input**: User description: "Add an SDK operation to create a single password-protected snapshot file for a MicroVM, together with a CLI command. The snapshot must capture a coherent point-in-time copy of the machine's disk while a running VM continues, include the metadata and SSH credentials needed to recreate it on another compatible PC, and exclude host runtime binaries. The CLI must support interactive selection, an optional VM name and output path, and `--password`."

## User Scenarios & Testing

### User Story 1 - Capture a running VM (Priority: P1)

As a VM operator, I want to create a snapshot while a MicroVM remains running, so that I can save its disk and configuration without shutting down the workload.

**Why this priority**: Capturing a stable disk view during normal operation is the central value of this feature.

**Independent Test**: Start a VM, change data on its disk while the snapshot is being written, and verify that the completed archive contains the disk state from the capture point and that the VM remains running.

**Acceptance Scenarios**:

1. **Given** a running VM with enough temporary snapshot space, **When** the SDK creates a snapshot, **Then** it produces one encrypted archive and the VM process remains running.
2. **Given** the guest continues writing to its disk after the snapshot point, **When** the archive finishes, **Then** those later writes do not change the disk contents represented by the archive.
3. **Given** a stopped VM, **When** the SDK creates a snapshot, **Then** it captures the stable disk and the same portable VM metadata without requiring a running snapshot layer.
4. **Given** snapshot space is unavailable or the snapshot view becomes invalid, **When** creation fails, **Then** the SDK reports the failure to its caller, the source disk remains intact, the running VM remains running, and no incomplete archive is published at the requested destination.
5. **Given** a snapshot is active for one VM, **When** a lifecycle operation targets that VM, **Then** the operations are serialized safely while an operation for a different VM can proceed independently.
6. **Given** snapshot creation is cancelled after temporary resources are created, **When** cleanup completes, **Then** the VM remains running, the source disk is unchanged, and the incomplete archive is not published.
7. **Given** an SDK snapshot operation fails, **When** it returns to the caller, **Then** it returns an error without writing prompts, progress, or diagnostics to standard output or standard error.

### User Story 2 - Create a snapshot from the CLI (Priority: P1)

As a CLI user, I want to select a VM or name it directly and choose where to save its snapshot, so that I can use the same operation interactively or in a script.

**Why this priority**: The CLI is the main user-facing entry point requested for the SDK operation.

**Independent Test**: Exercise the command with no name, with a name, and with both a name and output path; verify each invocation calls the SDK and reports the resulting file path.

**Acceptance Scenarios**:

1. **Given** an interactive terminal and no VM name, **When** the user runs `microvm snapshot`, **Then** the CLI offers the available VMs for selection.
2. **Given** a VM name and no output path, **When** the user runs `microvm snapshot <NAME>`, **Then** the CLI writes to `./<NAME>.tmvmsnap` and reports that path.
3. **Given** a VM name and output path, **When** the user runs `microvm snapshot <NAME> <OUTPUT_PATH>`, **Then** the CLI writes the archive to that path.
4. **Given** a password supplied through `--password <PASSWORD>`, **When** the command runs, **Then** the CLI passes it to the SDK without displaying it in output.
5. **Given** an interactive terminal and no `--password`, **When** the command needs the encryption password, **Then** the CLI requests it without echoing the characters.
6. **Given** no interactive terminal and no VM name, **When** the command runs, **Then** it returns a clear actionable error instead of blocking for input.
7. **Given** no interactive terminal and no password, **When** the command runs, **Then** it returns a clear actionable error instead of blocking for input.
8. **Given** the requested output path already contains a file, **When** the command runs, **Then** it refuses to overwrite that file and explains how to choose another path.

### User Story 3 - Keep the archive portable and protected (Priority: P1)

As an operator moving a VM to another compatible host, I want the archive to contain the VM's required guest data and portable configuration while protecting all contents with a password.

**Why this priority**: A snapshot is useful only if it can be transported safely and contains enough information to reconstruct the VM independently of the source host's local inventory.

**Independent Test**: Create an archive, verify that its contents can be recovered with the correct password, and confirm that a wrong password or modified archive is rejected.

**Acceptance Scenarios**:

1. **Given** a valid password, **When** an archive is opened by a compatible snapshot reader, **Then** it yields the captured root disk, guest kernel, portable VM configuration, and host SSH credentials.
2. **Given** an incorrect password or a modified archive, **When** a reader verifies the archive, **Then** it rejects the archive without accepting partial contents as a valid snapshot.
3. **Given** an archive created on one host, **When** it is used on another compatible host, **Then** its portable contents do not depend on the source host's absolute paths, local database, process IDs, sockets, loop or mapper names, or host network resources.
4. **Given** the destination host, **When** the VM is reconstructed, **Then** host-specific networking and runtime resources can be created locally without importing source-host ownership state.
5. **Given** a snapshot archive is successfully decrypted, **When** its contents are verified, **Then** it contains the VM disk, kernel, configuration, and credentials needed for a bootable recovery, without guest memory or in-memory process state.

### Edge Cases

- The selected VM is starting, stopping, missing, or otherwise unavailable for a consistent capture.
- Another lifecycle operation targets the same VM while snapshot creation is active.
- The temporary storage cannot reserve enough space for ongoing guest writes, or reaches its limit during capture.
- The output path already exists, its parent directory is unavailable, or the output cannot be finalized.
- A source disk, guest kernel, or SSH credential file is missing, unreadable, or inconsistent with its stored metadata.
- The operation is cancelled or the host command fails while snapshot resources are active.
- The guest architecture is incompatible with the destination host.
- The archive is truncated, corrupted, or opened with the wrong password.

## Requirements

### Functional Requirements

- **FR-001**: The SDK MUST provide an operation that accepts a VM name, an output path, and a password and reports whether creation succeeded or failed.
- **FR-002**: When the VM is running, snapshot creation MUST capture one stable point-in-time view of its complete root disk while leaving the VM process running.
- **FR-003**: Guest writes made after the capture point MUST NOT alter the disk contents represented by the archive.
- **FR-004**: Snapshot creation MUST coordinate with lifecycle operations for the same VM and MUST allow independent VMs to be snapshotted independently.
- **FR-005**: Snapshot creation MUST detect unavailable or exhausted temporary snapshot storage, fail without publishing an incomplete archive, preserve the source disk, and leave a running VM running.
- **FR-006**: The archive MUST contain a versioned manifest with the VM identity, guest architecture, CPU and memory configuration, disk size, boot configuration, distribution and image provenance, and logical network configuration needed for reconstruction.
- **FR-007**: The archive MUST contain the captured root disk, the exact guest kernel required to boot it, and the host SSH public and private credentials associated with the VM.
- **FR-008**: The archive MUST be a single password-encrypted file. The password MUST NOT be stored in the archive, logged, or written to standard output or standard error by the SDK.
- **FR-009**: The archive MUST detect incorrect passwords and content modification. The SDK MUST publish the requested output only after the archive has been completed successfully.
- **FR-010**: The portable archive MUST NOT depend on source-host absolute paths, the source host's complete local database, process identifiers, control sockets, runtime device names, or host network resource ownership records.
- **FR-011**: The archive MUST exclude host execution binaries. It MUST record host compatibility requirements; reconstruction requires a compatible host with the necessary runtime installed.
- **FR-012**: The CLI MUST expose `microvm snapshot`, `microvm snapshot <NAME>`, and `microvm snapshot <NAME> <OUTPUT_PATH>` forms and delegate snapshot behavior to the SDK.
- **FR-013**: When the CLI receives no VM name in an interactive terminal, it MUST offer VM selection. Without an interactive terminal, a missing name MUST produce an actionable error.
- **FR-014**: When no output path is given, the CLI MUST write `./<NAME>.tmvmsnap`. It MUST reject an existing destination rather than overwrite it silently.
- **FR-015**: The CLI MUST accept `--password <PASSWORD>` and MUST support a hidden password prompt when running interactively without that option. It MUST NOT display the password in command output.
- **FR-016**: When the VM is stopped, the SDK MUST create the same portable archive from its stable root disk without requiring an online snapshot view.
- **FR-017**: Snapshot creation MUST clean up temporary snapshot resources on success, failure, and cancellation without deleting or replacing the VM's persistent root disk.
- **FR-018**: This feature MUST create a bootable recovery archive and MUST NOT claim to capture guest memory or resume in-memory processes from the exact execution state.
- **FR-019**: The SDK MUST not write prompts, progress, or diagnostics directly; the CLI owns user interaction and error presentation. The SDK MUST report expected input, storage, encryption, and lifecycle failures to its caller.

### Key Entities

- **Snapshot Archive**: One password-protected file containing a portable manifest and the VM payload needed for reconstruction.
- **Snapshot Manifest**: Versioned description of VM identity, guest compatibility, boot and resource configuration, logical network intent, and payload integrity data.
- **Point-in-Time Disk View**: The immutable contents of the VM root disk as observed at the snapshot capture point.
- **VM Credentials**: The host SSH public and private keys associated with a VM and the metadata needed to restore them with appropriate file permissions.

## Success Criteria

### Measurable Outcomes

- **SC-001**: A user can create one password-protected archive for any selected VM using the SDK or one of the documented CLI forms.
- **SC-002**: In 100% of successful online captures, the VM remains running and writes made after the capture point do not appear in the archived disk view.
- **SC-003**: In 100% of tested incorrect-password and modified-archive cases, verification rejects the archive before treating it as complete.
- **SC-004**: In 100% of tested temporary-storage and cancellation failures, the source root disk remains intact and no final archive is published.
- **SC-005**: The archive contains the guest disk, guest kernel, VM configuration, and SSH credentials without requiring the source host's database or host-specific runtime state.

## Assumptions

- The default output path is the current working directory with the VM name and `.tmvmsnap` extension; an existing output file is never silently overwritten.
- In interactive use, the CLI can prompt for the password without echoing it. The explicit `--password <PASSWORD>` form is available for callers that supply it directly.
- The guest kernel is part of the VM payload because it is required to boot the disk. Host-side `Firecracker` and `firectl` binaries are supplied by the destination host.
- Snapshot consistency covers a coherent disk view. Application-specific database quiescing is not guaranteed; applications may perform their normal recovery on boot.
- A brief disk I/O delay may occur while the capture boundary is established. The VM process and its vCPUs are not stopped as part of the disk-only snapshot.
- The target host must be compatible with the guest architecture and provide Linux virtualization support and the runtime required to start the VM.
- This feature defines a portable archive for a future restore workflow; a separate restore/import command is outside this feature's scope.
