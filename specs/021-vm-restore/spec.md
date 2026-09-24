# Feature Specification: Portable MicroVM Restore

**Feature Branch**: `021-vm-restore`  
**Created**: 2026-09-23  
**Status**: Draft  
**Input**: User description: "Add SDK restore and CLI restore operations for password-encrypted snapshots. At snapshot creation, the operator chooses whether the archive includes the source IPv4 network addresses. If included, restore preserves them and reports a conflict if the destination cannot use them. If omitted, the archive retains only whether the VM is exposed on the LAN, and restore allocates destination-local addresses based on that setting. Restore reconstructs the VM locally with its disk, kernel, distribution and image information, SSH credentials, and included metadata, and leaves it stopped."

## Clarifications

### Session 2026-09-23

- Q: When restoring on another host, how should restore handle guest and LAN IP addresses? → A: Preserve exact archived IPv4 values and fail atomically on conflicts when the snapshot includes them; otherwise allocate destination-local IPv4 values based on the archived LAN exposure setting.
- Q: Should restore support only IPv4 addresses or also IPv6? → A: Support IPv4 only and reject archives containing IPv6 network values.
- Q: If the host shuts down unexpectedly during restore, how should the next restore invocation handle partially created files and host resources? → A: Automatically reconcile and clean operation-owned partial state using a durable journal before allowing retry.

### Session 2026-09-24

- Q: When the operator omits IP preservation, should the snapshot remove source IP settings from the Taumaru-managed guest network file within the disk payload, or only omit them from the manifest? → A: Remove source IP settings from that file in the archive copy; other guest files may still contain IP text.

## Snapshot Archive Dependency

This restore feature defines the paired snapshot producer contract. The version 2 manifest MUST declare whether source IPv4 addresses are included.

- When the operator chooses to include addresses, the manifest MUST contain the source guest IPv4 address and prefix length, guest IPv4 gateway when present, LAN IPv4 address when configured, network mode, LAN exposure intent, and guest MAC. Restore MUST preserve these values and fail atomically if the destination cannot reproduce them or any address or MAC conflicts with existing allocations.
- When the operator chooses not to include addresses, the manifest MUST retain only the source LAN exposure setting as network configuration. It MUST omit the source guest and LAN IP addresses, prefix, gateway, network mode, and guest MAC. The archived disk copy MUST have the source IP settings removed from the Taumaru-managed guest network configuration, while other guest files remain untouched. Restore MUST derive the destination network mode from the exposure setting, allocate destination-local IPv4 addresses and network identity using the normal destination allocation behavior, and write the resulting configuration into the recovered disk. These transformations MUST NOT modify the source VM or its root disk.

This feature supports IPv4 network values only; restore MUST reject archives containing IPv6 addresses. Both address policies MUST include portable kernel registry metadata sufficient to register the exact embedded kernel in the destination artifact inventory: kernel ID, name, display name, version, architecture, registry path and URL, filename, format, MIME type, and modified time. The payload record is authoritative for the kernel byte count and SHA-256 digest.

Snapshot creation MUST fail without publishing an archive when the selected policy cannot be represented or required kernel registry metadata is unavailable. Source host interfaces, host-side addresses, routes, leases, ownership records, database contents, and executable paths remain excluded. Restore accepts this version 2 contract only when its declared address policy and corresponding fields are valid; it rejects version 1, unknown versions, and inconsistent or incomplete policy data.

## User Scenarios & Testing

### User Story 1 - Restore a MicroVM from an archive (Priority: P1)

As an operator moving a MicroVM to another compatible host, I want to restore its encrypted snapshot so that I recover the same guest disk, boot configuration, credentials, and portable settings without depending on the source host's database.

**Why this priority**: Reconstructing the machine from the single portable archive is the feature's primary value.

**Independent Test**: Restore a valid archive into a clean SDK home, then compare the recovered disk and kernel with the archive payloads and compare the restored VM configuration with its manifest.

**Acceptance Scenarios**:

