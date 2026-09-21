# Quickstart: Stop a Running MicroVM

This feature is consumed through the Rust SDK. It does not add a CLI command. Creation, start,
and artifact acquisition are separate operations and must complete before stop.

## Host prerequisites

The host must be Linux with:

- a running VM in the explicit SDK home's inventory, with its volume directory and
  `{volume}/firecracker.sock` control socket intact;
- permission to connect to the volume-local control socket and to signal the recorded machine
  process on the forced path.

The SDK home must be supplied explicitly. It is the directory containing `state/inventory.db`,
downloaded artifacts, and the VM volumes. The SDK does not read `HOME` or another environment
variable to choose it.

## Prepare a running VM

Use the existing SDK start operation first. Stop does not create, configure, download, or repair
anything; it reads the volume directory and runtime references from the inventory record.

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;
```

## Stop a VM by name

```rust
let stopped = sdk.stop_microvm("build_vm").await?;
```

On success:

- `stopped.state` is `Stopped` and the machine stays stopped after the call returns;
- `stopped.socket_path` is `{volume}/firecracker.sock` and is silent on the control channel;
- `stopped.forced` is `false` when the guest exited on its own and `true` only when SIGKILL was
  delivered to the recorded process.

Stopping an already-stopped machine is success with `forced: false` and no host change, so
recovery flows can stop unconditionally.

## Graceful versus forced

The operation first asks the machine to shut down through its control socket and waits 60
seconds for the exit. If the guest ignores the request, the operation sends SIGKILL to the
recorded process identity (re-verified to still reference this VM), re-verifies the stop, and
reports `forced: true`. Guests need the standard shutdown drivers for the graceful path; without
them the forced path is the backstop.

A shutdown request that cannot be delivered while the machine still answers is a typed error
with no forced attempt. A machine that stays running with no usable process identity is a typed
error that never reports success.

## Expected failure handling

Stop fails before host mutation when the name is invalid or unknown, or when the creation
record is incomplete. It fails with a typed error when the graceful request cannot be
delivered, when forced termination cannot proceed or fails, and when the machine is still
running afterwards. Every expected failure is a typed `SdkError` with no panic, no process
exit, and no stdout/stderr output.

## Verification commands

Run the repository gates after implementation:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The automated suite covers graceful stop, already-stopped idempotency across stopped,
never-started, and externally-killed rows, forced escalation with the flag, undeliverable-request
and unforceable-machine errors, PID-reuse safety, the escalation race, concurrent same-name
stops, and operation silence through injected fake runtime ports. Real socket, process, and
signal integration requires a capability-gated Linux environment and is not executed by the
default test suite.
