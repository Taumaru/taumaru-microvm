# Snapshot and Restore Data Model

## Snapshot request

The SDK snapshot request accepts:

- A source VM name.
- An output path.
- A non-empty password.
- A required typed IPv4 address policy: preserve source IPv4 or regenerate on restore.

The SDK does not prompt, log, or write output. The CLI asks for the policy interactively and warns that preserving source addresses can conflict during restore. A non-interactive CLI call must supply the policy explicitly.

## Version 2 archive manifest

The manifest declares format identifier, version 2, creation time, archived VM name, CPU/memory/disk limits, guest architecture, boot configuration, distribution and image provenance, kernel registry metadata, SSH credential metadata, payload records, and an explicit network policy.

Portable kernel metadata includes ID, name, display name, version, architecture, registry path and URL, filename, format, MIME type, and modified time. The payload record is authoritative for kernel byte count and SHA-256. Registry path/URL are provenance and are not used to fetch artifacts during restore.

Network data is a discriminated policy:

| Policy | Required manifest network values |
| --- | --- |
| Preserve IPv4 | Policy, guest IPv4, prefix length, optional guest gateway, optional LAN IPv4, logical mode, LAN exposure, and guest MAC |
| Regenerate IPv4 | Policy and LAN exposure only |

The regenerate branch MUST NOT serialize guest/LAN addresses, prefix, gateway, source mode, or source MAC. Both branches reject IPv6 network values. Version 1 and unknown versions are rejected.

## Snapshot disk view lifecycle

| State | Meaning | Next states |
| --- | --- | --- |
| Validating | Validate VM, policy, output, password, filesystem tools, and snapshot prerequisites | Capturing, Failed |
| Capturing | Select the stable source: read-only Device Mapper capture for a running VM or stopped source disk view | PreparingPrivateCopy, Archiving, Failed |
| PreparingPrivateCopy | For regenerate policy, create an owned writable child CoW view; recover committed ext4 journal state and remove only the managed Taumaru network file | Archiving, Failed |
| Archiving | Stream root disk, kernel, and SSH files into encrypted archive; hash bytes and monitor all active DM snapshots | Finalizing, Failed, Cancelled |
| Finalizing | Close/authenticate and flush temporary archive output; release child then parent views; atomically publish only after cleanup succeeds | Completed, Failed |
| Completed | Archive is published; source VM/disk remains unchanged | Terminal |
| Failed/Cancelled | Remove partial archive and operation-owned mapper/loop/COW/staging state; leave source unchanged | Terminal |

The private ext4 operation is fixed to `/etc/systemd/network/10-taumaru.network`; it has no caller-provided guest path. It first replays committed ext4 journal transactions on the private view, then removes the managed file. This is normal filesystem crash recovery, not arbitrary guest-file sanitization. It fails if journal preparation or the expected regular managed file cannot be processed. It never scans arbitrary guest files. If a nested classic DM view is not supported by a verified capability check, a private regular-file copy of the stable view may be used if capacity is sufficient. All DM and loop identities include VM identity plus an operation identity; no persistent database record is required to identify ownership.

## Archive members and local destinations

| Archive member | Destination after restore | Mode |
| --- | --- | --- |
| `payload/rootfs.ext4` | `vms/<name>/rootfs.ext4` | 0600 |
| `payload/kernel/vmlinux` | `artifacts/kernels/<kernel-id>/<filename>` | 0644 |
| `payload/ssh/id_ed25519` | `vms/<name>/ssh/id_ed25519` | 0600 |
| `payload/ssh/id_ed25519.pub` | `vms/<name>/ssh/id_ed25519.pub` | 0644 |
| `manifest.json` | Parsed from authenticated stream; not installed | 0600 |

The reader maps exact member names to SDK-selected staging files. Archive-supplied paths are never joined to a destination. It rejects duplicate, missing, extra, linked, non-regular, unsafe, oversized, or hash-mismatched members.

The archived root disk hash/size is verified before restore writes destination network configuration. A regenerate-policy restored disk intentionally differs from the archived payload after the fixed managed network file is written with destination-local values.

## Destination records

- **MicroVmRecord**: archived VM identity and portable boot/resource/distribution/image/kernel IDs, plus destination-local volume, disk, and socket paths.
- **PersistedNetwork**: exact archived values for preserve policy, or newly allocated values for regenerate policy; destination-owned TAP/bridge/host resources and ownership records are created locally.
- **PersistedCredential**: destination-local key paths, archived SSH username/port/type/fingerprint, and restrictive private-key permissions.
- **PersistedRuntime**: destination-local executable/socket paths, no process ID, and stopped state.
- **Kernel artifact inventory**: verified embedded kernel registered at its destination cache path with archived portable identity and metadata.

Do not import source absolute paths, database row IDs, process state, sockets, loop/Device Mapper names, interface names, host addresses, routes, leases, or network ownership fingerprints.

## Restore lifecycle

| State | Meaning | Next states |
| --- | --- | --- |
| ValidatingInput | Validate request, archive path, manifest/version/policy, destination capabilities, archived name, and conflicts | Staging, Failed |
| Staging | Decrypt and decompress exact fixed members into operation-private files while calculating sizes/digests | Verifying, Failed, Cancelled |
| Verifying | Check hashes, SSH key correspondence, kernel metadata, archive framing, and authenticated EOF | PreparingDestination, Failed, Cancelled |
| PreparingDestination | Check paths, runtime compatibility, storage, and network values; persist the durable journal before mutations | Installing, Failed, Cancelled |
| Installing | Install verified payloads without replacement, configure destination network resources, and rewrite only the managed network file in the staged rootfs | Committing, Failed, Cancelled |
| Committing | Commit VM, network, credential, stopped runtime, and kernel inventory together; finalize journal | Restored, Failed |
| Restored | VM is discoverable under its archived name and stopped | Terminal |
| Failed/Cancelled | Remove or journal-reconcile only operation-owned files, kernel cache entries, and network resources | Terminal |

## Invariants

- Snapshot creation does not stop a running VM. Its read-only parent view represents one stable block-level, crash-consistent point in time; guest writes continue against the origin. It does not quiesce applications or coordinate database transactions, so guest applications use their normal recovery behavior.
- Sanitization writes go only to the private child CoW or isolated full copy. The source disk, active VM, and read-only capture parent remain unchanged.
- Regenerate snapshots omit source address assignments from both network metadata and the fixed managed network file in the archive copy. Arbitrary files are not scanned or rewritten and may contain IP text.
- Regenerate restore derives logical mode from LAN exposure, allocates destination-local IPv4/network identity, and writes it into the recovered root disk before publishing the VM.
- Preserve restore either reproduces the archived guest/LAN IPv4 settings and MAC or fails atomically; it does not silently substitute them.
- Host interfaces, routes, leases, and ownership are destination-local under both policies.
- Restore verifies archived rootfs bytes before any intentional destination network rewrite.
- Untrusted archive paths are never used for filesystem writes.
- The VM becomes discoverable only after all required payloads and local records are verified and committed; it remains stopped.
- Failure/cancellation preserves the archive and pre-existing VMs and removes only operation-owned state.
- SDK methods return typed errors and remain silent; CLI owns prompts, progress, and diagnostics.