1. **Given** a supported archive, the correct password, and a compatible destination, **When** the user restores it, **Then** the SDK creates a local VM using the name in the archive, restores its required guest data and settings, and leaves it stopped.
2. **Given** a successfully restored VM, **When** the user lists or inspects local VMs, **Then** the restored VM appears with its original portable configuration and destination-local paths.
3. **Given** the archive was created on another compatible host, **When** it is restored, **Then** reconstruction does not require the source host's database, absolute paths, process IDs, sockets, or network resource ownership.
4. **Given** the restored VM is stopped, **When** the user later starts it on the destination, **Then** its disk, guest kernel, boot settings, credentials, and network configuration selected by the snapshot's address policy are available to the normal lifecycle workflow.

---

### User Story 2 - Reject conflicts and invalid archives safely (Priority: P1)

As an operator, I want restore to fail safely when the archive cannot be trusted or its VM name is already in use, so that existing machines and data are never overwritten.

**Why this priority**: Restore writes persistent disk, credential, and inventory data, so safe failure is essential.

**Independent Test**: Try a duplicate name, wrong password, damaged archive, and unsupported format; verify that each attempt reports an actionable error and leaves existing VM records and files unchanged.

**Acceptance Scenarios**:

1. **Given** a local VM already has the name stored in the archive, **When** restore is attempted, **Then** the SDK returns a conflict and does not replace or alter that VM.
2. **Given** the password is wrong or the archive is truncated, modified, malformed, or has an unsupported version, **When** restore is attempted, **Then** the SDK rejects it and publishes no VM or partial payload files.
3. **Given** the destination lacks compatible guest architecture, runtime, network capability, a required exact archived address, or sufficient storage, **When** restore is attempted, **Then** the SDK reports the unmet prerequisite and leaves no partial VM.
4. **Given** extraction or local persistence fails after staging has begun, **When** restore returns, **Then** temporary files are removed, the source archive is unchanged, and no incomplete VM is discoverable.

---

### User Story 3 - Restore interactively or from a script (Priority: P1)

As a CLI user, I want to supply a snapshot path and one password directly or through prompts, so that I can restore a VM interactively or automate the operation.

**Why this priority**: Both guided use and deterministic automation are required for the requested CLI command.

**Independent Test**: Exercise `microvm restore`, `microvm restore <ARCHIVE_PATH>`, and `microvm restore <ARCHIVE_PATH> --password <PASSWORD>`; verify prompt behavior, non-interactive errors, and the resulting stopped VM.

**Acceptance Scenarios**:

1. **Given** an interactive terminal and no path argument, **When** the user runs `microvm restore`, **Then** the CLI asks for the snapshot file path and then requests the password once without echoing it or asking for confirmation.
2. **Given** an interactive terminal and an archive path, **When** the user runs `microvm restore <ARCHIVE_PATH>`, **Then** the CLI requests the password once without echoing it or asking for confirmation.
3. **Given** an archive path and `--password <PASSWORD>`, **When** the user runs `microvm restore <ARCHIVE_PATH> --password <PASSWORD>`, **Then** the CLI completes without prompts and does not display the password.
4. **Given** no interactive terminal and a missing path or password, **When** restore is invoked, **Then** the CLI exits with an actionable error rather than waiting for input.
5. **Given** a successful restore, **When** the CLI reports completion, **Then** it displays the restored VM name and that its state is stopped.

### User Story 4 - Choose address portability when creating a snapshot (Priority: P1)

As an operator creating a snapshot, I want to choose whether it preserves the source IPv4 addresses, so that I can either retain the exact network identity or let another host allocate compatible local addresses.

**Why this priority**: Preserving addresses can conflict with another VM on the destination, while regenerating them lets the restored VM fit the destination network.

**Independent Test**: Create snapshots with each choice and restore them on a destination where the source addresses are occupied; verify that the preserve choice fails safely and the regenerate choice allocates destination-local addresses according to LAN exposure.

**Acceptance Scenarios**:

