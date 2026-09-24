# SDK Contract: Snapshot and Restore

## Snapshot creation

All public snapshot methods operate without an archive-password argument:

- create_snapshot(vm_name, output_path, address_policy)
- create_snapshot_with_cancellation(vm_name, output_path, address_policy, cancellation)
- create_snapshot_with_cancellation_and_progress(vm_name, output_path, address_policy, cancellation, on_progress)

They retain existing VM selection, address policy, cancellation, caller-owned progress, lifecycle locks, consistency, and typed-error behavior.

SnapshotResult reports vm_name, output_path, archive_size_bytes, and source_was_running. The encrypted_size_bytes field is removed. Public Rustdoc explains that the archive is unencrypted and readable by accounts that can access the file.

## Restore

RestoreRequest contains archive_path only. All restore methods operate without a password input and leave the restored VM stopped on success.

Restore continues to validate the archive and destination, stage files safely, verify payload integrity, use the existing durable restore journal, and commit only a complete VM. A legacy age-encrypted archive returns the existing typed RestoreArchive error with an actionable reason to create a new unencrypted snapshot. Restore performs no legacy decryption. Update the public error documentation so it no longer implies that restore decrypts archives.

## Public API migration

The password-parameter removal, RestoreRequest field removal, and SnapshotResult field rename are source-breaking changes. Publish them with the planned workspace version 0.2.0 and migration guidance from 0.1.0:

1. Remove the password argument from snapshot calls and the password field from RestoreRequest construction.
2. Replace reads of encrypted_size_bytes with archive_size_bytes.
3. Recreate archives produced by the previous encrypted format before restoring them.
