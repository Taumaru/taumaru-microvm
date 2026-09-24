# Snapshot Archive Contract

## New archive format

- File extension remains .tmvmsnap.
- The archive is a Zstandard-compressed TAR stream with no encryption, password, or decryption key.
- The manifest format identifier remains taumaru.microvm.snapshot and the format version is 3.
- TAR members retain the fixed root-disk, guest-kernel, SSH-key, and manifest set. The manifest remains the final TAR member.
- Payload records retain member path, size, SHA-256 digest, and file mode.
- The Zstandard frame includes its content checksum. Restore also validates manifest payload digests and all current TAR and manifest invariants.
- These checks can detect accidental corruption. They do not establish archive authenticity or protect confidentiality.

## Legacy and invalid inputs

- Age-encrypted archives from the previous format are unsupported. Restore detects the age header and reports that the source VM must be snapshotted again in the unencrypted format.
- Restore does not request or accept a password and does not attempt decryption.
- Unsupported manifest versions, malformed TAR members, invalid payloads, truncation, checksum failures, and digest mismatches fail before the restored VM is committed.
- Failure removes operation-owned staging state and leaves the source archive and existing VMs unchanged.

## File access

- The output archive follows the operating system's default file-creation permissions and the destination directory's access policy, including its umask and default ACL behavior.
- The archive is created by the current elevated process flow. File ownership follows the effective creating process; no ownership transfer is part of this feature.
- The staged file is not linked to the requested output name until the archive is complete and synchronized. Existing output paths are never overwritten.
