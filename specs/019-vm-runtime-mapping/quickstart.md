# Quickstart: Validate Per-VM Runtime Disk Mapping

## Prerequisites

- Linux host with Firecracker/KVM support, loop devices, and a Device Mapper snapshot-origin target.
- losetup and dmsetup installed and available to the SDK process.
- The host execution context can create loop and Device Mapper mappings and can read/write the resulting mapper node.
- The same firectl and Firecracker artifacts configured for the project.
- Two stopped test MicroVMs with separate persistent rootfs.ext4 files.
- A privileged integration environment for the host-device scenario. Unit tests must not modify the host's real loop or Device Mapper state.

The SDK does not elevate itself. The existing CLI start/stop commands use the current privilege path; direct SDK callers must supply host capabilities and mapper-node access themselves.

## Automated validation

From the repository root, run the relevant Rust gates after implementation:

~~~bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
~~~

The focused SDK tests should cover:

- The start result still returns the persistent rootfs.ext4 path while firectl arguments contain the mapper path.
- A valid map is reused; two VMs use distinct mapper names and loop dependencies.
- A partial owned mapping is cleaned and rebuilt; foreign or in-use mappings are unchanged.
- Start failure cleans resources only after process exit is verified.
- Stop removes Device Mapper before loop, retains resources on busy/error, and never removes rootfs.ext4.
- Repeated stop cleans verified leftovers and is idempotent.
- Same-home concurrent start/stop calls from two SDK instances do not duplicate or race process and mapping state.
- Retrying after interruption with a persisted starting PID adopts a ready process or preserves a live unready process without clearing its identity or starting a duplicate.

## Privileged host integration scenario

1. Create two ordinary test VMs using the existing CLI or SDK workflow.
2. Start the first VM through the existing lifecycle command. Confirm the guest becomes ready.
3. Inspect the active snapshot-origin names and UUIDs with dmsetup ls --target snapshot-origin and dmsetup info -c --noheadings -o name,uuid,open.
4. Inspect the table and dependency for the first VM. Confirm its table names snapshot-origin and resolves to a loop device whose backing file is the first VM's rootfs.ext4.
5. Repeat for the second VM. Confirm its name/UUID and loop dependency are distinct.
6. Perform a guest write through each VM and confirm it persists in the corresponding .ext4 after stop.
7. Stop the first VM. Confirm its owned mapper is gone and its loop association is detached; confirm the second VM's mapper remains active.
8. Stop the second VM. Confirm its mapper and loop association are gone and both .ext4 files still exist at their original paths.
9. Start one VM again after removing only its transient mapping while stopped, or after a host reboot in an isolated test environment. Confirm start reconstructs a fresh mapping from the inventory and rootfs.

Expected outcome: firectl launches through the verified mapper path; public start results continue to report the persistent .ext4 path; stop releases only the target VM's temporary kernel resources after process exit.

## Failure validation

In tests with an injected fake runtime-disk controller, simulate missing tools/permissions, a foreign name/UUID collision, mapper creation failure after loop setup, busy mapper removal, firectl launch failure, readiness timeout, and cleanup failure. Confirm every failure returns a typed SDK error, preserves the persistent root disk, and leaves foreign or in-use resources untouched.

Do not run privileged integration cleanup against unrelated host mappings. Remove only test resources with the deterministic identity created by that test.
