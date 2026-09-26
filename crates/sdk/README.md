<div align="center">

# taumaru-microvm

**The whole Firecracker microVM lifecycle, as one async Rust API.**

[![crates.io](https://img.shields.io/crates/v/taumaru-microvm.svg)](https://crates.io/crates/taumaru-microvm)
[![docs.rs](https://img.shields.io/docsrs/taumaru-microvm)](https://docs.rs/taumaru-microvm)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://opensource.org/licenses/MIT)

</div>

```rust
let sdk = MicroVmSdk::new("/var/lib/my-app/microvm")?;
sdk.create_microvm(request, None::<fn(CreationProgress)>).await?;
let vm = sdk.start_microvm("dev").await?;
println!("ssh {}@{}", vm.ssh.user, vm.ssh.address); // a real, KVM-isolated Linux machine
```

Firecracker gives you a VMM and a Unix-socket REST API. Everything else is up to
you: kernels, root filesystems, TAP devices, IP allocation, SSH keys, process
tracking, disk snapshots, and cleanup when something fails halfway through.

**`taumaru-microvm` is that missing layer**, as a typed and embeddable library.
It is the same engine behind the [`microvm` CLI](../../README.md), so every
operation has already been exercised end to end.

## Why use it

- **Artifacts included.** Download the Firecracker runtime, kernels, and
  ready-to-boot images from the curated Taumaru Artifacts Registry. Each file is
  checked against its SHA-256 digest, recorded in the inventory, and cached for
  later VMs.
- **Create, start, stop, delete.** Each VM gets its own ext4 disk, a fresh
  ed25519 key pair injected into the guest, and host-only or routed LAN
  networking. You get back the SSH user, address, port, and key path.
- **Live, portable snapshots.** Archive a running VM's disk into one `.tmvmsnap`
  file with no downtime (Device Mapper point-in-time view), then restore it on
  any host with preserved or reassigned IPv4 addresses.
- **Boot autostart policies** with retry attempts and pause/resume, plus one
  call to start them all.
- **Live state, not stored state.** `Running` means the VM's control socket
  answered right now. There are no stale "running" rows after a crash or reboot.
- **Safe for a library.** No panics, nothing printed to stdout or stderr, and no
  global state. Every failure is a typed `SdkError`. Long operations take
  progress callbacks and cooperative cancellation handles, and clean up after
  themselves.
- **Concurrency-aware.** Per-VM locks serialize conflicting operations on the
  same machine, even across processes.

## Install

```sh
cargo add taumaru-microvm
cargo add tokio --features macros,rt-multi-thread
```

**Host requirements:** Linux x86_64 with KVM (`/dev/kvm`), and root privileges
for operations that touch Firecracker, networking, or Device Mapper (`start`,
`stop`, `create`, `snapshot`, and so on).

## Quickstart: your first microVM

```rust
use taumaru_microvm::{CreateMicroVmRequest, CreationProgress, MicroVmSdk, SdkError};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), SdkError> {
    // Opens (or creates) the home directory and its SQLite inventory.
    let sdk = MicroVmSdk::new("/var/lib/my-app/microvm")?;

    // 1. Fetch what the VM needs from the Taumaru registry. Verified files are cached,
    //    so running this again is instant.
    let distribution = sdk
        .list_distributions()
        .await?
        .into_iter()
        .find(|d| d.id == "ubuntu-24.04")
        .expect("published distribution");
    sdk.download_binary("firecracker-1.17.0-x86_64", |_| {}).await?;
    sdk.download_binary("firectl-0.2.0-x86_64", |_| {}).await?;
    sdk.download_kernel(&distribution.default_kernel, |_| {}).await?;
    sdk.download_distribution_image("ubuntu-24.04", "ubuntu-24.04", |_| {})
        .await?;

    // 2. Create the VM: disk, SSH keys, and network are set up for you.
    let created = sdk
        .create_microvm(
            CreateMicroVmRequest {
                name: "dev".into(),
                distribution_id: "ubuntu-24.04".into(),
                image_id: "ubuntu-24.04".into(),
                disk_size_bytes: 10 * 1024 * 1024 * 1024,
                vcpu_count: 2,
                memory_bytes: 1024 * 1024 * 1024,
                expose_on_lan: false,
                lan_address: None,
                volume_path: None,
            },
            Some(|p: CreationProgress| println!("{:>3}% {}", p.overall_percent, p.stage)),
        )
        .await?;

    // 3. Boot it.
    let running = sdk.start_microvm(&created.name).await?;
    let ssh = &running.ssh;
    println!(
        "ssh -i {} -p {} {}@{}",
        ssh.private_key_path.display(),
        ssh.port,
        ssh.user,
        ssh.address
    );

    // 4. Stop it when you are done. The disk is kept for the next start.
    sdk.stop_microvm("dev").await?;
    Ok(())
}
```

Run it with `sudo`, paste the printed `ssh` command, and you are `root` inside
Ubuntu 24.04.

> [!TIP] The registry IDs above are current at the time of writing. Use
> `list_binaries()`, `list_kernels()`, and `list_distributions()` to discover
> what is published, or `is_distribution_image_ready()` to skip downloads you
> already have. Every `download_*` method has a `*_with_cancellation` variant,
> and its callback receives `DownloadProgress` events for your progress bar.

## Recipes

### List machines with their live state

```rust
for vm in sdk.list_microvms().await? {
    let state = match vm.state {
        MicroVmState::Running => "running",
        MicroVmState::Stopped => "stopped",
    };
    println!("{:<16} {state:<8} {} vCPU  {} MiB", vm.name, vm.vcpu_count, vm.memory_bytes >> 20);
}
```

`list_running_microvms()` returns only running VMs together with their SSH
connection details.

### Snapshot a running VM and restore it elsewhere

```rust
// Works while "dev" is running: the disk is captured through a point-in-time view.
let snapshot = sdk
    .create_snapshot("dev", Path::new("./dev.tmvmsnap"), SnapshotAddressPolicy::RegenerateIpv4)
    .await?;
println!("wrote {} bytes to {}", snapshot.archive_size_bytes, snapshot.output_path.display());

// On any host: recreate the VM (stopped) from the archive, then start it.
let restored = sdk
    .restore_snapshot(RestoreRequest { archive_path: "./dev.tmvmsnap".into() })
    .await?;
sdk.start_microvm(&restored.vm_name).await?;
```

- Snapshots are crash-consistent and disk-only; guest memory is not captured.
- Use `create_snapshot_with_cancellation_and_progress` and
  `restore_snapshot_with_cancellation_and_progress` to drive a progress bar and
  to allow cancelling.
- Archives are **not encrypted** and contain the VM's disk and SSH private key.
  Protect them with file permissions.

### Start machines at host boot

```rust
sdk.create_autostart_policy("dev", AutostartSettings::default()).await?;
sdk.update_autostart_policy(
    "dev",
    AutostartPolicyUpdate { enabled: Some(false), max_start_attempts: None },
)
.await?;
// At host boot, from your own service:
let report = sdk.start_autostart_microvms().await?;
if report.has_failures() {
    eprintln!("some machines did not start");
}
```

### Reclaim disk space

```rust
let summary = sdk.prune_unused_artifacts().await?;
println!("freed {} bytes", summary.freed_bytes_kernels + summary.freed_bytes_images);
```

`list_prune_candidates()` previews what would be removed. Kernels and images
still used by a VM are never touched.

### Handle errors precisely

```rust
match sdk.start_microvm("dev").await {
    Ok(vm) => println!("running as pid {}", vm.process_id),
    Err(SdkError::NotFound { kind, id }) => println!("no {kind} named {id}"),
    Err(SdkError::LifecycleConflict { state, .. }) => println!("dev is {state}"),
    Err(other) => return Err(other),
}
```

## API at a glance

| Area      | Methods                                                                                                                                                                                   |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Registry  | `list_kernels`, `list_binaries`, `list_distributions`                                                                                                                                     |
| Artifacts | `download_binary`, `download_kernel`, `download_distribution_image`, `is_distribution_image_ready`, `list_present_distribution_images`, `list_prune_candidates`, `prune_unused_artifacts` |
| Lifecycle | `create_microvm`, `start_microvm`, `stop_microvm`, `delete_microvm`, `configure_network`                                                                                                  |
| Inventory | `list_microvms`, `list_running_microvms`                                                                                                                                                  |
| Snapshots | `create_snapshot*`, `restore_snapshot*`                                                                                                                                                   |
| Autostart | `create_autostart_policy`, `update_autostart_policy`, `delete_autostart_policy`, `autostart_policy`, `list_autostart_policies`, `start_autostart_microvms`                                |

The full reference is on [docs.rs](https://docs.rs/taumaru-microvm).

## How it stores things

`MicroVmSdk::new(home)` manages one self-contained directory:

```text
<home>/
├── artifacts/{kernels,rootfs}   # verified, shared downloads
├── tools/                       # Firecracker runtime binaries
├── vms/<name>/                  # rootfs.ext4, firecracker.sock, firecracker.log, ssh/
├── tmp/                         # snapshot and restore scratch space
└── state/inventory.db           # SQLite, migrated automatically
```

Point several apps, or several tenants of one app, at different homes to keep
them fully isolated.

## License

Released under the [MIT license](https://opensource.org/licenses/MIT).
