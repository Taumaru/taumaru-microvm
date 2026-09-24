# Data Model: Unencrypted MicroVM Snapshots and Restore

## Snapshot Archive

A version 3 portable archive written as a Zstandard frame containing a TAR stream. It has no archive password, encryption key, or encrypted container layer.

The TAR stream retains the existing fixed payload members:

- payload/rootfs.ext4: captured root disk.
- payload/kernel/vmlinux: exact guest kernel.
- payload/ssh/id_ed25519: private SSH key.
- payload/ssh/id_ed25519.pub: public SSH key.
- manifest.json: portable VM, compatibility, boot, network, SSH, consistency, and payload metadata.

Each payload record contains its member path, logical size, SHA-256 digest, and file mode. The Zstandard frame checksum detects accidental compressed-stream corruption. Neither checksum provides authenticity or confidentiality.

### Validation rules

- The format identifier remains taumaru.microvm.snapshot and the manifest version becomes 3.
- Restore accepts only version 3 plain archives.
- Age-encrypted legacy input is rejected before staging; no password or decryption attempt is available.
- TAR member paths, member types, member counts, sizes, manifest fields, payload digests, and SSH key correspondence retain existing validation.
- Archive, root-disk, kernel, key, and manifest size limits remain unchanged.
- The snapshot result reports archive_size_bytes after compression.

## Snapshot Request and Result

A snapshot request identifies a MicroVM, output path, and address policy. It has no archive-password field or parameter.

SnapshotResult contains the VM name, final output path, archive_size_bytes, and whether the source VM was running when capture began. Existing progress and cancellation contracts remain; progress descriptions refer to archive preparation, payload streaming, and finalization without encryption terminology.

The archive is created with operating-system default file permissions and the destination directory access policy. Under the current elevated command flow, the effective creating process determines ownership and mode defaults.

## Restore Request and Result

RestoreRequest contains only the archive path. RestoreResult retains the existing restored VM identity, stopped state, destination-local paths, network settings, and SSH connection data.

## State and Identity

- Snapshot does not create a second VM or change the source VM's running/stopped state.
- Restore uses the VM name and portable settings from the validated manifest.
- Restore succeeds only after payload and destination setup have completed; the restored VM remains stopped.
- A duplicate name, unsupported format, corrupt archive, or unmet destination prerequisite leaves existing VMs unchanged and publishes no partial VM.
- Restore retains the current durable journal and operation-owned cleanup behavior. No inventory database schema change is required.