1. **Given** an interactive snapshot command, **When** the CLI asks whether to include source IP addresses, **Then** it explains that choosing yes preserves those addresses and may conflict during restore, while choosing no stores only LAN exposure and lets restore allocate destination-local addresses.
2. **Given** the operator chooses to include addresses, **When** the snapshot is created, **Then** the manifest records the exact guest and applicable LAN IPv4 settings and the restore either preserves them or fails atomically on a destination conflict.
3. **Given** the operator chooses not to include addresses, **When** the snapshot is created, **Then** the manifest stores only the LAN exposure setting from the source network configuration, the archived disk copy has source IP settings removed from the Taumaru-managed guest network file, and restore allocates destination-local IPv4 settings based on that setting and updates the recovered guest configuration.
4. **Given** snapshot creation runs without an interactive terminal, **When** no address policy was explicitly supplied, **Then** the CLI returns an actionable error without choosing a policy or publishing an archive.

### Edge Cases

- The archive path is missing, unreadable, not a regular file, or points to an existing VM payload.
- The password is empty or incorrect, or the encrypted contents are truncated, modified, or malformed.
- The manifest version is unsupported, required entries are absent or duplicated, payload paths are unsafe, or payload size and checksum do not match the manifest.
- A VM with the archived name exists in the local inventory, or destination files already occupy its managed paths without a matching VM record.
- An address-preserving archive contains an IPv4 address or guest MAC that conflicts with another local VM, or the destination cannot reproduce the archived network configuration.
- An address-regenerating archive lacks the LAN exposure setting needed to select destination network mode, or the destination cannot allocate compatible local IPv4 addresses.
- The manifest's address policy is missing, unsupported, or inconsistent with its network fields.
- Snapshot creation cannot sanitize the Taumaru-managed guest network configuration in the archive copy; creation fails without changing the source disk or publishing a partial archive.
- The archive contains an IPv6 guest address, gateway, or LAN address, which this feature does not support.
- The archive lacks the kernel registry metadata needed to register the embedded kernel in the destination SDK artifact inventory.
- The destination cannot provide a compatible guest architecture, runtime, or sufficient storage.
- Restore is cancelled while payloads are being recovered, or the host shuts down unexpectedly; after a shutdown, the next restore invocation automatically reconciles journaled state owned by the interrupted operation before allowing a retry.
- A restored private SSH key cannot be written with restrictive permissions.
- Two restore operations target the same archived VM name concurrently.

## Requirements

### Functional Requirements

