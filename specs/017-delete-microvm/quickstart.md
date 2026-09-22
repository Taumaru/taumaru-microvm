# Quickstart: Delete a MicroVM

This feature is consumed through the Rust SDK. It does not add a CLI command. Creation, start,
stop, and artifact acquisition are separate operations; delete reads everything it needs from the
inventory record left behind by creation.

## Host prerequisites

The host must be Linux with:

- a VM record in the explicit SDK home's inventory, with its volume directory
  (`{volume}/rootfs.ext4`, `{volume}/firecracker.sock`, `{volume}/ssh/`) intact for the full
  case — partially removed volumes still delete, converging on the end state;
- permission to remove the VM volume directory and to release the VM's owned host network items.

The SDK home must be supplied explicitly. It is the directory containing `state/inventory.db`,
downloaded artifacts, and the VM volumes. The SDK does not read `HOME` or another environment
variable to choose it.

## Prepare a stopped VM

Use the existing SDK operations first. Delete does not create, configure, download, repair, or
stop anything; it refuses a running machine instead of stopping it.

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;
```

## Delete a VM by name

```rust
let deleted = sdk.delete_microvm("build_vm").await?;
```

On success:

- `deleted.name` is the deleted machine's name;
- the name no longer resolves: lookups report not-found and a repeat delete returns `NotFound`;
- the whole volume directory (`{volume}/` including disk copy, SSH keys, socket files, logs) is
  gone — no empty directory is left behind;
- the VM's owned host network items are released while other VMs and unowned host
  configuration are untouched;
- the referenced kernel and distribution image remain usable by other machines with no
  re-download required.

## Running machines are refused

```rust
match sdk.delete_microvm("build_vm").await {
    Err(SdkError::LifecycleConflict { .. }) => {
        // Stop first, then delete again with the same name.
    }
    result => result?,
}
```

A refused delete changes nothing: owned files, the record, and the network attachment are
untouched.

## Failed deletes are retried, not repaired

Delete keeps the record on every failure (unreleasable network item, unremovable volume,
record-write failure), so the recovery path is always the same call again after fixing the
cause. Already-absent owned files and network items count as already removed; only a fully
unknown machine name errors as not-found.

## Expected failure handling

Delete fails before host mutation when the name is invalid or unknown, when the machine is
running, and when the control socket cannot be probed. It fails with the record kept when
persisted paths escape the volume or home, when owned file or network removal fails, and when
the record deletion fails. Every expected failure is a typed `SdkError` with no panic, no
process exit, and no stdout/stderr output.

## Verification commands

Run the repository gates after implementation:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The automated suite covers stopped delete with full owned-trace removal, running refusal with
zero host change plus stop-then-delete success, repeat-delete not-found, absent-file and
absent-network-item convergence, network and volume failure with record kept plus retry
success, containment refusal, incomplete-record delete, unknown and malformed names,
multi-VM isolation, concurrent same-name deletes, shared-artifact preservation, and operation
silence through injected fake ports. Real socket, network, and filesystem integration requires
a capability-gated Linux environment and is not executed by the default test suite.
