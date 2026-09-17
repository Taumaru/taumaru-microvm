# Phase 0 Research: MicroVM Creation and Initial Configuration

## Research goals

This research resolves the implementation questions for an SDK-only operation that turns a
verified local distribution image into a configured, stopped MicroVM. The implementation must
keep Firecracker, `firectl`, Linux networking, ext4 manipulation, and SQLite behind SDK-owned
boundaries.

## Findings

### Firecracker and `firectl` boundaries

- Firecracker requires a Linux host with KVM access and a readable/writable `/dev/kvm`; the
  SDK therefore performs those checks during preflight and returns a typed prerequisite error
  before creating VM state.
- Firecracker uses an ext4 root filesystem and a separate kernel image. The selected registry
  image and the distribution's default kernel remain separate artifacts in the existing local
  inventory.
- `firectl` accepts independent paths for the Firecracker executable, kernel, root drive, TAP
  device/MAC, vCPUs, memory, kernel options, and socket path. This supports the requested
  independent Firecracker and `firectl` artifact records and a deterministic per-VM command
  assembled by the runtime adapter.
- The SDK must pass the copied VM-local `rootfs.ext4` to `firectl`; it must never pass the
  downloaded source image directly. `firectl`'s `--root-drive` supports a writable root drive
  and `--socket-path` allows the socket to stay under the VM volume directory.
- Firecracker's API configuration is a pre-boot concern. Network interfaces and boot settings
  are prepared before a temporary start, and successful creation removes the active socket and
  leaves no Firecracker process running.

