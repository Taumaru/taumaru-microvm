# Research: Unencrypted MicroVM Snapshots and Restore

## Decision 1: Write a plain, compressed archive format

**Decision**: Keep the existing TAR member layout and Zstandard compression, but remove the age-encryption wrapper. Write the new manifest with format version 3. Keep the archive identifier and manifest fields except for the version number.

**Rationale**: The current writer streams payloads through TAR, Zstandard, and age. Removing only the outer age layer eliminates passwords and encryption while retaining compression, bounded streaming, manifest metadata, and payload layout. The container framing changes, so version 3 distinguishes the new plain format from the previous encrypted format.

**Alternatives considered**:

- Keep manifest version 2 and infer the container type solely from its leading bytes. Rejected because the externally portable archive framing changed and should have an explicit version boundary.
- Remove compression and write a plain TAR. Rejected because the user requested no encryption, not removal of the existing compression behavior, and uncompressed VM disks would be larger.
- Keep age as an optional mode. Rejected because the requested contract removes password and encryption options entirely.

**Evidence**: The current writer in crates/sdk/src/adapters/archive/age_tar_zstd.rs wraps a Zstandard encoder in age encryption. The manifest in crates/sdk/src/adapters/archive/manifest.rs currently declares version 2. The SDK already validates a fixed TAR member set and records per-payload size and SHA-256.

## Decision 2: Reject old encrypted archives before staging

**Decision**: Inspect the archive prefix before creating restore staging files. Recognize the age encryption header and return the existing typed restore-archive error with a message that the encrypted format is unsupported and the source VM must be snapshotted again. Do not decrypt or migrate old archives.

**Rationale**: Passwordless restore cannot recover a passphrase-encrypted archive. Early detection gives an actionable compatibility error and avoids creating restore staging state.

**Alternatives considered**:

- Continue decrypting legacy archives if the caller supplies a password. Rejected because it contradicts the passwordless restore contract.
- Report a generic decompression or malformed-archive error. Rejected because users would not know that a new snapshot is required.
- Add a new public SDK error variant. Rejected because the existing RestoreArchive error already represents archive parsing and validation failures; reusing it avoids an unrelated public error-enum change.

**Evidence**: crates/sdk/src/adapters/archive/restore.rs currently requires an age header and password before it reads the Zstandard/TAR stream. crates/sdk/src/error.rs already defines RestoreArchive with an operation and reason.

## Decision 3: Preserve accidental-corruption checks without claiming authenticity

**Decision**: Retain manifest payload sizes and SHA-256 digests, the existing member and size validation, and enable the Zstandard frame content checksum. Restore verifies all of them before committing the VM. Documentation must state that these checks detect accidental corruption but do not provide confidentiality or authenticity.

**Rationale**: Age currently authenticates the complete encrypted stream. Removing it removes that authentication guarantee. A Zstandard checksum covers compressed-frame corruption, while the existing payload digests and strict TAR validation continue to detect incomplete or inconsistent payloads. These unkeyed checks cannot prove who created an archive or prevent a deliberate editor from recomputing its hashes.

**Alternatives considered**:

- Rely only on payload SHA-256 fields. Rejected because they do not cover compressed framing and TAR metadata.
- Add signatures or another encryption/authentication scheme. Rejected because it expands the trust model and is not requested; the archive must remain unencrypted.

**Evidence**: zstd 0.13.3 is the workspace version and its installed encoder exposes include_checksum. The current TAR writer calculates SHA-256 as each payload is streamed. The current reader verifies declared size, digest, member set, manifest, and key-pair validity.

## Decision 4: Remove password fields from public and CLI contracts

**Decision**: Remove the password argument from all snapshot SDK methods, remove the password field from RestoreRequest, remove the CLI password option and prompts, and rename SnapshotResult.encrypted_size_bytes to archive_size_bytes. Keep cancellation, progress, path selection, VM selection, and address-policy behavior. Update public Rustdoc and human progress text to use archive terminology. Add a crate-level SDK migration note in crates/sdk/src/lib.rs.

**Rationale**: Keeping an ignored or optional password field would still expose a password surface and contradict the no-password contract. The result field also must not retain a false encryption claim.

**Alternatives considered**:

- Keep a deprecated password argument that is ignored. Rejected because callers would still be offered a password contract that does nothing.
- Keep encrypted_size_bytes as a compatibility alias. Rejected because its meaning becomes incorrect and would preserve a misleading public API.

**Migration decision**: The public SDK changes are source-breaking. The current shared workspace version is 0.1.0; plan a pre-1.0 minor version bump to 0.2.0 and publish migration guidance explaining the removed password inputs, renamed size field, and need to recreate legacy archives.

**Evidence**: Snapshot methods in crates/sdk/src/manager.rs take password parameters. RestoreRequest in crates/sdk/src/domain/restore.rs contains password. SnapshotResult in crates/sdk/src/domain/snapshot.rs exposes encrypted_size_bytes. The workspace version is 0.1.0 in Cargo.toml. The repository constitution requires explicit versioning and migration guidance for breaking public changes.

## Decision 5: Follow the selected output-directory permission policy

**Decision**: Create the hidden staging archive in the output directory using normal file-creation permissions, allowing the operating system umask and directory default ACL to set access. Keep the existing no-overwrite hard-link publication and cleanup behavior; publish the requested filename only after archive finalization and sync.

**Rationale**: The current staging helper explicitly sets mode 0600, and hard-link publication preserves that mode. Removing the explicit mode makes the resulting archive follow the operating system and destination-directory policy selected in the clarification.

**Alternatives considered**:

- Keep mode 0600. Rejected because it overrides the selected system and directory defaults.
- Add a new owner-only mode or an ownership-transfer step. Rejected because neither was requested and both would introduce a different access policy.

**Operational note**: Snapshot commands currently run through privilege escalation. Normal ownership and mode defaults are therefore those of the effective creating process and destination directory. No ownership rewrite is planned.

## Decision 6: Reject the removed CLI option and do not forward secrets

**Decision**: Remove --password from snapshot and restore arguments, help, prompts, non-interactive validation, and elevated child-command arguments. Preserve existing privilege escalation and all non-password arguments. Confirm with command-surface coverage that the removed option fails and its supplied value is not echoed.

**Rationale**: Password prompts and forwarding currently occur in multiple CLI paths, including privilege re-execution. Removing the option at parsing prevents all paths from receiving a password and keeps secret text out of child process arguments.

**Alternatives considered**:

- Accept --password and ignore it. Rejected because the specification says the option is unsupported.
- Keep the flag only for encrypted archive compatibility. Rejected because restoring old archives with a password is explicitly unsupported.

**Evidence**: CLI definitions and behavior are in crates/cli/src/cli.rs and commands/snapshot.rs / commands/restore.rs. Progress and success wording is in crates/cli/src/output/human.rs. Current command-surface tests encode password help and required-password behavior and must be revised.
