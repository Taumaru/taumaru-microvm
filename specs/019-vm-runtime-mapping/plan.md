# Implementation Plan: Per-VM Runtime Disk Mapping

**Branch**: 019-vm-runtime-mapping (Spec Kit feature context; the Git checkout reported main)
**Date**: 2026-09-23
**Spec**: specs/019-vm-runtime-mapping/spec.md

**Input**: Feature specification for creating and releasing per-VM loop and Device Mapper resources during start and stop.

## Summary

Add an internal SDK runtime-disk boundary backed by Linux loop and Device Mapper tools. On start, the SDK will verify or create a loop association and a named Device Mapper snapshot-origin for the VM's persistent rootfs.ext4, then pass only the mapper path to firectl. On stop, after process exit is verified, it will remove that VM's Device Mapper mapping and detach its exact loop association. The public rootfs_path result and persistent inventory remain unchanged; no mapping identifiers are added to SQLite.

The active snapshot-origin is groundwork for a later CoW snapshot operation. It does not itself create a point-in-time snapshot or backup.

## Technical Context

**Language/Version**: Rust 2024. The active workspace toolchain is rustc 1.98.1; no explicit workspace MSRV is declared.

**Primary Dependencies**: Existing SDK dependencies, including sha2 for deterministic identity. The runtime adapter will invoke losetup and dmsetup with argument vectors and captured output, without a shell. Add fs4 for a cross-process advisory lock in the SDK runtime directory; acquire it with nonblocking attempts and async waiting so lock contention does not block a Tokio worker, and avoid introducing a Rust 1.89 standard-library API requirement.

**Storage**: The VM's rootfs.ext4 remains the durable backing file. Its temporary loop association and Device Mapper table are kernel runtime state and are not recorded as mapping rows in SQLite.

**Testing**: SDK unit tests with a fake runtime-disk controller; adapter tests for naming, table verification, command outcomes, ownership conflicts, and cleanup order; privileged Linux integration validation with loop, dm-snapshot, Firecracker, and firectl available. Run the repository's Rust quality gates during implementation.

**Target Platform**: Linux hosts with loop devices, the Device Mapper snapshot-origin target, losetup, dmsetup, Firecracker, and the host permissions needed to create and access the block devices.

**Project Type**: Rust SDK lifecycle capability consumed by the existing thin CLI. The implementation belongs in the SDK only.

**Performance Goals**: No numeric start-latency target is specified. Creating the mapping must not read or copy the root disk, so setup work is independent of image size. This feature does not pause an already-running VM to make a backup.

**Constraints**: Preserve public SDK method signatures and MicroVmStartResult.rootfs_path as the persistent .ext4 path. Keep the mapper path internal and pass it only to firectl. Do not add database mapping state or a migration. Use per-VM host-visible identity derived from SDK home and validated VM name. Never remove unverified or in-use resources. Do not implement snapshot export, encryption, restore, or snapshot persistence in this feature. SDK operations remain silent and return typed errors. Direct SDK callers must provide host privileges and device-node access; the SDK does not invoke sudo.

**Scale/Scope**: Multiple VMs may be started concurrently and must have independent resource identities. No fixed maximum number of VMs is introduced. Dynamic loop numbers are discovered at runtime and are never a durable identity.

## Constitution Check

### Gate before Phase 0

- **SDK-first shared core — PASS**: start/stop orchestration remains in MicroVmSdk; the CLI continues to call typed SDK operations.
- **Panic-free, silent SDK boundary — PASS**: host commands run without a shell, output is captured, and failures use existing typed SdkError variants. No stdout, stderr, logging, or process termination side effects are added.
- **Explicit local state and lifecycle — PASS**: the persistent VM record remains authoritative. The host-visible mapping name and UUID identify transient resources without SQLite mapping fields. Existing runtime process metadata remains available for safe cleanup and retry.
- **Closed for modification, open for extension — PASS**: add an internal runtime-disk port and Linux adapter. Keep mapper paths and command mechanics behind the SDK boundary.
- **CLI and documentation — PASS**: no CLI lifecycle engine or public command is added. Update SDK Rustdoc and feature contracts in English.
- **Gate result**: No constitution violations identified.

## Design Decisions

1. **Use a snapshot-origin mapping over a writable loop device.** The root disk remains the backing file; guest reads and writes pass through the mapper. A future snapshot operation can attach a Device Mapper snapshot with its own COW device.
2. **Use deterministic Device Mapper identity.** Derive a short readable device name and stable Device Mapper UUID from the canonical SDK home and validated VM name. Include the VM name in the visible device name and a SHA-256 digest in both identifiers. Stay within kernel name limits and use only Device Mapper-safe characters.
3. **Discover loop devices from the mapping chain and backing-file identity.** Treat /dev/loopN as an allocated endpoint, not identity. Verify the live Device Mapper table and dependencies, then verify the selected loop's backing file against the expected rootfs. Do not require losetup --loop-ref, which is informational and was added in util-linux 2.40.
4. **Serialize the full per-VM lifecycle across SDK instances.** Keep the existing per-VM in-process lifecycle lock and add a per-VM advisory lock under the SDK-owned runtime directory. Hold both locks from runtime-state inspection through start launch/readiness/persistence or stop exit/cleanup. Use nonblocking lock attempts with async waiting, losetup --nooverlap, and safe allocator-contention handling. Keep lock files in place so concurrent callers cannot lock different inodes.
5. **Keep commands replaceable and contained.** Add a private RuntimeDiskController port and a Linux DeviceMapperRuntime adapter under adapters/runtime. Invoke losetup and dmsetup directly with Command arguments; never interpolate user input into a shell command.
6. **Preserve the SDK contract.** Keep MicroVmStartResult.rootfs_path and all public result fields unchanged. Rename or clarify the internal StartRequest field so firectl receives the mapper path without changing the persistent rootfs path held by the manager.
7. **Use existing typed errors.** Map missing tools, permissions, or targets to RuntimeIncompatible or HostCommand; identity collisions to StorageConflict; process or open-reference conflicts to TemporaryRuntime; and primary failures with rollback failures to Cleanup. Avoid adding a public SdkError variant or changing its compatibility contract.

