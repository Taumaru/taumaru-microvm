# Snapshot Design Research

## 1. Live disk capture with Device Mapper

**Decision**: Use the existing per-VM snapshot-origin mapping as the write interception point. During an online snapshot, create a sibling classic Device Mapper snapshot target whose origin is that active mapping and whose COW device is a private temporary file exposed through a verified loop device. Use a persistent COW store with the overflow-status feature, expressed as PO, where the host kernel supports it; fall back to P if the optional O status feature is unsupported. Use a 64-sector (32 KiB) chunk size.

The current SDK runtime already attaches the guest disk through a Device Mapper snapshot-origin target backed by the rootfs loop device. Firecracker must continue to write through that mapping. A separately created snapshot target reads old chunks from its COW device and unchanged chunks from the origin. Guest writes made after capture are redirected through snapshot-origin so that the old chunk is copied into COW before the origin changes.

The online sequence is:

1. Acquire the existing per-VM lifecycle lock and the volume target lock. Resolve the VM and verify its process identity and current mapping.
2. Refuse to repair or replace a missing or mismatched mapping while Firecracker is live.
3. Create and preallocate the private COW file, attach a loop device by its exact backing path, and verify both resources.
4. Suspend the existing snapshot-origin mapper so in-flight block I/O drains and new I/O queues. While the mapper is suspended, create the sibling snapshot target over the origin mapper and COW loop, then resume the origin mapper immediately.
5. Stream reads from the snapshot target. Continue checking the target status and stop on a read error, Invalid state, or Overflow state.
6. Finish and validate the archive. To remove the snapshot target, suspend the existing snapshot-origin mapper again, remove the sibling target, and resume the origin. Then detach only the verified COW loop and remove the temporary COW file. Leave the live origin mapping and Firecracker process untouched.

The kernel documentation states that snapshot-origin preserves old chunks in each snapshot's COW device, that a full COW device invalidates the snapshot, and that the corresponding origin must be suspended while snapshot targets are loaded or unloaded. The optional O feature makes overflow explicit in status; when it is unavailable, P status plus read errors still cause the SDK to reject the archive. A suspend briefly queues disk I/O; it does not stop the Firecracker process or vCPUs. This produces a coherent block-level point-in-time view, not application-level quiescence. A guest database may need its usual filesystem or database recovery after boot.

The PO store keeps exception metadata on disk instead of keeping all transient exception metadata in kernel memory. This is a better default for a long stream on a large disk, where the set of changed chunks can grow. The COW capacity policy should budget for all root-disk data chunks plus persistent-store metadata and a safety reserve; allocation must be checked before capture. High write rates can still exhaust it, so a successful archive requires both successful reads and a final healthy status check. A full COW must fail the operation rather than publish a partial archive.

Deterministic Device Mapper names and UUIDs derive from the canonical SDK home and VM identity. Use a separate snapshot suffix from the existing runtime mapping. The COW file name also contains that identity. Identify loop devices by exact backing file path and verify the Device Mapper table and UUID before adopting or removing resources. This supports multiple VMs and recovery of operation-owned leftovers without consulting the database for ephemeral host resources.

**Alternatives considered**:

- Directly copying the rootfs file while the VM runs is unsafe because later writes can race reads.
- FICLONE/reflink is filesystem-specific and cannot create a stable block-device view on filesystems without clone support.
- N on the snapshot target avoids persistent COW metadata, but stores the exception map in kernel memory. That can grow with the number of distinct chunks changed during a long capture; PO keeps that metadata in the COW store and exposes overflow status.
- LVM thin snapshots require provisioning and managing an LVM thin pool, which is not part of the current SDK storage model.
- Suspending or stopping Firecracker for the whole copy would avoid concurrent writes but violates the online-capture requirement.
- A filesystem freeze is not a substitute for preserving a long-lived point-in-time block view while the copy continues.

**Operational constraints**: The host needs the Device Mapper snapshot target and loop support, available COW space, and privileges. The COW file contains old plaintext disk chunks while capture runs. Keep it in the SDK-owned temporary directory with mode 0600; do not package it in the archive; delete it only after the snapshot mapping and loop association are removed. Cleanup failures must be reported and must preserve any backing file still referenced by a device.

References:

- Linux kernel, Device-mapper snapshot support: https://docs.kernel.org/admin-guide/device-mapper/snapshot.html
- dmsetup manual: https://man7.org/linux/man-pages/man8/dmsetup.8.html

## 2. Single-file archive, compression, encryption, and integrity

**Decision**: Encode a TAR archive, compress it as a Zstandard stream, then encrypt that stream with the age passphrase format. The result is one opaque file using the CLI default extension .tmvmsnap. Use streaming APIs and a bounded I/O buffer throughout.

The archive contains payload members first and a versioned manifest last. Hash each payload while it is copied into TAR, then write the final manifest with payload sizes and SHA-256 digests. This avoids a second full-disk read solely to calculate hashes. The root disk is a block device view, so copy its exact logical size; do not rely on sparse TAR detection for a mapped block device.

Finish the nested writers in order: TAR, Zstandard, and age. The age finalization step writes its authenticated final chunk; a failed finalization, flush, or sync fails the snapshot. Write to a randomly named mode-0600 temporary file adjacent to the requested destination. Sync it, then publish with an atomic no-clobber operation and sync the destination directory. Never create a plaintext TAR or compressed temporary archive. Recheck the final path at publication to handle races.

