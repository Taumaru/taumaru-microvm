# Phase 1 Data Model: Delete MicroVM

## Input

### Delete intent

The operation takes only a VM name. The SDK home comes from the constructed `MicroVmSdk`; the
volume directory and every owned reference are read from the inventory record, never from the
caller.

| Field | Type | Rules |
|---|---|---|
| `name` | `&str` | 1–64 ASCII characters; first character alphanumeric; remaining alphanumeric, `-`, or `_`. Validated with the existing `validate_vm_name` helper before any host work. |

No volume-path input exists. No request struct is introduced; the name is passed directly, mirroring
`start_microvm` / `stop_microvm`.

## Records read

### `MicroVmRecord` (existing, read-only for delete)

One durable row identifies one independently managed VM. Delete reads `name`, `volume_path`,
`rootfs_path`, and `socket_path` for containment checks and removal targeting; it never modifies
these fields. Unlike start and stop, delete deliberately skips the `require_complete` gate:
incomplete creation leftovers are deletable, and absent child rows simply skip their step.

### `PersistedNetwork` (existing, read-only probe input for release)

When present, the persisted attachment supplies the ownership-scoped release set: `config`
(tap name, mode, addresses, uplink/bridge names) plus the ownership flags and resource
fingerprints the adapter recorded at creation. When absent (incomplete record), the network
release step is skipped entirely.

### `PersistedCredential` (existing, shape-check input only)

When present, the recorded `private_key_path` / `public_key_path` are checked against the
expected volume-internal shapes (`{volume}/ssh/id_ed25519`, `{volume}/ssh/id_ed25519.pub`).
Key contents are never read; on shape mismatch the operation refuses with `StorageConflict`
and the record is kept. When absent, the shape check is skipped. The keys themselves are
removed as part of the whole volume-directory removal, never individually.

### `PersistedRuntime` (existing, read for probe context only)

Supplies no deletion input beyond what the record already carries. No runtime write ever
happens on the delete path: the row is removed by cascade with the record, never reset.

## Deletion set

### Whole volume directory (removed second)

| Item | Expected shape | Absent handling |
|---|---|---|
| Disk copy of the distribution image | `{volume}/rootfs.ext4` | Path shape still enforced when the record claims it; a missing file is already removed |
| Control socket file(s) | `{volume}/firecracker.sock` | Missing is already removed |
| SSH key files | `{volume}/ssh/id_ed25519`, `{volume}/ssh/id_ed25519.pub` | Missing is already removed |
| Logs and temp parts (`firecracker.log`, `.rootfs.*.part`) | Anywhere under `{volume}/` | Missing is already removed |
| The volume directory itself | `{volume}/` under `{home}/` | Missing is already removed |

Containment invariants checked before any removal: `volume_path` is absolute, starts with the
SDK home, and is not the home itself; `rootfs_path` and `socket_path` equal the expected
volume-internal joins; present credential paths equal the expected key joins. Violation is a
typed `StorageConflict` with the record kept. `remove_dir_all` never follows symlinks, so
nothing outside the volume is reachable.

### Owned host network items (released first)

Exactly the items the persisted attachment records as SDK-owned for this VM (tap link,
forwarding/iptables rules, host route, proxy-neighbour entry, sysctl restorations), released
through `cleanup_for_delete`. Already-absent items skip as already removed; unowned host
configuration is never addressed. A present-but-unreleasable item is a typed error with the
record kept.

### Inventory row plus cascades (deleted last)

Deleting the `microvms` row cascades to `vm_networks`, `vm_network_resources`,
`vm_credentials`, and `vm_runtime` via the existing `ON DELETE CASCADE` foreign keys, and the
existing repository method recomputes legacy bridge reference counts. Artifact rows (kernels,
images, downloads, binaries) have no foreign key from `microvms` and are unreachable.

## Records written

### `MicroVmDeleteResult` (new public type)

Returned once, after the record deletion commits:

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | Stable identifier of the deleted VM |

Failures surface as typed errors with no result. A repeat delete of the same name returns
`NotFound`.

## State machine

```text
lookup by name ──► unknown ──► NotFound (no host change)

known ──► socket probe ──► Ok(true) ──► LifecycleConflict stop-first (no host change)
                         │
                         ├── Err ──► probe error propagated (no host change)
                         │
                         └── Ok(false) ──► release owned network ──► failure ──► typed error, record kept
                                                                     (retry resumes here)
                                                │
                                                └── success ──► remove volume dir ──► failure ──► typed error, record kept
                                                                                         (retry resumes here)
                                                                      │
                                                                      └── success ──► delete record ──► failure ──► typed error, record kept
                                                                                                (retry resumes here)
                                                                             │
                                                                             └── success ──► MicroVmDeleteResult { name }
```

Every failure keeps the record; every retry re-reads the record and converges on the end
state (no record, no owned files, no owned network items). Success is reported only when all
three steps hold.

## Liveness decision table

| Socket probe | Decision |
|---|---|
| `Ok(true)` (answers) | running: refuse with stop-first `LifecycleConflict` |
| `Ok(false)` (clean silence) | stopped: proceed with deletion |
| `Err` (unprobable) | typed probe error; never delete under a possibly live machine |

Stored state, process liveness, and socket file existence never decide alone.

## Ownership and multi-VM rules

- The per-name lock (`{home}/vms/{name}`) plus the volume lock (when the persisted volume
  differs) are held across the probe, network release, file removal, and record deletion, so
  concurrent same-name deletes run one deletion sequence and all callers observe one
  consistent outcome.
- Network release is limited to the deleted VM's recorded owned items; other VMs' attachments
  and unowned host configuration are never addressed.
- No process is ever signaled. No artifact, cache, tool, tmp, or other-VM path is ever
  removed.
