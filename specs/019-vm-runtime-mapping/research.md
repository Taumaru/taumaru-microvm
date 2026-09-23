# Research: Per-VM Runtime Disk Mapping

## Decisions

### Device Mapper target

Use a writable loop device backed by the VM's persistent rootfs.ext4, then expose it through a Device Mapper snapshot-origin target. The mapping table covers the full backing-device length in Device Mapper sectors and names the loop device as its dependency.

The Linux kernel documentation describes snapshot-origin as the origin path used by snapshots. Reads pass through to the backing device; once a COW snapshot exists, writes are copied to that snapshot's COW device so its view stays unchanged. A snapshot-origin without a snapshot target and COW device is only a runtime mapping; it is not an immutable snapshot or backup. Creating a snapshot is outside this feature.

When a later feature loads or unloads a snapshot target, it must suspend the matching origin. The kernel documentation warns that failing to suspend it can cause data corruption. That protocol belongs to the future snapshot feature.

**Alternatives considered**:
- A plain linear mapping was rejected because it does not establish the snapshot-origin target needed by the intended future COW flow.
- Creating a snapshot target during ordinary VM start was rejected because it would need a COW device and would create snapshot lifecycle work that this feature explicitly excludes.

### Loop and Device Mapper management

Use util-linux losetup and dmsetup from a private Linux SDK adapter. The current SDK already uses host commands for filesystem and runtime work. Direct command invocation avoids shell parsing and does not require unsafe Rust ioctl code. Missing tools and unsupported kernel targets become typed SDK errors.

The loop device number is allocated by Linux and is not a stable identity. The stable identity belongs to the Device Mapper mapping name and UUID derived from canonical SDK home plus VM name. Verify the mapper UUID and live table, inspect its device dependency, and verify that the loop's backing file is the expected rootfs using its canonical path and file identity. This makes the loop discoverable from the host state and database-independent.

Do not require losetup --loop-ref. The losetup reference is informational, is limited to 64 bytes, is readable only by root, and is not used by the kernel. util-linux added the CLI option and REF output in version 2.40. Requiring it would unnecessarily narrow host compatibility.

losetup warns that multiple loop devices for the same backing file can cause data loss or corruption. It also documents that --find setup is not atomic and recommends locking for heavily parallel use. Use --nooverlap and serialize mapping reconciliation with an advisory SDK-home runtime lock. A loop detach may be lazy; wait for udev/device state to settle and verify the backing association is gone before reporting cleanup success.

**Alternatives considered**:
- Persisting /dev/loopN in SQLite was rejected because loop numbers are allocated runtime state and may change or be reused after reboot.
- Treating a mapper name alone as proof of ownership was rejected because a name can collide with a foreign mapping. Verify the UUID, target table, dependency, sector length, backing file identity, and process/open state.
- Requiring --loop-ref was rejected because it is a recent informational utility option. The mapper dependency and unique backing disk are sufficient for discovery.
- Using global detach/remove commands was rejected because they can affect unrelated VMs and host resources.

### Locking

Use the existing in-process per-VM lifecycle lock plus a per-VM cross-process advisory lock file in the SDK-owned runtime directory. Hold both locks across the complete start or stop operation, from runtime-state inspection through process launch/readiness/persistence or process exit and mapping cleanup. Locking only the mapping commands leaves a race where separate SDK clients can start the same VM while the first process has not opened its mapper yet. The deterministic mapper name provides a second collision boundary, while loop overlap checks and retry handling cover allocator contention. Keep lock files in place; unlinking a locked file can let another client lock a new inode for the same path.

Use fs4's nonblocking file-lock API with async waiting between attempts so contention does not block a Tokio worker. This avoids raising the SDK's Rust MSRV to the standard-library File::lock stabilization version. The repository uses Rust 2024 and currently has no explicit rust-version declaration.

### firectl and Firecracker path

Keep the public SDK API unchanged. The existing firectl root-drive option takes a host path with an optional :ro or :rw suffix, and the Firecracker drive API identifies a host-side path. The runtime adapter will pass the mapper node with :rw. The manager continues validating the persistent .ext4 file and returns that same path in MicroVmStartResult.rootfs_path.