- **FR-001**: The SDK MUST provide a restore operation that accepts a snapshot archive path and one password and returns the restored VM identity and outcome through its normal typed result surface.
- **FR-002**: The CLI MUST expose `microvm restore`, `microvm restore <ARCHIVE_PATH>`, and `microvm restore <ARCHIVE_PATH> --password <PASSWORD>` forms and MUST delegate restore behavior to the SDK.
- **FR-003**: In an interactive terminal, the CLI MUST prompt for a missing archive path and a missing password. The password MUST be requested once, without confirmation, and MUST NOT be echoed.
- **FR-004**: In a non-interactive terminal, the archive path and password MUST be supplied explicitly; a missing value MUST return an actionable error without blocking for input.
- **FR-005**: The VM name MUST come from the decrypted archive manifest. Restore MUST NOT silently rename the VM or overwrite an existing VM.
- **FR-006**: Before publishing a restored VM, the SDK MUST validate the password, supported archive format and version, manifest, required payload set, payload paths, sizes, and integrity data. It MUST validate the archived VM name and require kernel filenames to be safe single path components before using them in destination paths. The supported version 2 manifest MUST declare its address policy. For the preserve policy, it MUST include the guest IPv4 address and prefix length, guest IPv4 gateway when present, LAN IPv4 address when configured, network mode, LAN exposure intent, and guest MAC. For the regenerate policy, it MUST include only the LAN exposure setting as source network configuration and MUST omit source addresses, prefix, gateway, mode, and MAC. Restore MUST reject IPv6 values, unknown or inconsistent policies, version 1 archives, and missing fields required by the selected policy. Both policies MUST include portable kernel registry metadata sufficient to register the exact embedded kernel in the destination artifact inventory.
- **FR-007**: The SDK MUST reject a name already present in local inventory and conflicting destination paths. It MUST safely serialize concurrent restores targeting the same name.
- **FR-008**: A successful restore MUST create a local stopped VM record containing the archived identity and portable CPU, memory, disk-size, boot, distribution, image-provenance, and guest-architecture information.
- **FR-009**: A successful restore MUST recover the complete root disk and the exact guest kernel from the archive, verify the extracted payloads against the recorded sizes and integrity data before applying destination-local network configuration, place them at destination-local paths, and register the kernel with its archived metadata so the existing start workflow resolves those exact bytes.
- **FR-010**: For an archive using the preserve policy, a successful restore MUST retain the guest IPv4 address, prefix length, guest IPv4 gateway when present, LAN IPv4 address when configured, logical network mode, LAN exposure intent, and guest MAC. If an exact value cannot be reproduced or an archived address or MAC conflicts with a destination allocation, restore MUST fail atomically. For an archive using the regenerate policy, restore MUST derive network mode from the archived LAN exposure setting, allocate IPv4 addresses and network identity through normal destination-local allocation, and update the guest network configuration in the recovered disk to match those values. Host-specific interfaces, routes, leases, and ownership records MUST always be established on the destination, never imported from the source. Neither policy may change the source archive or pre-existing VMs.
- **FR-011**: A successful restore MUST recover the associated SSH public and private keys and their metadata. The private key MUST be protected from access by other local users, and the public key and fingerprint MUST correspond to the restored credential.
- **FR-012**: The restored local inventory MUST preserve distribution and image identifiers and descriptive provenance from the archive. The restored embedded root disk and guest kernel MUST be sufficient for the guest payload; restore MUST NOT depend on source-host artifact paths or the source local database.
- **FR-013**: Restore MUST NOT include or import source-host Firecracker or `firectl` binaries, absolute executable paths, process identifiers, control sockets, loop devices, Device Mapper names, or source-host network ownership state. The restored VM MUST remain stopped and MUST NOT be launched as part of restore.
- **FR-014**: Restore MUST use compatible runtime capabilities available through the destination host's normal setup. If a required destination capability cannot be resolved, restore MUST fail without publishing an incomplete VM and MUST identify the missing prerequisite.
- **FR-015**: Restore MUST make the VM discoverable only after all required payloads and local metadata have been recovered and verified. Before changing host state, it MUST persist a durable journal sufficient to identify temporary files, partially installed payloads, kernel-cache entries, network resources, and metadata owned by the operation. On failure or cancellation, it MUST remove operation-owned partial state while preserving the source archive and all pre-existing VMs. After an abrupt host shutdown, the next restore invocation MUST reconcile incomplete journal entries for the affected VM before allowing a retry, removing only state owned by the interrupted operation.
- **FR-016**: The SDK MUST remain silent and report expected failures through typed errors. It MAY report progress through a caller-provided callback; the CLI owns prompts, progress, and user-facing diagnostics.
- **FR-017**: The CLI MUST report the restored VM name, stopped state, and destination-local identity after success, and explain the cause and next action after failure.
- **FR-018**: The SDK snapshot operation MUST receive an explicit choice to preserve or omit source IP addresses and MUST NOT prompt or write user-facing output. In an interactive terminal, the snapshot CLI MUST ask this choice and explain that preserving addresses may cause conflicts during restore. In a non-interactive terminal, the CLI MUST require an explicit choice and MUST NOT publish an archive when it is absent.
- **FR-019**: Snapshot manifests using the preserve policy MUST include the source guest IPv4 address and prefix, applicable gateway and LAN IPv4 address, LAN exposure setting, network mode, and guest MAC. Manifests using the regenerate policy MUST include only the LAN exposure setting from source network configuration and MUST omit all source IP addresses and other source network-assignment values; the archived disk copy MUST also omit source IP settings from the Taumaru-managed guest network file.
- **FR-020**: Sanitization for the regenerate policy MUST change only the private snapshot copy, MUST leave the source root disk and running VM unchanged, and MUST prevent publication of an incomplete archive if sanitization fails. It MUST NOT scan or rewrite arbitrary guest files.

