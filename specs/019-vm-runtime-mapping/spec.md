# Feature Specification: Per-VM Runtime Disk Mapping

**Feature Branch**: 019-vm-runtime-mapping
**Created**: 2026-09-23
**Status**: Draft
**Input**: User description: "Implement only the start/stop portion: on start, create and attach a loop device and Device Mapper mapping for each MicroVM if no valid active mapping already exists, and pass that mapping to firectl instead of the .ext4 file; on stop, after the machine has fully exited, remove any Device Mapper and loop configuration for that MicroVM without deleting its .ext4 file. Support multiple independent MicroVMs. Use mapping names that make existing configurations discoverable without relying on the persistent database, because these configurations are transient."

## Clarifications

### Session 2026-09-23

- Q: After the VM starts through Device Mapper, should the SDK's public `rootfs_path` result field continue to point to the persistent `rootfs.ext4` file? → A: Preserve the existing public `rootfs_path` field and its value as the persistent `.ext4` path; keep the Device Mapper path internal and pass it only to firectl.
- Q: If start finds incomplete loop or Device Mapper resources attributable to the VM, with no active process using them, should the SDK clean them up and recreate them automatically? → A: Clean up and recreate only resources whose ownership by the VM is verified and that are not in use; if ownership cannot be verified, return an error without changing them.

## User Scenarios & Testing

### User Story 1 - Start a MicroVM through its runtime disk mapping (Priority: P1)

An application developer starts a stopped MicroVM. The SDK prepares or reuses that VM's own loop and Device Mapper resources, then launches the VM using the mapped disk path. The guest continues to use the existing root disk file as its persistent storage.

**Why this priority**: Every successful start must route guest disk I/O through the per-VM mapping for the later online snapshot workflow to be possible.

**Independent Test**: Start two stopped MicroVMs with different root disk files. Confirm each launch uses a distinct mapped path associated with its own file and neither launch passes the root disk file directly.

**Acceptance Scenarios**:

1. **Given** a stopped MicroVM with no active runtime mapping, **When** the caller starts it, **Then** the SDK creates its loop association and snapshot-capable Device Mapper origin before launch, and firectl receives the mapped path.
2. **Given** a valid active mapping for the same MicroVM and root disk, **When** the caller starts the MicroVM, **Then** the SDK reuses that mapping and does not create a duplicate.
3. **Given** a host-visible mapping name is already present but refers to another VM or a different root disk, **When** the caller starts the MicroVM, **Then** the SDK returns a typed conflict error and does not take over or alter that mapping.
4. **Given** two different MicroVMs are started, **When** both launches complete, **Then** each VM has distinct runtime resources and uses only its own mapped disk path.
5. **Given** an incomplete runtime resource set is verified to belong to the requested MicroVM and no process is using it, **When** the caller starts that MicroVM, **Then** the SDK removes the owned partial resources and creates a fresh mapping before launch.

---

### User Story 2 - Stop a MicroVM and release its runtime disk resources (Priority: P1)

An application developer stops one MicroVM. After the Firecracker process has exited and released the disk, the SDK removes that MicroVM's Device Mapper mapping and loop association. Its root disk file remains available for the next start.

**Why this priority**: Runtime mappings consume host resources and must not survive normal stop operations as stale or ambiguous VM state.

**Independent Test**: Start one VM, stop it, and verify the VM process has exited, its owned Device Mapper and loop resources are gone, its root disk file still exists, and another VM's resources are unchanged.

**Acceptance Scenarios**:

1. **Given** a running MicroVM, **When** the caller stops it and the process exits, **Then** the SDK removes only that VM's Device Mapper mapping and loop association after the process releases the disk.
2. **Given** a MicroVM is already stopped but has leftover runtime mappings, **When** the caller stops it, **Then** the SDK cleans up those mappings idempotently and reports success only when cleanup is complete.
3. **Given** a MicroVM has stopped, **When** the caller checks its storage, **Then** its existing .ext4 root disk file remains present and is not removed, replaced, or truncated by cleanup.
4. **Given** one VM is stopped while another VM is running, **When** cleanup completes, **Then** the running VM's process and mappings remain untouched.
5. **Given** the SDK cannot confirm that Firecracker exited or a device remains in use, **When** cleanup is requested, **Then** the SDK retains the resources and returns a typed error that can be retried.

