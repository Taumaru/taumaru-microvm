# SDK Contract: MicroVM Creation and Network Configuration

This contract describes the public SDK surface for the feature. It intentionally contains no CLI
command, Firecracker command line, shell command, SQLite statement, private-key material, or
registry HTTP implementation detail.

## Public types

The types below are re-exported from `crates/sdk/src/lib.rs` and carry Rustdoc describing the
same invariants.

```rust
pub struct CreateMicroVmRequest {
    pub name: String,
    pub distribution_id: String,
    pub image_id: String,
    pub disk_size_bytes: u64,
    pub vcpu_count: u32,
    pub memory_bytes: u64,
    pub expose_on_lan: bool,
    pub volume_path: Option<std::path::PathBuf>,
}

pub enum MicroVmState {
    Creating,
    Running,
    Configured,
}

pub enum NetworkMode {
    HostOnly,
    Lan,
}

pub struct SshConnectionInfo {
    pub user: String,
    pub port: u16,
    pub address: std::net::IpAddr,
    pub private_key_path: std::path::PathBuf,
    pub public_key_path: std::path::PathBuf,
}

pub struct MicroVmCreationResult {
    pub name: String,
    pub state: MicroVmState,
    pub distribution_id: String,
    pub image_id: String,
    pub volume_path: std::path::PathBuf,
    pub rootfs_path: std::path::PathBuf,
    pub socket_path: std::path::PathBuf,
    pub vcpu_count: u32,
    pub memory_bytes: u64,
    pub disk_size_bytes: u64,
    pub network: NetworkConfiguration,
    pub ssh: SshConnectionInfo,
}

pub struct NetworkConfiguration {
    pub mode: NetworkMode,
    pub guest_address: std::net::IpAddr,
    pub prefix_length: u8,
    pub gateway: Option<std::net::IpAddr>,
    pub tap_name: String,
    pub bridge_name: Option<String>,
    pub uplink_name: Option<String>,
}

pub enum NetworkResource {
    Tap,
    TapAddress,
    Bridge,
    BridgeUplinkAttachment,
    BridgeTapAttachment,
    Forwarding,
    Nat,
    FirecrackerInterface,
    DhcpLease,
}

pub struct NetworkConfigurationResult {
    pub name: String,
    pub state: MicroVmState,
    pub configuration: NetworkConfiguration,
    pub applied: Vec<NetworkResource>,
    pub skipped: Vec<NetworkResource>,
}
```

`MicroVmState::Configured` is the only successful state returned by this feature. `Creating` and
`Running` are observable in internal persistence and runtime coordination but are not returned as
successful results. `socket_path` is the expected per-VM path; a successful result guarantees that
no active socket exists there.

`SshConnectionInfo` contains the path needed to use the private key, not the key contents. The
fixed guest identity is `root` on port `22`. The public key path is returned to make host-side
diagnostics and future lifecycle operations possible, but the SDK never returns the guest key
file contents through this API.

## Public operations

### Create a MicroVM

```rust
impl MicroVmSdk {
    /// Creates and initially configures one stopped MicroVM from verified local artifacts.
    pub async fn create_microvm(
        &self,
        request: CreateMicroVmRequest,
    ) -> Result<MicroVmCreationResult, SdkError>;
}
```

Required behavior:

1. Validate the name, IDs, resource sizes, volume path, and explicit SDK home without consulting
   environment variables.
2. Resolve the exact distribution and caller-selected image from the local artifact inventory,
   verify the image's ownership by the distribution, ext4 metadata, registry-reported `size_bytes`
   as the minimum disk size, and physical digest/size.
3. Resolve the distribution's default kernel and one independently tracked Firecracker package
   plus one independently tracked `firectl` package. Both must be verified, executable when
   required, architecture-compatible, and selected using the component-aware semantic-version
   policy. One package may provide both components; split packages may provide them with different
   versions. Runtime startup remains the final compatibility check.
4. Reject all missing, stale, corrupt, ambiguous, or incompatible prerequisites before creating
   host resources. Creation never downloads a missing artifact.
5. Claim the VM name and volume path under a per-VM lock. A missing volume path defaults to
   `{sdk_home}/vms/{name}`. The volume is a directory; the stable children are `rootfs.ext4`,
   `ssh/id_ed25519`, `ssh/id_ed25519.pub`, and `{volume}/firecracker.sock`.
6. Copy the verified source image into the VM-local `rootfs.ext4`. Reject too-small requests
   before the copy; if the requested size is larger than the source, grow the copy and its ext4
   filesystem to the requested size. The source image is never modified.
7. Generate an Ed25519 key pair, store it with restrictive permissions, and inject only the public
   key into `/root/.ssh/authorized_keys` in the VM-local copy. Existing authorized keys are
   preserved and the generated key is not duplicated.
8. Configure the selected network mode through the SDK network port. `false` selects one exclusive
   host-only `/30`, TAP, static guest boot parameters, and host NAT. `true` selects a shared
   SDK-managed bridge on the detected uplink and guest DHCP; it never falls back to host-only.
