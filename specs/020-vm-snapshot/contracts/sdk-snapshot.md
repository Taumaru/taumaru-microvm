# SDK Snapshot Contract

## Public operation

Add a documented operation on MicroVmSdk with this semantic signature:

    create_snapshot(vm_name: &str, output_path: &Path, password: &str) -> Result<SnapshotResult, SdkError>

Add a cancellation-aware variant accepting a public SnapshotCancellation token. The simple method delegates to the same implementation with an uncancelled token. Export the result and cancellation types from the SDK facade and document their public behavior with Rustdoc.

SnapshotResult contains the VM name, final output path, encrypted archive byte count, and whether the source VM was running at capture start. It contains no temporary device paths or secret data.

## Preconditions and behavior

- The VM exists, is complete, and has locally available root disk, kernel, boot metadata, and SSH credentials.
- A running VM has a verifiable persisted process and the already active Device Mapper snapshot-origin path used by Firecracker.
- A stopped VM has no live process or active writer and is copied from its stable root disk.
- The output parent exists and is writable; the destination does not exist when publication occurs.
- The password is non-empty. The SDK never returns or logs it.
- The SDK acquires the existing per-VM lifecycle lock and volume lock for the full capture and cleanup.
- The SDK creates one versioned TAR, compresses it with Zstandard, encrypts with age passphrase streaming, and publishes one file without replacing an existing destination.
- The SDK never stops Firecracker or changes the persistent rootfs. Online capture can briefly queue guest disk I/O while the Device Mapper boundary is installed and removed. Guest vCPUs keep running.
- The archive represents the root-disk state at the capture boundary, not a live memory checkpoint or application-quiesced database transaction.

## Failure and cleanup contract

Return a typed SdkError for unknown or incomplete VM, transition or liveness conflict, missing or invalid mapper, missing kernel or credentials, invalid request, output conflict, file-system or COW capacity failure, Device Mapper/loop command failure, COW Invalid/Overflow state, stream/encryption finalization failure, integrity failure, cancellation, or cleanup failure.

On failure:

- Do not publish an incomplete archive at the requested path.
- Leave the persistent root disk untouched and leave a live VM running.
- Remove the snapshot mapping before its COW loop, and remove the loop before deleting its temporary file.
- Preserve any backing file still referenced by a loop or mapping, and report cleanup errors together with the primary failure.
- Never print, prompt for, or log through stdout/stderr/tracing.

## Format contract

The file is one age passphrase-encrypted stream containing a Zstandard-compressed TAR archive. Version 1 payload members and manifest fields are defined in specs/020-vm-snapshot/data-model.md. Decryption/authentication, supported-version checks, member completeness, sizes, and digests must all succeed before a future reader accepts a snapshot. This feature creates the archive; it does not implement import or restore.

## Threading and cancellation

Archive I/O uses bounded streaming and must not monopolize an async executor worker. Check cancellation between bounded reads and before publication. Cancellation is cooperative: it returns only after snapshot resources and temporary output have been cleaned or a cleanup error has been returned. The owned blocking worker retains the lifecycle lock and cleanup guard while archive I/O runs. Dropping the public future signals cancellation to that worker; it stops between bounded reads and cleans resources before releasing the lock.
