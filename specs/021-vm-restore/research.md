# Restore and Snapshot Research

## Repository Findings

- Snapshot orchestration is in `crates/sdk/src/manager.rs`. Running snapshots already select a private Device Mapper point-in-time block view and archive it; stopped snapshots use the root disk path. The online view is removed after archive success or failure.
- The archive writer at `crates/sdk/src/adapters/archive/age_tar_zstd.rs` produces age-encrypted Zstandard/TAR, writes the manifest last, and hashes fixed payloads while streaming. It currently emits version 1 and has no reader. Payload handling accepts the normal rootfs file or the designated online snapshot block device.
- Snapshot manager reads persisted network metadata and currently writes mode/MAC metadata without an address policy. The version 2 archive must represent one of two explicit policy shapes.
- The managed guest network file is `/etc/systemd/network/10-taumaru.network`. `crates/sdk/src/adapters/storage/guest_fs.rs` edits ext4 offline through `debugfs`; `GuestStorage` in `crates/sdk/src/ports/storage.rs` is the boundary to extend with a fixed-path sanitization operation and a destination-config writer.
- The current network writer encodes guest address, prefix/gateway, and optional LAN settings. Normal VM creation allocates destination values, writes the guest network configuration, then persists network and VM state. Restore can reuse this path for the regenerate policy but needs an internal request that can also carry exact archived values for preserve policy.
- The network adapter already checks LAN subnet/address conflicts. The repository does not expose a compound restore commit today: VM, network, credential, runtime, and kernel inventory persistence are separate operations. A single SQLite transaction is needed for publication; host networking and filesystem changes require a durable operation journal for rollback/reconciliation.
- Existing VM creation derives a MAC from the name and validates it against that derivation. Restore must preserve the archived MAC and check destination uniqueness. If that invariant is kept, exact restore must validate that the archived MAC is reproducible or extend restore-specific validation while retaining collision checks.
- Start resolves kernels through the destination artifact inventory. Restore must import and register the exact embedded kernel bytes with portable kernel metadata; it must not depend on source-host artifact paths or registry availability.
- Runtime binary paths, process state, sockets, and host resource ownership remain destination-local and are not archive inputs.

## Decisions

### Manifest and address policy

**Decision**: Version 2 declares an address policy and contains a conditionally shaped network section.

- **Preserve IPv4** stores guest IPv4, prefix, optional guest gateway, optional LAN IPv4, logical mode, LAN exposure, and guest MAC.
- **Regenerate IPv4** stores only the policy and LAN exposure from source network configuration. It omits source addresses, prefix, gateway, mode, and MAC.
- Both policies retain VM boot/resource settings, distribution and image provenance, SSH credentials, fixed payload names, digests, and portable kernel registry metadata.

**Reason**: The first policy reproduces the source assignment or fails on destination conflicts. The second allows local allocation without leaking source assignments into manifest metadata.

Version 1 and unknown/inconsistent version 2 policy shapes are rejected. IPv6 network values are unsupported. TAR member names remain fixed by the SDK and never determine output paths. The reader validates payload sizes/digests, key correspondence, stream framing, and authenticated age EOF before publication.

### Private snapshot transformation

**Decision**: Keep the existing running-VM capture view read-only and stable. For regenerate policy, create a separately named writable classic Device Mapper snapshot as a child, with its own loop-backed COW store. For a stopped VM, use a read-only loop over the source root disk as the child origin. Replay committed ext4 journal transactions in the child view, then call a narrow GuestStorage operation that removes only `/etc/systemd/network/10-taumaru.network`. Archive the child view. Do not mount or edit the source disk or read-only parent view read-write.

The capture is block-level crash-consistent, not application-consistent: it does not quiesce guest applications or coordinate database transactions. Guest applications rely on their normal recovery behavior. The child and parent views are separately validated for snapshot validity/overflow through the full archive read. Their names and loop identities include VM identity and operation identity. Cleanup follows ownership dependencies: remove child mapper, detach child COW loop, remove child COW backing file, then release the parent capture view/loop and its COW file. Any failed sanitization, journal replay, integrity check, or COW overflow fails the operation and prevents publishing the archive.

Before implementation treats nested classic snapshots as portable, add a privileged integration check on supported kernels. The Linux DM snapshot docs state that snapshot views are writable without modifying their origin and route changed chunks to a separate COW device, but they explicitly describe recursive snapshot depth for thin provisioning rather than guaranteeing every classic snapshot nesting arrangement. Kernel source shows a classic snapshot opens its origin for reading and directs snapshot writes through its COW mapping. If verified capability is absent, use an isolated full-disk regular-file copy of the stable parent view before sanitizing; check storage capacity and report the additional work. Do not silently fall back after arbitrary mapper, I/O, or COW errors.

**Reason**: This keeps edits isolated from a live VM and limits the normal sanitization work to changed filesystem blocks. A full copy is a correct fallback with a clear extra disk-space and I/O cost.