Current project code starts firectl directly and does not configure a jailer. Therefore the SDK process identity must be able to create the loop and Device Mapper resources, and the launched firectl/Firecracker process must be able to read and write the mapper block node. The CLI already has a privileged start/stop path. Direct SDK callers must run under an appropriately privileged host context; the SDK will not execute sudo. If jailer support is introduced later, the mapper node must be made available inside its jail before privileges are dropped.

Upstream firectl documents a path for --root-drive. The Firecracker virtio block implementation opens the host path and has block-device handling, but upstream overview docs usually describe block backings as files. Validate the exact firectl and Firecracker artifacts used by this project with an end-to-end mapper-node launch before shipping.

### Failure and cleanup policy

Reuse a mapping only when its deterministic UUID, active table, snapshot-origin target, size, loop dependency, and backing identity all match. An expected name with another UUID or backing file is a conflict and is left untouched.

For incomplete resources, remove and rebuild only the parts verified as belonging to this VM and unused by any process. Remove Device Mapper before loop. If a mapping is busy, if the Firecracker process is still live, or if ownership is ambiguous, preserve the chain and return a typed retryable error. Never use forced or deferred Device Mapper removal and never detach all loops.

A spawned process identity is persisted before readiness completes. On failed readiness, the SDK terminates and waits for the process before releasing mappings. If it cannot confirm exit, it keeps the PID and mapping so stop can retry. On stop, clear process metadata only after the mapper and loop cleanup succeeds.

### Consistency limits

The future block snapshot would be crash-consistent at the block-device boundary. It would not automatically make application databases transaction-consistent; any application-level quiescing or recovery contract is outside this start/stop feature. This feature itself does not copy disk contents, pause a running VM, export a snapshot, encrypt an archive, or restore an image.

## Source Review

- Current SDK lifecycle is coordinated in crates/sdk/src/manager.rs. Start builds StartRequest from the persisted rootfs path, launches through RuntimeController, and stores runtime process metadata after readiness. Stop already implements graceful shutdown and forced termination but clears stale references on some socket-silent paths. The planned changes must keep process evidence until storage cleanup succeeds.
- Current internal StartRequest contains rootfs_path and FirecrackerRuntime::start_arguments passes it to firectl as --root-drive=<path>:rw. Rename the internal value to runtime_disk_path or document it explicitly as the launch path.
- MicroVmStartResult.rootfs_path is populated from the persistent inventory record. Keep it unchanged.
- RuntimeController is a private process boundary. A separate private RuntimeDiskController keeps block-resource allocation testable without mixing it into process lifecycle methods.
- Existing target_lock uses Tokio mutexes held in one MicroVmSdk object. It is not cross-process; a per-VM advisory file lock held for the full start/stop transition is needed for SDK instances sharing a home. A provisional starting PID must not be cleared solely because the socket is silent; verify process identity and exit first.
- The CLI start and stop commands already use the shared SDK and require elevated privileges. No CLI implementation change is needed.

## References

- Linux kernel, Device-mapper snapshot support: https://docs.kernel.org/admin-guide/device-mapper/snapshot.html
- util-linux, losetup manual: https://man7.org/linux/man-pages/man8/losetup.8.html
- Linux man-pages, dmsetup manual: https://man7.org/linux/man-pages/man8/dmsetup.8.html
- util-linux 2.40 release notes, addition of loop references: https://github.com/util-linux/util-linux/blob/master/Documentation/releases/v2.40-ReleaseNotes
- firectl README and --root-drive option: https://github.com/firecracker-microvm/firectl/blob/main/README.md
- firectl option parsing: https://github.com/firecracker-microvm/firectl/blob/main/options.go
- Firecracker virtio block host path handling: https://github.com/firecracker-microvm/firecracker/blob/main/src/vmm/src/devices/virtio/block/virtio/device.rs
- Firecracker host setup and jailer guidance: https://github.com/firecracker-microvm/firecracker/blob/main/docs/prod-host-setup.md
- Firecracker architecture and storage overview: https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md
- fs4 synchronous file-lock API and MSRV: https://docs.rs/fs4/latest/fs4/