---

### User Story 3 - Recreate transient mappings after a host restart (Priority: P2)

An application developer starts a MicroVM after the host has rebooted. Its persistent inventory and root disk still exist, while its former kernel loop and Device Mapper resources may not. The SDK reconstructs the runtime mapping from the VM's stable identity and disk, without requiring a saved mapping record in the database.

**Why this priority**: The loop and Device Mapper state is host runtime state; a reboot must not make an otherwise intact VM permanently unstartable.

**Independent Test**: Create a VM, remove its runtime mappings or reboot the host, retain its inventory and root disk, then start it. Confirm a fresh per-VM mapping is created and the VM launches through that mapping.

**Acceptance Scenarios**:

1. **Given** the inventory and root disk persist but the kernel mappings are absent, **When** the caller starts the MicroVM, **Then** the SDK rediscovers the intended resource identity and recreates the loop and Device Mapper resources.
2. **Given** runtime mappings for multiple VMs are absent after reboot, **When** each VM starts, **Then** each mapping is reconstructed independently and no VM relies on another VM's device number or mapping state.

### Edge Cases

- The host lacks the required permissions or loop/Device Mapper support; startup returns a typed error and does not launch Firecracker with the raw root disk.
- Loop setup succeeds but Device Mapper setup fails; the SDK removes the loop association created by that attempt and preserves the root disk.
- Mapping setup succeeds but Firecracker launch fails; the SDK confirms the process has exited before cleaning up resources created by that attempt.
- A loop device number changes or is reused after reboot; the SDK does not treat a numeric loop path as a stable VM identity.
- A mapping exists under the expected name but points to a different disk, SDK home, or VM; the SDK fails safely without modifying another resource.
- An incomplete runtime resource set is removed and recreated only when the SDK verifies that it belongs to the requested VM and no process is using it; ambiguous ownership or active use returns an error without changes.
- Device Mapper removal or loop detachment fails because a process or another device still holds it; cleanup reports a typed error and leaves the root disk intact.
- Concurrent start or stop calls target the same VM; they do not create duplicate mappings or race resource teardown.
- Operations target different VMs concurrently; resource names and cleanup remain isolated per VM.

## Requirements

### Functional Requirements

- **FR-001**: The SDK MUST prepare a loop association and a Device Mapper path for a MicroVM before launching its Firecracker process.
- **FR-002**: The SDK MUST pass the mapped disk path to firectl and MUST NOT pass the .ext4 file directly as the running VM's writable root drive. It MUST preserve the existing public `rootfs_path` result field as the persistent .ext4 file path; the Device Mapper path MUST remain internal to the SDK and MUST NOT be exposed through the existing public result fields.
- **FR-003**: The active Device Mapper path MUST act as a snapshot-capable origin through which all guest disk writes pass, so a later feature can create a CoW snapshot without changing the VM's backing-file identity. Creating or exporting a snapshot is outside this feature's scope.
- **FR-004**: Each VM's runtime resources MUST have a deterministic, host-visible identity derived from the SDK home and validated VM name, so the resources can be found and attributed without reading database runtime records.
- **FR-005**: The Device Mapper name or identifier MUST make the owning VM recognizable and MUST be unique across VMs and SDK homes managed on the same host.
- **FR-006**: The loop association MUST be discoverable by its VM-specific reference or by its uniquely identified backing disk. A dynamically allocated loop device number MUST NOT be treated as a durable identity or persisted as the sole way to find the association.
- **FR-007**: On start, the SDK MUST reuse an existing mapping only after verifying that the complete mapping chain belongs to the requested VM and targets its expected root disk. If an incomplete mapping chain is verified to belong to that VM and no live process holds any part of it, the SDK MUST remove those partial resources and create a fresh mapping. If no mapping resources exist, the SDK MUST create them. If ownership or process use cannot be verified safely, the SDK MUST return a typed conflict error without altering the resources.
- **FR-008**: If a mapping identity collides with a resource owned by another VM, home, process, or backing disk, the SDK MUST return a typed conflict error and MUST NOT adopt, overwrite, or remove that resource.
- **FR-009**: The SDK MUST create and validate the mapping before launching firectl. If preparation fails, it MUST return a typed error and MUST NOT launch the VM using the raw .ext4 path.
- **FR-010**: Repeated start calls for an already-running VM MUST preserve the existing running process and mapping without creating duplicates.
- **FR-011**: The SDK MUST preserve the existing graceful and forced stop behavior, and MUST begin mapping cleanup only after it verifies that the Firecracker process has exited and released the disk.
- **FR-012**: Stop MUST remove the target VM's Device Mapper resources before detaching its associated loop device, and MUST leave resources owned by every other VM untouched.
- **FR-013**: Stop MUST also discover and clean up leftover resources owned by the target VM when the VM is already stopped, making cleanup idempotent.
- **FR-014**: Stop MUST NOT delete, replace, truncate, or relocate the VM's .ext4 root disk file. Guest writes made through the mapping continue to persist in that file.
- **FR-015**: If the SDK cannot verify process exit or cannot release a runtime mapping safely, it MUST return a typed error, retain enough host-visible identity for retry, and leave the root disk intact.
- **FR-016**: After a host reboot, the SDK MUST be able to recreate missing loop and Device Mapper resources from stable VM identity and the configured root disk, without requiring a persisted mapping identifier in SQLite.
- **FR-017**: Multiple MicroVMs MUST be able to use independent mappings concurrently; start or stop of one VM MUST NOT change another VM's disk path or mapping state.
- **FR-018**: Expected permission, host capability, mapping conflict, launch, and cleanup failures MUST be returned as typed SDK errors without panic, process termination of the caller, stdout/stderr output, or logging side effects.