**Filesystem safety**: The current editor uses `debugfs` against ext4 images. Do not edit a crash-consistent child before journal recovery. Use ext4 tooling on the private writable child only; fail if journal recovery or the targeted managed-file operation cannot complete. Never scan or rewrite arbitrary guest files. If the managed file is absent or is not a regular file, surface a typed sanitization failure unless implementation proves an equivalent safe state. Check required host tools before starting mutations.

### Destination network behavior

**Decision**: Restore uses an internal policy-aware network request.

- Preserve policy passes the exact archived guest address/prefix/gateway, optional LAN address, logical mode, exposure, and MAC. Validate that these values are valid for the destination and unused by persisted or live allocations. Fail and roll back on any mismatch; do not silently substitute an address.
- Regenerate policy derives the logical mode from `expose_on_lan`, invokes the normal destination allocator for IPv4 and network identity, then writes that allocated configuration into the recovered root disk before the VM is published.
- Both policies create destination-owned TAP/bridge/host resources and ownership records on the destination. They never import source host interfaces, addresses, routes, leases, or ownership fingerprints.

**Reason**: Only guest-level assignment is portable. Host-side networking is coupled to the destination host. Restore must also prevent manifest/network-config drift by ensuring the guest config matches the persisted destination network.

### Archive, destination files, and kernel inventory

**Decision**: Recover root disk, kernel, and SSH key members to private operation staging with restrictive permissions. Check the authenticated archive and all payload records before installing managed files. Import the exact embedded kernel to the normal destination kernel cache and register it using archived identity/metadata. Rewrite the staged root disk's managed network file from the final network record for either policy; for preserve, use the exact archived values. This occurs after the archived rootfs hash is verified, so regenerate restores are host-local derived disks and their bytes intentionally differ from the archived payload.

**Reason**: The normal start workflow uses local kernel inventory; a file alone is insufficient. Rewriting the managed network unit aligns guest configuration with actual destination resources.

### Atomic publication and recovery

**Decision**: Serialize restores by archived VM name; use a private staging area and a durable journal before changing host state. Validate archive, disk capacity, runtime compatibility, name/path collisions, and policy data first. Apply destination networking with journaled ownership, install verified payloads without replacing existing files, write final guest network config, and publish VM, network, credential, stopped runtime, and kernel inventory in one SQLite transaction. Mark completion and clear the journal only after all required local state is durable. On failure/cancellation, roll back only operation-owned files, cache entries, and network resources. Reconcile journaled state before a later operation retries that VM name.

**Reason**: The database cannot transact atomically with files and Linux networking. The journal identifies exactly what to remove after an interruption and prevents incomplete VMs from appearing in inventory.

Do not call normal `create_microvm` as the restore implementation; it allocates a new disk, keys, and network assignment before replacing them.

## Alternatives Considered

- Store only source IPs in the manifest and leave them in the copied guest network file: rejected because regenerate restore would retain stale source settings before it can write destination values.
- Edit the source ext4 disk or running VM: rejected because it violates source immutability and can change guest behavior.
- Edit the read-only parent capture view: rejected because it is the stable backup point and must remain unchanged.
- Mount the source or parent ext4 view read-write: rejected because it can write/replay journal state into the source or captured parent.
- Copy the full disk for every sanitize operation: correct but rejected as the primary path due to an extra full-disk read/write and extra temporary storage; retained as fallback if child mappings are unavailable.
- Import source database rows, host network resources, executable files, or process metadata: rejected as host-specific state.
- Re-download the kernel during restore: rejected because it may be unavailable or may not match the snapshot digest.

## Primary References

- Linux Device Mapper snapshot behavior: [kernel documentation](https://docs.kernel.org/admin-guide/device-mapper/snapshot.html)
- Classic snapshot origin and COW implementation: [Linux kernel dm-snap.c](https://code.googlesource.com/linux/torvalds/linux/+/21e4675d9305f6ccd20b95d943882d607c8ae288/drivers/md/dm-snap.c)
- ext4 journal replay and check behavior: [e2fsck(8)](https://man7.org/linux/man-pages/man8/e2fsck.8.html)
- ext4 journal recovery implications: [ext4(5)](https://man7.org/linux/man-pages/man5/ext4.5.html)
- ext filesystem offline editing: [debugfs(8)](https://man7.org/linux/man-pages/man8/debugfs.8.html)
- age decryption and stream validation: [age Decryptor](https://docs.rs/age/latest/age/struct.Decryptor.html), [age scrypt Identity](https://docs.rs/age/latest/age/scrypt/struct.Identity.html), [age format](https://github.com/C2SP/C2SP/blob/main/age.md)
- TAR archive entry validation: [tar Entry](https://docs.rs/tar/latest/tar/struct.Entry.html)
- Zstandard streaming decode: [zstd read Decoder](https://docs.rs/zstd/latest/zstd/stream/read/struct.Decoder.html)