## Lifecycle Design

### Start

1. Acquire the per-VM in-process lock and cross-process advisory lock before inspecting runtime state; hold them until the start operation returns. Validate the VM record and rootfs.ext4 using existing prerequisites.
2. Reconcile a provisional starting process before clearing stale metadata: adopt it as running only when its PID, executable identity, and ready socket agree; if the process is alive but not ready, return a retryable error and retain its PID and mappings; proceed with cleanup/restart only after the process is confirmed exited. Unknown process identity is a conflict and is left untouched.
3. Inspect the expected mapper name and UUID. Reuse only a complete, active table with the expected snapshot-origin target, sector length, loop dependency, and backing-file identity. A different UUID or backing file under the expected name is a conflict and is left untouched.
4. If no mapping exists, attach the rootfs to a writable loop device with overlap protection, create the Device Mapper snapshot-origin, and verify the resulting table and block path.
5. If an incomplete chain is positively attributed to this VM and has no live process or open holder, remove the owned mapper first, detach its loop, then rebuild. Ambiguous or busy resources return an error without mutation.
6. Pass the verified /dev/mapper/<name> path to firectl through the internal start request. Continue returning rootfs.ext4 in the public start result.
7. Persist the spawned process identity as starting before waiting for socket readiness, then mark it running after readiness. If persisting the PID fails, terminate the just-spawned child and verify exit before releasing mappings; if exit cannot be confirmed, retain the mapping and report cleanup failure. On launch/readiness failure, terminate and verify process exit before releasing mappings. If exit cannot be confirmed, retain the PID and mappings for stop retry and return a typed error.

### Stop

1. Acquire the same per-VM in-process and cross-process locks and hold them through shutdown and cleanup. Preserve current graceful shutdown and forced termination behavior.
2. In every stop path, including an already-silent socket, verify the recorded process no longer references the VM before releasing storage. If a persisted starting process is still alive but the socket is silent, return a retryable error and retain its identity and mappings. Keep the process metadata until mapping cleanup succeeds.
3. Query and remove only the exact owned Device Mapper mapping, without force or deferred removal. A busy mapping is retained and reported as retryable failure.
4. Detach only the loop device verified from that mapper/backing identity. Wait for the association to disappear before reporting success.
5. Clear stale runtime metadata and remove the stale socket only after storage cleanup succeeds. Preserve rootfs.ext4 and all other VMs' resources.

### Identity and ownership

- Canonicalize the SDK home after SDK directory creation and combine it with the validated VM name using an unambiguous separator before hashing.
- Use a human-readable Device Mapper name such as tmvm-<vm-name>-<short-digest> and a stable UUID such as TAUMARU-MICROVM-<full-digest>.
- Verify UUID, mapping table target, mapped length, dependency device number, and loop backing-file identity before reuse or removal.
- Discover a loop through the mapper dependency or the VM-unique backing disk. Never store a numeric loop path in SQLite.
- Use normal removal only. Do not use dmsetup remove_all, losetup --detach-all, forced removal, or deletion/truncation of rootfs.ext4.
- Create mappings only when host tools, target support, permissions, and mapper-node read/write access are available. Do not change mapper device permissions broadly; surface a typed host capability error.

## Constitution Check After Design

- **SDK-first shared core — PASS**: lifecycle sequencing is in manager.rs and host storage commands are behind a private SDK adapter.
- **Panic-free, silent SDK boundary — PASS**: subprocess output is captured, error paths are Result-based, and cleanup conflicts retain resources.
- **Explicit local state and lifecycle — PASS**: PID tracking is updated before readiness; mapping state stays transient and discoverable by kernel identity.
- **Closed for modification, open for extension — PASS**: storage/runtime mechanics are replaceable through an internal port; no public API or database schema changes.
- **CLI and documentation — PASS**: existing privilege escalation and command surface remain. Public Rustdoc documents the preserved rootfs_path meaning and stop cleanup contract.
- **Gate result**: No constitution violations identified. The actual bundled firectl/Firecracker versions must be exercised with a mapper block node during implementation validation.

## Project Structure

~~~text
crates/sdk/src/
├── manager.rs                         # start/stop orchestration and lifecycle tests
├── error.rs                           # reuse typed host/runtime/cleanup errors
├── ports/
│   ├── mod.rs
│   └── runtime_disk.rs                # private runtime disk mapping boundary
└── adapters/
    ├── mod.rs
    └── runtime/
        ├── mod.rs
        ├── device_mapper.rs           # loop and Device Mapper lifecycle
        └── firecracker.rs             # firectl argument uses mapped disk path

crates/sdk/Cargo.toml                  # advisory lock dependency
specs/019-vm-runtime-mapping/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
└── contracts/
    └── runtime-disk-lifecycle.md
~~~

**Structure Decision**: Implement the lifecycle use case in the existing SDK manager. Add one private port and one runtime adapter because block mapping is a replaceable host capability with independent ownership and failure behavior. Keep firectl process creation in the existing Firecracker adapter. Do not add CLI lifecycle code, public mapper types, database fields, or migrations.

## Complexity Tracking

No constitution violations; no additional complexity entries are required.