### Key Entities

- **Snapshot Archive**: The password-protected input containing a versioned manifest, complete guest root disk, guest kernel, SSH credentials, and portable VM settings.
- **Snapshot Manifest**: The archived VM name, resource limits, guest architecture, boot settings, distribution and image provenance, the declared address policy and its required network values, credential metadata, and payload integrity information.
- **Restored MicroVM**: A stopped VM represented in destination-local inventory with recovered payload files, portable settings, destination-local paths, and no source-host runtime state.
- **Destination Runtime**: Host-local execution and network capabilities required to start the restored VM after restore has completed.

## Success Criteria

### Measurable Outcomes

- **SC-001**: In 100% of successful restores of supported archives on compatible hosts, the VM appears under the archived name in local inventory and is stopped.
- **SC-002**: In 100% of successful restores, the recovered root disk and guest kernel match the manifest's recorded logical sizes and integrity values, all archived portable VM settings are present locally, and networking follows the declared address policy.
- **SC-003**: In 100% of duplicate-name, wrong-password, unsupported-version or policy, unsupported-IPv6, address-conflict, corrupt-payload, and unmet-prerequisite cases, no incomplete VM record or managed payload is left behind and no pre-existing VM is changed.
- **SC-004**: Interactive restore requests the archive path only when absent and requests the password exactly once without echo or confirmation; non-interactive restore never waits for input.
- **SC-005**: For each non-empty payload, reported restore progress advances with recovered bytes and reaches completion only after payload verification succeeds.
- **SC-006**: A restored VM can be started through the existing lifecycle workflow once compatible destination runtime and network prerequisites are available.
- **SC-007**: In 100% of restores resumed after an abrupt host shutdown, the next invocation reconciles journaled partial state owned by the interrupted operation before retrying, without removing pre-existing VM data or resources.
- **SC-008**: In every interactive snapshot, the CLI presents both address choices and the conflict warning before archive creation; in every non-interactive snapshot, creation without an explicit choice fails without publishing an archive.
- **SC-009**: In 100% of restores from regenerate-policy archives, destination IPv4 configuration is allocated from destination network state using the archived LAN exposure setting, the recovered guest network configuration matches those allocations, and the manifest contains no source IP values.
- **SC-010**: In 100% of regenerate-policy snapshots, the archived disk's Taumaru-managed guest network file contains no source IP settings, while the source root disk remains unchanged; arbitrary guest files are not scanned or rewritten.

## Assumptions

- The archive's embedded VM name is authoritative; choosing a different name is outside this version of restore.
- Restore supports version 2 snapshot manifests that declare either the preserve or regenerate address policy. Version 1 archives lack this policy and fail with a clear compatibility error; future unsupported versions also fail.
- The root disk and guest kernel are embedded in the archive; distribution and image identifiers are retained as provenance, not used to replace the recovered disk.
- Host execution binaries are not part of the archive. A compatible destination-host runtime must be available through the project's normal setup before the restored VM can be started.
- Snapshot address preservation is optional and must be selected explicitly. When selected, exact source IPv4 settings and the existing portable network identity are retained, and restore fails atomically on conflicts. When omitted, the manifest stores only LAN exposure intent and the archived disk copy removes source IP settings from the Taumaru-managed guest network file; arbitrary guest files are not scanned or rewritten and may contain IP text. Destination-local addresses and network identity are allocated during restore based on LAN exposure. IPv6 network values are outside this feature and cause a compatibility error. Host interface names, host-side addresses, routes, leases, and resource ownership are recreated or reconciled on the destination host.
- Non-interactive snapshot creation requires an explicit address policy; interactive creation asks the operator and explains the restore conflict risk before continuing.
- Restore produces a bootable stopped VM, not a memory image; guest RAM and in-memory process state are not restored.
- The SDK stores restored VM files under its normal destination-local data directory. The archive path is an input and does not select the destination volume directory.
