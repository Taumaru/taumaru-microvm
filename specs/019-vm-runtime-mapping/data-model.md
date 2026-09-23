# Data Model: Per-VM Runtime Disk Mapping

## Persistent inventory

No persistent schema change is planned. The existing MicroVM inventory remains the source of durable identity and configuration.

| Field | Source | Lifetime | Rules |
|---|---|---|---|
| SDK home | MicroVmSdk constructor | Host-local | Normalize and canonicalize the existing SDK home for runtime identity generation. |
| VM name | Persisted VM record | Persistent | Validate using the existing VM-name validator. |
| rootfs_path | Persisted VM record | Persistent | Must remain the VM volume's existing regular rootfs.ext4 file. It is never replaced by the Device Mapper path in public results. |
| process_id | Existing vm_runtime row | Process lifetime | Record the spawned firectl identity before readiness completes; retain it if exit cannot be confirmed so stop can retry. |
| process_state | Existing vm_runtime row | Process lifetime | Use starting while waiting for readiness, running after readiness, and stopped only after process and mapping cleanup complete. On retry, adopt a starting process only when PID, executable identity, and ready socket agree; do not clear it merely because the socket is silent. No new table or column is needed. |

## Runtime resource entities

These entities represent kernel state only. They are not serialized to SQLite and may disappear during host reboot.

### Per-VM lifecycle lock

A deterministic lock file under the SDK-owned runtime directory serializes start and stop across separate SDK instances using the same canonical home and VM name. The lock is held for the full lifecycle transition, and its file is not unlinked after use. Acquisition uses nonblocking attempts with async waiting so lock contention does not block the Tokio executor.

### Runtime mapping identity

| Attribute | Meaning |
|---|---|
| canonical_home | Canonical SDK home path used to distinguish separate inventories on the same host. |
| vm_name | Validated name of the owner VM. |
| digest | SHA-256 digest over an unambiguous canonical_home and vm_name encoding. |
| mapper_name | Readable Device Mapper name containing a bounded VM-name slug and a digest suffix. It must fit Linux Device Mapper naming limits. |
| mapper_uuid | Stable owner UUID with a Taumaru prefix and the full digest. |

The name is for host inspection. The UUID and verified live mapping table are the ownership proof. A name match without a UUID and table match is a conflict.

### Loop association

| Attribute | Meaning |
|---|---|
| device_node | Current allocated endpoint such as /dev/loopN. It may change after reboot and is never treated as stable identity. |
| backing_path | The VM's persistent rootfs.ext4 path. |
| backing_file_identity | File device and inode, checked against the expected persistent file to detect a wrong or replaced backing file. |
| dm_dependency | The major/minor dependency referenced by the owned Device Mapper table. |

The loop is discovered from the Device Mapper dependency or the VM-unique backing file. The implementation must reject ambiguous or foreign loop associations rather than adopting them.

### Device Mapper mapping

| Attribute | Meaning |
|---|---|
| mapper_path | Host-visible /dev/mapper/<mapper_name> path passed internally to firectl. |
| target | One snapshot-origin target covering the full rootfs block range. |
| sector_length | Device Mapper sector count matching the loop device size. |
| loop_dependency | The loop device number currently backing the target. |
| open_count | Current kernel references reported for the mapper; nonzero means cleanup must not proceed. |
| ownership_state | Internal classification used for reuse, repair, conflict, and cleanup. |

The mapper path is not returned through MicroVmStartResult.rootfs_path or any existing public result field.

## Ownership states

| State | Evidence | Start behavior | Stop behavior |
|---|---|---|---|
| Absent | No expected mapper and no VM-specific loop association | Create loop, then mapper | Return success after normal process cleanup |
| ValidOwned | Expected UUID, snapshot-origin table, size, loop dependency, and backing-file identity all match | Reuse | Remove mapper, then detach verified loop after process exit |
| PartialOwnedUnused | Incomplete chain is attributable to this VM and has no live process or open holder | Remove only owned partial resources in dependency order, then recreate | Remove owned leftovers idempotently |
| Conflict | Name/UUID/table/backing identity disagrees, or ownership is ambiguous | Return typed conflict without mutation | Return typed conflict without mutation |
| InUse | Process is live or kernel open references remain | Do not detach or overwrite | Retain resources and return retryable typed error |

## Lifecycle transitions

- Stopped VM with absent mapping -> loop attached -> snapshot-origin mapper verified -> firectl process starting -> socket ready -> process state running.
- Any setup failure before process spawn -> clean up only resources created by that attempt; preserve rootfs.ext4.
- Readiness failure after spawn -> terminate and confirm process exit -> remove mapper -> detach loop -> mark stopped.
- If process exit cannot be confirmed -> retain process identity and mapping; stop can retry later.
- Running VM -> graceful stop or existing forced-stop path -> process and socket exit verified -> mapper removed -> loop detached -> stopped metadata persisted.
- Host reboot -> kernel mappings absent while inventory/rootfs persist -> next start recreates the loop and mapper from deterministic identity.
- SDK interruption after persisting starting -> next operation validates the recorded PID and executable; a ready process is adopted, a live but unready process retains resources and returns a retryable error, and only a confirmed exited process may be cleaned up.