Age provides passphrase-based streaming encryption and authenticated ciphertext. Its format can be read by age-compatible tools such as rage. TAR member hashes provide an additional payload-level check after decryption. A future reader must treat decryption/final authentication, supported manifest version, complete required members, declared sizes, and hashes as one validation step; restore/import itself is outside this feature.

**Alternatives considered**:

- A custom encrypted archive format built directly on an AEAD library would create unnecessary cryptographic format and versioning responsibilities.
- Encrypting after writing an unencrypted archive would leave a large plaintext temporary file.
- Compressing after encryption is ineffective because ciphertext is not compressible.
- Storing the VM database or absolute file paths would couple the archive to the source host and export unrelated host state.

References:

- age Rust crate documentation: https://docs.rs/age/latest/age/
- age streaming Encryptor documentation: https://docs.rs/age/latest/age/struct.Encryptor.html
- age format specification: https://github.com/C2SP/C2SP/blob/main/age.md
- rage implementation: https://github.com/str4d/rage
- TAR Builder: https://docs.rs/tar/latest/tar/struct.Builder.html
- Zstandard streaming encoder: https://docs.rs/zstd/latest/zstd/stream/write/struct.Encoder.html

## 3. Portable metadata without a source-host dependency

**Decision**: Build the manifest from the stored MicroVM record, persisted logical configuration, a new read-only local repository query for distribution boot data, and the exact local kernel artifact. Do not fetch the registry during snapshot creation.

SQLite already stores the VM's distribution, image, and kernel identifiers; CPU, memory, and disk sizing; distribution root device and kernel arguments; and logical network and SSH configuration. Add a repository getter for the relevant boot and artifact provenance fields. The repository already has the cached-kernel resolver, so the SDK can verify and stream the exact kernel used by this VM. Hash the actual kernel payload while streaming it. No schema change or migration is expected.

The manifest stores portable identity and resource configuration, distribution/image/kernel IDs and provenance, guest architecture, boot root device and portable kernel arguments, logical network mode and portable guest settings, SSH user/port/key type/fingerprint, and payload paths/sizes/hashes. Include only source runtime family/package provenance needed to describe compatibility; do not include executable bytes.

Exclude the complete SQLite database, source absolute paths, host IP addresses, tap names, DHCP leases, network resources and ownership fingerprints, process IDs, sockets, loop names, Device Mapper names, and Firecracker/firectl binaries. Network intent is portable; host-specific addresses and resources are recreated by a future restore operation.

The archive includes the VM's private and public SSH key files. Validate that the files are regular, readable, and consistent with stored credential metadata before exporting; preserve restrictive private-key permissions in TAR. The encrypted archive protects the key material in transit and at rest as one file.

**Alternatives considered**:

- Copying the SQLite database would leak source-host inventory and runtime ownership and would make restore depend on source schema/state.
- Consulting the current registry manifest could fail offline and could capture metadata that changed after VM creation. Use persisted local metadata to describe the machine that exists.
- Exporting only a kernel registry ID would make the snapshot dependent on the destination registry retaining that artifact. Include the exact cached kernel bytes.
- Exporting the whole SDK home would package host tools, unrelated VMs, caches, runtime files, and secrets not needed for this VM.

References:

- Repository schema and methods: crates/sdk/src/adapters/persistence/sqlite.rs
- Persisted VM and network types: crates/sdk/src/domain/microvm.rs
- Kernel artifact resolution: crates/sdk/src/ports/repository.rs

## 4. Per-VM lifecycle, cancellation, and cleanup

**Decision**: Hold the existing cross-process per-VM lifecycle lock and the current in-process volume target lock until the snapshot view is removed and output publication or failure cleanup is complete. Reuse the existing runtime-disk boundary for create, verify, status, and remove operations on the snapshot resources.

The start and stop paths already acquire the same lifecycle lock. Holding it for the full export prevents that VM from stopping, restarting, or changing its root-disk mapping while the snapshot is read. Other VMs use independent lock identities and remain operable. The online path validates the Firecracker process against persisted runtime metadata and validates the active snapshot-origin table. If live state is missing or inconsistent, return a typed conflict instead of repairing the VM's mapping.

Expose a cancellation token patterned after the existing SDK download cancellation API. Poll it during bounded streaming and status checks. Cancellation follows the same path as any other failure: do not publish the destination, remove owned snapshot resources, and preserve the source disk and live process. The CLI must allow a graceful signal path through privilege escalation so the child can observe cancellation and clean up before a forced termination fallback. A host crash or forced kill can leave ephemeral resources; the next operation inspects deterministic names, UUIDs, loop backing paths, and the private COW filename. It resumes a verified owned origin mapper left suspended by an interrupted boundary operation, removes only a matching snapshot target, and preserves any COW file still referenced by a device.

**Alternatives considered**:

- Writing snapshot state into SQLite is unnecessary for a short-lived operation, introduces migration and recovery states, and conflicts with the existing principle that ephemeral runtime resources are verified against host state.
- Locking only while the snapshot is created would permit stop or mapper teardown during the export.
- Removing any resource that merely matches a VM name is unsafe. Verify canonical identity, UUID/table, backing file, and dependencies before cleanup.
- Killing the elevated child immediately on Ctrl-C bypasses SDK cleanup and can leave COW and Device Mapper resources behind.

References:

- Existing lifecycle locking and Device Mapper ownership: crates/sdk/src/adapters/runtime/device_mapper.rs
- Existing start and stop orchestration: crates/sdk/src/manager.rs
- Existing cancellation API pattern: crates/sdk/src/domain/artifact.rs