9. Prepare the runtime with the copied rootfs path, selected kernel, requested resources, network
   attachment, distribution boot arguments, and the VM-local socket path. `firectl` and
   Firecracker process mechanics remain inside the runtime adapter.
10. For LAN mode, temporarily start the VM to observe a DHCP lease associated with its MAC. Stop
    the exact process before success. Host-only mode does not require a temporary start; a bounded
    internal SSH probe may be used when configured by the runtime adapter.
11. Verify the rootfs, key paths, persisted resource values, network ownership, stopped process
    state, and absent active socket. Commit the VM as `Configured` and return its connection
    metadata.

Repeated calls are idempotent only when the existing VM's immutable creation configuration matches
the request. The SDK revalidates the existing files and network state before returning the existing
result. A different image, distribution, resource value, network mode, or volume path returns a
typed conflict and does not mutate the existing VM.

### Reconcile network configuration

```rust
impl MicroVmSdk {
    /// Reconciles the persisted network attachment for one configured VM.
    pub async fn configure_network(
        &self,
        vm_name: &str,
    ) -> Result<NetworkConfigurationResult, SdkError>;
}
```

Required behavior:

- Resolve the VM by name from the explicit SDK home's SQLite inventory and reject unknown,
  creating, or invalid records with typed errors.
- Read the persisted desired mode, MAC, guest address/lease, TAP, bridge/uplink, and ownership
  fingerprints. The caller does not provide a new mode in this focused repair operation.
- Inspect live host state through the network adapter. Report each resource that already matches as
  `skipped`; create or repair only missing or SDK-owned stale resources as `applied`.
- Never reuse another VM's TAP, `/30`, MAC, address, bridge ownership, or firewall rule. A foreign
  resource is a typed ownership conflict.
- For LAN mode, reuse the persisted bridge/uplink and DHCP identity. If the lease/address is stale
  or missing, the adapter may temporarily start the VM to obtain a new lease, then must stop it
  and remove its socket before returning.
- Persist the observed result atomically with the resource fingerprints. The operation is safe to
  repeat and must not allocate a second subnet, duplicate a rule, or replace a correct resource.
- Return only after the VM is stopped and no active control socket remains.

## Error contract

The public result is `Result<_, SdkError>`. The implementation adds typed variants for the
following categories, preserving the existing artifact/download errors:

| Category | Required distinguishing data |
|---|---|
| Invalid request/name/resource | Field and validation reason; no host mutation. |
| Missing or stale artifact | Artifact kind, registry ID, local path, and acquire/repair reason. |
| Image disk-size prerequisite | Image ID, requested size, registry-reported `size_bytes`, and current verified image size. |
| Runtime incompatibility | Firecracker/firectl component, package IDs/versions, architecture, and compatibility reason. |
| Storage conflict | VM name, volume path, owner, and preservation reason; caller data is untouched. |
| VM lifecycle conflict | VM name, current state, and requested operation. |
| Network or permission failure | Mode, operation/resource, interface/address identity, and rollback result. |
| Guest filesystem or credential failure | Operation and non-secret path; never key content. |
| Temporary runtime/startup failure | Runtime component, exit/health condition, and stopped-state result. |
| Cleanup failure | Primary failure plus each cleanup failure and owned resource identity. |
| SQLite/migration failure | Existing typed database or migration error with no success result. |

All error display text is English and safe to expose to callers. No error variant includes a
private-key value, serialized secret, command output containing secret material, or a promise that
the VM is running. Missing prerequisites are actionable: callers are told to acquire or repair the
artifact before retrying.

## Side-effect and rollback contract

- Public operations are silent: no stdout/stderr, logging subscriber, tracing event, process exit,
  or global mutable state.
- Preflight failures perform no VM host mutation. Post-preflight failures remove only resources
  created by the attempt: process, socket, TAP/bridge/firewall changes, generated keys, copied
  rootfs, and provisional inventory rows.
- Existing source images, caller-owned volume contents, foreign host-network resources, and other
  VM records are preserved.
- Cleanup is attempted in reverse dependency order and is safe to retry. A typed cleanup failure
  is returned if the SDK cannot prove that the attempt-owned resource was removed.
- A successful create or network reconciliation leaves the VM stopped, its expected socket path
  persisted but inactive, and its host network configuration available for a future start.

## Example usage

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;

let created = sdk
    .create_microvm(CreateMicroVmRequest {
        name: "build_vm".to_owned(),
        distribution_id: "ubuntu-24.04".to_owned(),
        image_id: "ubuntu-24.04-minimal".to_owned(),
        disk_size_bytes: 16 * 1024 * 1024 * 1024,
        vcpu_count: 2,
        memory_bytes: 2 * 1024 * 1024 * 1024,
        expose_on_lan: false,
        volume_path: None,
    })
    .await?;

assert_eq!(created.state, MicroVmState::Configured);
assert_eq!(created.ssh.user, "root");
assert_eq!(created.ssh.port, 22);

let reconciled = sdk.configure_network("build_vm").await?;
assert_eq!(reconciled.state, MicroVmState::Configured);
```

The example intentionally uses only public SDK operations. It does not list, inspect, start,
stop, reboot, delete, download, or assemble a `firectl` command.
