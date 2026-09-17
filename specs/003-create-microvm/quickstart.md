# Quickstart: Create a Configured, Stopped MicroVM

This feature is consumed through the Rust SDK. It does not add a CLI command. Artifact download
is a separate operation and must complete before creation.

## Host prerequisites

The host must be Linux with:

- KVM enabled and `/dev/kvm` readable and writable by the SDK process;
- permission to create TAP devices, configure the selected bridge or host-only routes, and manage
  SDK-owned nftables rules;
- the ext4 filesystem and resize helpers used by the SDK storage adapter;
- a registry inventory containing the caller-selected distribution image, its default kernel,
  one verified Firecracker executable, and one verified `firectl` executable;
- a registry-declared minimum disk size for the selected image.

The SDK home must be supplied explicitly. It is the directory containing `state/inventory.db`,
downloaded artifacts, and the default VM volumes. The SDK does not read `HOME` or another
environment variable to choose it.

## Prepare artifacts

Use the existing SDK artifact operations to acquire and verify the distribution, its required
kernel, and the two independent runtime binary packages. Creation does not download missing files.
The selected image ID must be provided explicitly when creating the VM.

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;

// Artifact acquisition is intentionally separate from creation.
sdk.download_distribution("ubuntu-24.04", |_| {}).await?;
sdk.download_kernel("linux-6.8-x86_64", |_| {}).await?;
sdk.download_binary("firecracker-1.17.0-x86_64", |_| {}).await?;
sdk.download_binary("firectl-0.2.0-x86_64", |_| {}).await?;
```

The concrete IDs above are illustrative; the caller must use IDs published by the configured
registry and the distribution's actual default kernel. The two runtime packages may be a single
combined package or separate packages with different versions; each required component is selected
independently using the current host-architecture and highest-valid-semantic-version policy.

## Create a host-only VM

```rust
let result = sdk
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
```

On success:

- `result.state` is `Configured` and the VM is stopped;
- `result.volume_path` is `/var/lib/taumaru/vms/build_vm` unless a custom directory was supplied;
- `result.rootfs_path` is the copied and possibly expanded `rootfs.ext4` inside that directory;
- the source image is unchanged;
- the result contains a unique host-only guest address, gateway, TAP identity, and SSH metadata;
- `result.ssh.private_key_path` points to the protected host key, while only the public key was
  injected into `/root/.ssh/authorized_keys` in the VM-local image;
- `result.socket_path` is the path reserved for a future runtime, but there is no active socket or
  Firecracker process after return.

The host-only guest address is static and uses the host endpoint of its exclusive `/30` as the
gateway. Host NAT supplies outbound connectivity. The operation does not expose the VM on the LAN.

## Create a LAN VM

Set `expose_on_lan: true` in the same request. The SDK detects the default-route uplink, attaches
the VM TAP to the shared SDK-managed bridge, and starts the VM temporarily to obtain its external
DHCP address. It stops the exact temporary process and removes the active socket before returning.

If uplink discovery, bridge setup, DHCP, conflict validation, or required permission fails, the
operation returns a typed error and does not fall back to host-only mode.

## Reconcile a VM's network after a host restart

```rust
let network = sdk.configure_network("build_vm").await?;
```

The operation reads the persisted desired mode and ownership data. It returns resource identities
in `applied` and `skipped`:

- correct TAP, address, bridge, attachment, forwarding, NAT, and lease resources are skipped;
- missing or SDK-owned stale resources are repaired and reported as applied;
- foreign resources produce a typed ownership conflict;
- the VM remains stopped and its expected socket remains inactive.

For LAN mode, the SDK may temporarily start the VM again only when it must obtain or validate a
DHCP lease. It never starts a VM as the successful final state.

## Expected failure handling

Creation fails before host mutation when an ID, image, kernel, runtime binary, architecture,
registry minimum, resource value, or KVM prerequisite is invalid. After mutation begins, every
failure runs rollback for attempt-owned files, credentials, process/socket, network resources, and
the provisional SQLite row. Existing source artifacts, caller-owned directories/files, other VM
records, and foreign network resources are preserved.

The private key is never present in an error, log, SDK output stream, or SQLite value. Public SDK
operations are silent and return typed `SdkError` values for all expected failure paths.

## Verification commands

Run the repository gates after implementation:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The SDK tests use fake artifact, storage, network, guest-filesystem, and runtime ports for
deterministic success, idempotency, conflict, skip, repair, and rollback cases. Privileged Linux
integration tests are opt-in and run in an isolated network namespace when the required host
capabilities are available.
