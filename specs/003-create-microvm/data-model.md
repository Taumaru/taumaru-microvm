# Phase 1 Data Model: MicroVM Creation

## Domain records

### `CreateMicroVmRequest`

The request is caller-owned input and is never stored verbatim as an opaque blob.

| Field | Type | Rules |
|---|---|---|
| `name` | `String` | 1–64 ASCII characters; first character alphanumeric; remaining characters alphanumeric, `-`, or `_`; no spaces, separators, dots, or control characters. |
| `distribution_id` | `String` | Non-empty validated registry ID. |
| `image_id` | `String` | Non-empty validated image ID belonging to `distribution_id`. |
| `disk_size_bytes` | `u64` | Positive; at least the image's registry-reported `size_bytes` and verified source size; never shrunk. |
| `vcpu_count` | `u32` | Positive and at least the distribution minimum. |
| `memory_bytes` | `u64` | Positive and at least the distribution minimum converted to bytes; checked conversion to the `firectl` MiB argument. |
| `expose_on_lan` | `bool` | `false` selects host-only networking; `true` selects bridge/DHCP networking. |
| `volume_path` | `Option<PathBuf>` | Absolute normalized VM-volume directory; defaults to `{sdk_home}/vms/{name}`. |

The request is validated before a VM row, directory, host interface, key, or process is created.
Values that cannot fit SQLite's signed integer representation are rejected as invalid resource
requests rather than truncated.

### `MicroVmRecord`

One durable row identifies one independently managed VM. The stable identity is `name`; no global
single-VM field or process-local registry is used.

| Field | Meaning |
|---|---|
| `id` | SQLite identifier used by foreign keys. |
| `name` | Stable path-friendly VM identifier, unique on the host. |
| `state` | `creating`, temporary `running`, or successful `configured`. |
| `distribution_id` | Requested distribution registry ID. |
| `image_id` | Exact caller-selected image registry ID. |
| `kernel_id` | Distribution-selected default kernel registry ID. |
| `firecracker_package_id` | Independently resolved verified Firecracker package ID. |
| `firectl_package_id` | Independently resolved verified `firectl` package ID. |
| `disk_size_bytes` | Requested final root-disk file size. |
| `memory_requested_bytes` | Caller-requested memory. |
| `memory_effective_mib` | Checked value passed to `firectl`; ceiling conversion is explicit. |
| `vcpu_count` | Requested vCPU count. |
| `volume_path` | Per-VM directory, unique across VM records. |
| `rootfs_path` | `{volume_path}/rootfs.ext4`, unique and inside `volume_path`. |
| `socket_path` | `{volume_path}/firecracker.sock`; expected path is persisted even while no socket exists. |
| `created_at`, `updated_at` | Host-local timestamps. |

The durable `configured` state means the VM is stopped. `running` is persisted only around a
temporary setup or DHCP observation. A failed attempt is not published as a usable VM: after
rollback the provisional row is deleted. The internal state machine can report a retryable
failure while cleanup is executing without retaining a durable `failed` VM row.

### `VmNetwork`

Each VM has exactly one desired network attachment.

| Field | Host-only value | LAN value |
|---|---|---|
| `mode` | `host_only` | `lan` |
| `guest_ip` | Allocated static guest endpoint | Last verified DHCP address; nullable before the temporary lease check completes |
| `prefix_length` | `30` | Observed LAN prefix, if supplied by DHCP |
| `gateway_ip` | Host endpoint of the `/30` | DHCP-provided gateway, if observed |
| `host_ip` | Host endpoint of the `/30` on TAP | Host/bridge address reference, when applicable |
| `tap_name` | VM-specific deterministic TAP name | VM-specific deterministic TAP name |
| `guest_mac` | VM-specific locally administered MAC | VM-specific locally administered MAC |
| `bridge_id` | `NULL` | Shared SDK-managed bridge reference |
| `uplink_name` | `NULL` | Detected default-route uplink |
| `dhcp_lease_reference` | `NULL` | External lease identifier or observation reference; never a DHCP server owned by the SDK |
| `desired_boot_parameters` | Static `ip=` parameters | DHCP `ip=dhcp` parameter |
| `updated_at` | Last persisted reconciliation | Last persisted lease/reconciliation |

The table stores desired and observed values needed to reconstruct the attachment after SDK
recreation. It does not claim that a guest is running after creation.

### `NetworkBridge`

A shared host resource represents one SDK-managed bridge/uplink pair. It contains the bridge
name, uplink name, ownership marker, and current managed-reference count. A VM references the
bridge instead of owning its lifetime outright. A rollback deletes the bridge only when the
attempt created it and no other VM references it. A pre-existing foreign bridge is never replaced.

### `VmNetworkResource`

This child record makes reconciliation and safe cleanup explicit. Each row contains:

- VM foreign key;
- resource kind (`tap`, `tap_address`, `bridge_attachment`, `forwarding`, `nat`, or
  `firecracker_interface`);