### Key Entities

- **VM runtime disk mapping**: The temporary host resource chain that lets one MicroVM access its root disk through a loop association and a Device Mapper origin.
- **Backing root disk**: The persistent .ext4 file containing the VM's data. Its contents continue to receive guest writes, and stop cleanup preserves the file.
- **Runtime resource identity**: The deterministic VM-scoped host-visible name or reference used to discover, validate, reuse, and remove the correct loop and Device Mapper resources without database runtime state.
- **Mapped launch path**: The internal Device Mapper path supplied to firectl for the lifetime of one VM process; it is not returned through the existing public `rootfs_path` field, which continues to identify the persistent .ext4 file.

## Success Criteria

### Measurable Outcomes

- **SC-001**: Every successful start boots the intended VM against its own persistent root disk without using another VM's storage.
- **SC-002**: At least two MicroVMs can start and stop concurrently without affecting each other's running state or disk data.
- **SC-003**: Every successful stop releases the target VM's temporary runtime storage resources and preserves its existing root disk file at the original path.
- **SC-004**: After a host reboot, starting a VM succeeds when its inventory and root disk remain, even though transient runtime configuration was lost.
- **SC-005**: Repeating start and stop leaves at most one active runtime resource set for each running VM and none for a stopped VM in 100% of lifecycle scenarios.
- **SC-006**: Simulated setup, launch, process-exit, and cleanup failures preserve the VM's root disk and other VMs' resources while returning an actionable failure result.

## Assumptions

- The SDK caller supplies the SDK home explicitly, and the persisted inventory provides the VM identity and root disk path. Database runtime fields are not required to identify the loop or Device Mapper resources.
- Runtime loop and Device Mapper resources are kernel state and may disappear on reboot; start is responsible for reconstructing them when needed.
- The host runs Linux with loop and Device Mapper support, and the SDK process or its host-side runtime adapter has the permissions needed to manage those resources. Missing permissions are reported as typed errors.
- firectl and the configured Firecracker isolation can access the mapped device path. Planning must verify the exact path exposure and permissions for the project's supported runtime.
- The active mapping is prepared as an origin for a future CoW snapshot flow. Snapshot creation, backup streaming, encryption, restore, and snapshot persistence across reboot are out of scope.
- Stop cleanup applies only after the VM process no longer holds the disk. A failed cleanup leaves the VM's persistent root disk untouched and can be retried.
- VM creation continues to create the .ext4 root disk; this feature changes only runtime start and stop storage handling.