Sources: [Firecracker getting started](https://github.com/firecracker-microvm/firecracker/blob/main/docs/getting-started.md),
[firectl README](https://github.com/firecracker-microvm/firectl/blob/main/README.md).

### Guest key injection and SSH

- The selected image is an explicit caller choice and is required to contain the SSH server,
  the `root` account, and the expected SSH layout. The SDK does not install or configure an SSH
  server during creation.
- The SDK generates one Ed25519 key pair per VM on the host, writes it with restrictive modes
  inside the VM volume directory, and writes only the public key into the per-VM ext4 copy at
  `/root/.ssh/authorized_keys`.
- Injection is performed while the root filesystem is offline. The guest filesystem adapter
  mounts the copy read-write in a private temporary mountpoint, preserves existing authorized
  keys, appends the generated key only when absent, enforces the expected directory/file modes,
  unmounts it, and verifies the filesystem before runtime use. The mountpoint is removed before
  the operation returns.
- The create result exposes the private-key path, never the private-key contents. SQLite stores
  only paths, the public-key path, the guest path, and a fingerprint/reference.

The fixed `root:22` SSH metadata is therefore part of the public result, but the feature does not
make an SSH connection mandatory for host-only creation. LAN creation must temporarily start the
guest to obtain and validate its DHCP address; an optional bounded SSH probe can run during that
same temporary start without changing the final stopped state.

### ext4 copy and resize

- The source image remains immutable and must already be verified by the artifact inventory.
- The SDK creates the VM directory first, copies the source bytes into an attempt-local temporary
  file in that directory, atomically renames it to `rootfs.ext4`, and then grows the copy only
  when the requested size exceeds the source size.
- A request is rejected before copying when it is below the registry-declared image minimum or
  below the physical source image size. Missing registry minimum metadata is a typed prerequisite
  error; the SDK never infers a minimum from the file.
- Growth is performed by an ext4 storage adapter using a file-size expansion followed by an
  offline filesystem resize. The adapter verifies the final byte size and ext4 metadata. Shrink
  is never attempted.
- The registry model and SQLite persistence must carry the declared minimum size as optional
  metadata so that absence remains distinguishable from zero. The create preflight rejects an
  image with no declared minimum.

The exact helper binaries (`resize2fs`, filesystem inspection, and mount helpers) are invoked by
an injected typed host-command port. Arguments are passed as individual values, output is captured,
and command failures become typed SDK errors; no shell command string is assembled by domain code.

### Host-only networking

- Firecracker supports a TAP backend. The smallest private IPv4 network with one host endpoint and
  one guest endpoint is a `/30`.
- Each host-only VM receives a non-overlapping `/30` from a deterministic SDK pool, a unique TAP
  name, a host endpoint, a guest endpoint, and a gateway equal to the host endpoint.
- Host forwarding and an SDK-owned nftables NAT/forward rule provide outbound connectivity. The
  SDK checks the live route table and firewall state before allocating a subnet and compares each
  resource with persisted ownership during later reconciliation.
- The guest receives a static kernel `ip=` parameter containing guest address, gateway, netmask,
  and interface name. No per-VM DHCP service is created.
- Correct TAP, address, forwarding, and NAT resources are skipped by the public network
  reconciliation operation. A resource with a different owner is a conflict, not a resource to
  overwrite.

### LAN networking

- Firecracker's documented alternatives are NAT-based and bridge-based routing. Bridge-based
  routing is the selected design because LAN exposure is an explicit user choice.
- The Linux adapter detects the host interface used by the default route, creates or reuses one
  SDK-managed bridge for that uplink, attaches the uplink and each VM TAP to the bridge, and
  preserves the host's existing addresses/routes while moving link ownership where required by
  the host network manager.
- The VM receives a unique MAC and requests an address with the guest DHCP boot parameter. The
  SDK temporarily starts the VM, discovers the lease by the VM MAC on the bridge, rejects missing
  or conflicting results, persists the last lease/address, and stops the VM before success.
- The SDK does not run a DHCP server, invent a static LAN address, or silently fall back to
  host-only mode. Unsupported uplink management, missing DHCP, or insufficient permissions are
  typed errors with rollback of attempt-owned bridge/TAP/firewall changes.
- Bridges are host resources shared by multiple LAN VMs. Their ownership and reference count are
  persisted separately from each VM so reconciling or rolling back one VM cannot remove another
  VM's attachment.

Sources: [Firecracker network setup](https://github.com/firecracker-microvm/firecracker/blob/main/docs/network-setup.md),
[Firecracker getting started](https://github.com/firecracker-microvm/firecracker/blob/main/docs/getting-started.md).

### Temporary runtime and final lifecycle state

- Creation is synchronous. The durable success state is `configured`, which means stopped and
  ready for a future lifecycle operation.
- Host-only creation can finish without starting the guest because the static address and offline
  key injection are deterministic. LAN creation must use a temporary runtime to obtain DHCP.
- If a temporary start is used, the runtime adapter waits for the expected condition with a fixed
  internal 30-second deadline (tests inject a shorter deadline), stops the exact process it
  started, waits for exit, removes the socket, and verifies that no active process or socket
  remains before committing `configured`.
- The runtime adapter keeps the Firecracker command and process handle private. The SDK never
  writes runtime diagnostics to stdout/stderr or installs a global logger.
- Creation never invokes future list, inspect/status, start, stop, reboot, or delete operations.

### Artifact pair compatibility

- Firecracker and `firectl` remain independent inventory records. Resolution selects one verified
  executable component named `firecracker` and one named `firectl` for the host architecture.
- The conservative compatibility rule for this feature is exact release-version equality between
  the selected package versions, in addition to matching architecture and valid executable
  metadata. This avoids making an unverified API pairing decision when the registry offers
  multiple packages.
- If no unique exact-version pair exists, creation returns a typed runtime prerequisite or
  compatibility error. It never downloads, guesses, or selects an arbitrary pair.

### Persistence and failure safety

- Existing artifact tables remain the source of truth for readiness. A new migration adds VM,
  network, bridge, credential, and runtime metadata without copying private key material into the
  database.
- The operation acquires a per-name lock and uses a `creating` provisional record. Host resources
  are owned through explicit attempt bookkeeping. The final SQLite update to `configured` happens
  only after all checks and cleanup are complete.
- Any post-preflight failure stops the attempt process, removes the active socket, undoes owned
  network state, removes generated keys and the copied rootfs, and removes the provisional record.
  Caller-owned source images, existing directories, files, and host networking are never removed.
- Repeated identical requests return the existing configured record after revalidating its immutable
  configuration; a different immutable request returns a typed conflict.

## Resolved design decisions

1. The public feature surface is `create_microvm(request)` plus
   `configure_network(vm_name)`. The latter reads persisted desired network state and reports
   applied/skipped resources.
2. The VM volume is a directory. Its stable children are `rootfs.ext4`, `ssh/id_ed25519`,
   `ssh/id_ed25519.pub`, and the expected `firecracker.sock` path.
3. The caller supplies both distribution ID and exact image ID. The distribution supplies its
   default kernel.
4. Disk and RAM requests are bytes; vCPU is a count. Firecracker memory is passed in whole MiB,
   using a checked ceiling conversion and persisting both requested and effective values.
5. Host-only networking is the default and uses a `/30`, TAP, static guest boot parameters, and
   host NAT. LAN networking uses an SDK-managed bridge, detected uplink, and external DHCP.
6. SSH uses per-VM Ed25519 credentials, fixed guest path `/root/.ssh/authorized_keys`, and
   `root:22`; only the public key is injected.
7. The successful state is configured and stopped. There is no persistent process or active socket
   after creation or network reconciliation.
8. The implementation is confined to the SDK; the CLI receives no lifecycle implementation in
   this feature.

No unresolved technical questions remain for Phase 1 design.