- stable resource identity and desired fingerprint;
- SDK ownership marker;
- optional nftables handle or equivalent adapter reference;
- last observed state.

The identity/fingerprint pair is compared to live host state. A matching resource is reported as
skipped; a missing or SDK-owned stale resource is repaired; a foreign resource is a typed conflict.

### `VmCredential`

One credential record exists per VM:

| Field | Value |
|---|---|
| `private_key_path` | `{volume_path}/ssh/id_ed25519`; path only, never key content |
| `public_key_path` | `{volume_path}/ssh/id_ed25519.pub` |
| `guest_authorized_keys_path` | `/root/.ssh/authorized_keys` |
| `key_type` | `ed25519` |
| `ssh_user` | `root` |
| `ssh_port` | `22` |
| `public_key_fingerprint` | Non-secret fingerprint for revalidation and idempotency |
| `file_mode` | Restrictive mode verified by the credential adapter |

The public key is injected into the VM-local rootfs copy. The private-key path is returned by
creation and persisted for future SDK operations. SQLite has no private-key-content column.

### `VmRuntime`

Runtime metadata contains the selected executable paths, the expected socket path, the temporary
process identifier when a setup process is active, and the last observed process state. A
successful row has no active process and no live socket. The firectl/firecracker paths are
recorded separately so future operations can revalidate each independent artifact.

The selected runtime package IDs may be the same when one package provides both required
components, or different when the registry publishes split packages. Selection uses valid
semantic versions and host architecture independently for `firecracker` and `firectl`, choosing
the highest valid version for each component with deterministic tie-breaking; the versions do not
need to match.

## Registry additions used by creation

The registry image's `size_bytes` is the original `.ext4` file size and is the minimum disk size
accepted by creation. The same verified value is persisted in `distribution_images.size_bytes` and
used as the physical floor before copying. No duplicate minimum-size field is required, and no
registry service or download behavior is changed by this feature.

The image validation path also requires `format == "ext4"` and
`filesystem.type == "ext4"`. The distribution's existing boot configuration and default kernel
relationship remain the source of kernel arguments and root-device metadata.

## Lifecycle and ownership transitions

```text
absent
  │ preflight succeeds; per-name lock acquired
  ▼
creating ── temporary runtime required ──► running
   │                                         │
   │ setup/cleanup succeeds                   │ stop, remove socket, verify stopped
   │                                         ▼
   └──────────────────────────────────────► configured
   │
   └─ any failure ── rollback attempt-owned resources ──► absent
```

Rules:

1. Preflight resolves local artifact records and rechecks physical integrity before the
   `creating` row or host mutation.
2. The row is inserted with a unique name and immutable request fields. A concurrent same-name
   request waits on the SDK lock; the second request receives the existing result only when the
   immutable configuration matches.
3. `running` is never a successful return state. It exists only while the runtime adapter owns a
   temporary process for LAN DHCP discovery or an explicitly enabled health check.
4. The row changes to `configured` only after the process is stopped, the socket is absent, the
   rootfs and key references revalidate, network state is persisted, and the final transaction
   commits.
5. A failed attempt deletes its provisional row after cleanup. Cleanup is idempotent and only
   acts on resources tagged with the attempt's ownership marker.

## SQLite migration shape

Add `0002_microvm_creation.sql` and register it after the existing artifact-inventory migration.
The migration creates:

- `microvms` with unique name, volume/rootfs/socket paths, immutable creation configuration, and
  lifecycle state checks;
- `vm_networks` with one row per VM and mode-specific address/bridge fields;
- `network_bridges` with unique bridge/uplink identity and shared-resource ownership;
- `vm_network_resources` with ownership/fingerprint data for reconciliation;
- `vm_credentials` with path/fingerprint metadata and no secret material;
- `vm_runtime` with independent binary references and temporary process metadata;
- indexes for VM state, network address, resource identity, and bridge lookup.

Foreign keys use restrictive behavior for shared or source artifact records and cascading cleanup
for VM-owned child metadata. The migration verifier must require every new table and column, and
the migration checksum must be covered by the existing migration-drift check.

## Validation and invariants

- All persisted paths are normalized absolute paths and are checked to remain within the explicit
  SDK home unless the caller supplied an allowed external VM-volume path.
- `rootfs_path` and `socket_path` are deterministic children of `volume_path`; the SDK refuses a
  record that violates this invariant.
- VM names, volume paths, TAP identities, bridge identities, and network addresses have unique
  ownership checks independent of process-local memory.
- Disk size is compared in bytes before copy; expansion is monotonic and the source image is
  never modified.
- A configured VM has exactly one public SSH key fingerprint and a private-key path that exists
  with restrictive permissions; key contents never enter SQLite or SDK errors.
- Runtime process identifiers are advisory and are revalidated against the socket/path before
  use. A stale PID is never killed solely because it matches the database.
