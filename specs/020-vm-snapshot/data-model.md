# Snapshot Data Model

## Snapshot request

A public SDK request identifies exactly one managed VM, an output path, and the encryption password.

- VM name: validated using the existing VM-name rules and resolved through the local repository.
- Output path: caller supplied; the output must not already exist at final publication.
- Password: required and non-empty; never serialized into the manifest or emitted to diagnostics.
- Cancellation: optional caller-owned token, checked throughout streaming and before publication.

## Snapshot result

The successful result reports the VM name, final archive path, encrypted archive byte count, and whether the VM was running when the capture began. It does not expose host mapper names, loop devices, temporary paths, or credentials.

## Snapshot archive

The archive is one age-encrypted byte stream containing a Zstandard-compressed TAR archive. TAR member paths are relative and fixed by the archive version. The member list for version 1 is:

| Member | Content | Required mode |
| --- | --- | --- |
| payload/rootfs.ext4 | Exact logical root disk bytes from the snapshot view or stopped source file | 0600 |
| payload/kernel/vmlinux | Exact cached kernel artifact bytes needed by this VM | 0644 |
| payload/ssh/id_ed25519 | VM private SSH key | 0600 |
| payload/ssh/id_ed25519.pub | VM public SSH key | 0644 |
| manifest.json | Versioned metadata and SHA-256/size for every payload member | 0600 |

TAR member order places manifest.json last so payload digests can be calculated in a single sequential pass. File names and metadata remain encrypted inside the age stream.

## Manifest version 1

The manifest is a versioned, forward-compatible description of the portable VM. It includes:

- Format identifier, format version, and UTC creation time.
- VM name and stable local identity where portable; no source database ID is required.
- Guest architecture and compatibility requirements for a Linux host with KVM and a compatible Firecracker runtime.
- Requested vCPU count, memory bytes, and logical root-disk size.
- Distribution and image registry IDs and available provenance such as distribution name/version, image digest, and kernel ID.
- The exact guest kernel payload path, size, and SHA-256 digest.
- Boot root device and persisted distribution kernel arguments required by the guest.
- Portable boot/network intent, including host-only versus LAN exposure intent, guest MAC identity if required by the guest, and SSH user/port. Do not serialize the source host's addresses, tap name, DHCP lease, routes, or resource ownership records.
- SSH key type and public fingerprint, with key payload references. The private key content exists only in its encrypted TAR member.
- One payload record per required member with path, byte size, and lowercase SHA-256 digest.
- A consistency declaration that this is a disk-only, crash-consistent snapshot and does not include guest memory or in-memory process state.

Do not add source-host absolute paths, raw database rows, process IDs, control sockets, runtime loop or mapper names, host network resources, Firecracker/firectl binaries, password, or password-derived encryption material.

## Device Mapper resources

For a running VM, one ephemeral snapshot view consists of:

- Origin: the verified existing per-VM snapshot-origin mapping used by Firecracker.
- COW backing: an owner-only temporary file under the SDK home/tmp directory with enough capacity for the origin's data chunks, persistent snapshot metadata, and a reserve.
- COW loop: a loop device verified against that exact backing file.
- Snapshot mapping: a deterministic per-VM Device Mapper name and UUID with a table referencing the origin and COW loop.

The VM identity digest is derived from the canonical SDK home and VM name. Names and UUIDs are not persisted in SQLite. The active snapshot view is read-only from the SDK's perspective; the guest continues writing through the origin mapping.

## Capture states

| State | Meaning | Next states |
| --- | --- | --- |
| Preparing | Locks held, VM and local artifacts validated, output and temporary-resource paths checked | Capturing, Failed, Cancelled |
| Capturing | For a running VM, the origin is briefly suspended and the snapshot target is attached; for a stopped VM, a stable source file is opened | Streaming, Failed, Cancelled |
| Streaming | Required payloads are copied through hashing, TAR, Zstandard, and age into an encrypted temporary output | Validating, Failed, Cancelled |
| Validating | Archive writers are finalized, snapshot status and payload metadata are checked, temporary output is synced | Publishing, Failed, Cancelled |
| Publishing | Atomic no-clobber publication is attempted and the parent directory is synced | Published, Failed |
| Published | Final encrypted archive exists and temporary Device Mapper resources have been released | Terminal |
| Failed | Error returned; no incomplete final archive is accepted; owned resources are cleaned or cleanup failures are returned | Terminal |
| Cancelled | Cancellation error returned; no final archive is published; owned resources are cleaned or cleanup failures are returned | Terminal |

## Validation rules and invariants

- Never replace the persistent rootfs file or change the running VM's origin mapping.
- A running snapshot requires a verified live process and its already active, correctly owned snapshot-origin mapper. Do not silently create or replace that mapping under a running Firecracker process.
- A stopped snapshot requires liveness checks proving there is no active writer. It reads the rootfs directly and does not create an online snapshot view.
- Suspend the origin while adding the snapshot target and again while removing it, then resume it on every success and error path. Reconcile a verified owned origin left suspended after an interrupted operation before another lifecycle or snapshot operation proceeds. The Firecracker process remains active and block I/O resumes before bulk copying.
- Check target status during and after the copy. Invalid or Overflow state, read failure, insufficient space, cancellation, missing artifact, or integrity failure prevents publication.
- Cleanup removes the snapshot mapping before detaching the COW loop, and detaches the loop before unlinking its backing file. Never detach the VM's origin loop as part of snapshot cleanup.
- The complete TAR/Zstandard/age stream is finalized before publication. The destination is not overwritten if it appears during the operation.
- SDK errors are typed and silent. Only the CLI prompts, prints progress, and formats actionable diagnostics.
