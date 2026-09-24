# SDK Restore Contract

## Operation

The SDK exposes a restore operation accepting a snapshot archive path and one non-empty password, with optional caller-owned cancellation and progress observation. It returns a typed result with archived VM identity, destination-local volume path, and stopped state. The SDK does not prompt, print, or log.

Restore accepts manifest version 2 with an explicit IPv4 address policy. It rejects version 1, unknown versions, missing or inconsistent policy fields, IPv6 network values, unsafe metadata, and unsupported archive members.

## Preconditions

- The archive path is a readable regular file and the password is non-empty.
- The fixed member set, manifest, archive framing, age authentication, payload sizes/digests, SSH key correspondence, and kernel metadata verify.
- The archived VM name and managed destination paths are available.
- Destination storage and compatible runtime/architecture prerequisites are available.
- For preserve policy, exact guest/LAN IPv4 values and MAC are valid and available on the destination.
- For regenerate policy, the destination can allocate network values based on the archived LAN exposure setting.

## Success

- Root disk, kernel, and SSH keys are verified and installed at destination-local paths.
- Kernel metadata is registered with the exact embedded kernel in destination inventory.
- Preserve policy retains archived guest address/prefix/gateway, optional LAN address, mode, exposure, and MAC, or restore fails atomically.
- Regenerate policy derives mode from LAN exposure, allocates local IPv4 and network identity, and writes that configuration into the recovered disk.
- Host networking resources and ownership are created locally for either policy.
- VM, network, credential, stopped runtime, and kernel inventory are published together. The VM is stopped and can use the normal start operation.
- The source archive is unchanged.

## Failure and recovery

Expected input, password/authentication, compatibility, integrity, conflict, runtime, network, database, storage, cancellation, and filesystem failures return typed SDK errors. Failure must not publish an incomplete VM or alter pre-existing VMs. The operation cleans only files and host resources it owns; cleanup failures are included in error context.

Before host changes, the SDK persists a durable journal. The next restore invocation reconciles operation-owned files and resources left by an abrupt shutdown before retrying the affected name. The SDK emits no output or logs.
