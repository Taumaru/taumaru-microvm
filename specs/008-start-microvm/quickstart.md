# Quickstart: Start a Configured MicroVM

This feature is consumed through the Rust SDK. It does not add a CLI command. Creation and
artifact acquisition are separate operations and must complete before start.

## Host prerequisites

The host must be Linux with:

- KVM enabled and `/dev/kvm` readable and writable by the SDK process;
- permission to reconcile the VM's TAP device, addresses, routes, forwarding, and NAT rules;
- the VM's persisted boot artifacts still verified in the inventory (kernel, Firecracker,
  `firectl` binaries from creation time);
- one `Configured` (stopped) VM in the explicit SDK home's inventory, with its volume directory,
  `rootfs.ext4`, and SSH key files intact.

The SDK home must be supplied explicitly. It is the directory containing `state/inventory.db`,
downloaded artifacts, and the VM volumes. The SDK does not read `HOME` or another environment
variable to choose it.

## Prepare a stopped VM

Use the existing SDK creation operation first. Start does not create, configure, or download
anything; it reads the volume directory and full configuration from the inventory record.

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;
```

## Start a VM by name

```rust
let running = sdk.start_microvm("build_vm").await?;
```

On success:

- `running.state` is `Running` and the machine keeps running after the call returns;
- `running.volume_path` is the persisted volume directory from the inventory record;
- `running.rootfs_path` is the VM-local `rootfs.ext4` used for launch;
- `running.socket_path` is `{volume}/firecracker.sock` and answers on the control channel;
- `running.process_id` is the background machine process for forced termination fallback;
- `running.network` is the reconciled attachment with the persisted identity unchanged;
- `running.ssh` carries the `root:22` metadata with the private-key path only.

The caller holds no process, window, or session open. Later control prefers the socket; the
process ID is the fallback for an unresponsive machine.

## Repeat safely and recover from external kills

```rust
// While live: returns the same identity, launches nothing.
let again = sdk.start_microvm("build_vm").await?;
assert_eq!(again.process_id, running.process_id);
```

If the machine process was terminated outside the SDK, the next start detects the stale state
(dead process or silent socket) and boots a fresh running machine instead of reporting the dead
one as running.

## Repair host network lost to a reboot

No separate repair step is needed. Start inspects the persisted attachment, keeps correct items,
repairs only missing or stale items, and keeps the VM's addresses and mode unchanged. If repair
is impossible, the call returns a typed network error with the VM still stopped and no process
launched. If the launch itself fails after repair, repaired items stay in place for the next
retry while the VM stays stopped with no orphan process.

## Expected failure handling

Start fails before host mutation when the name is invalid or unknown, or when the stored state
is still `Creating`. It fails with a typed prerequisite error when the volume, rootfs, keys, or
boot artifacts are missing or unusable. A `Creating` row is never booted. Every expected failure
is a typed `SdkError` with no panic, no process exit, and no stdout/stderr output.

## Verification commands

Run the repository gates after implementation:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The automated suite covers start lifecycle, idempotent repeat, stale recovery in both mismatch
directions, PID-reuse safety, per-mode network repair with skipped/applied behavior, `Creating`
rejection, launch-failure cleanup, concurrent same-name starts, and operation silence through
injected fake network and runtime ports. Real KVM, process, socket, and network-administration
integration requires a capability-gated Linux environment and is not executed by the default test
suite.
