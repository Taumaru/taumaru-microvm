# Quickstart: Routed LAN Validation

This feature is consumed through the Rust SDK only. It changes LAN networking;
host-only behavior is unchanged. Run privileged (root or equivalent) for every
host-mutating step; unprivileged runs are expected to fail fast with a typed
privilege error.

## Host prerequisites

- Linux with KVM and a readable/writable `/dev/kvm`.
- One default IPv4 uplink (Wi-Fi or Ethernet) with a global address.
- Privilege to manage TAP, addresses, routes, proxy ARP entries, sysctls, and
  `iptables` rules.
- Duplicate-detection tooling: `arping` preferred, `ping` fallback documented.
- Verified inventory: caller-selected image, default kernel, Firecracker plus
  `firectl` executables, and the image's registry-reported `size_bytes`.

The SDK home is supplied explicitly and holds `state/inventory.db`, artifacts,
and default VM volumes.

## Prepare artifacts

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;
sdk.download_distribution("ubuntu-24.04", |_| {}).await?;
// Kernel and runtime downloads follow the existing acquisition flow.
```

Creation never downloads missing artifacts.

## Create a routed LAN VM

```rust
let created = sdk
    .create_microvm(CreateMicroVmRequest {
        name: "lan_vm".to_owned(),
        distribution_id: "ubuntu-24.04".to_owned(),
        image_id: "ubuntu-24.04-minimal".to_owned(),
        disk_size_bytes: 16 * 1024 * 1024 * 1024,
        vcpu_count: 2,
        memory_bytes: 2 * 1024 * 1024 * 1024,
        expose_on_lan: true,
        lan_address: None,
        volume_path: None,
    })
    .await?;
```

Expected: `state` is `Configured` stopped, no active socket or process, one
private `/30` plus one LAN address, host route and proxy entry present, owned
`iptables` rules present, guest reachable at both addresses, guest egress works.
With `lan_address: Some(explicit)` the same holds for the requested address when
free, else a typed error with rollback.

## Create a host-only VM (regression)

Same request with `expose_on_lan: false` and no `lan_address`. Expected: existing
host-only contract (exclusive `/30`, TAP, static guest boot parameters, host NAT,
no LAN route or proxy entry).

## Reconcile after host state loss

```rust
let reconciled = sdk.configure_network("lan_vm").await?;
```

Wipe host state (TAP, route, proxy entry, owned rules) while keeping the
database, then reconcile. Expected: same LAN and private addresses restored, no
duplicate resources, VM stopped.

## Failure drills

- No default route or no global uplink IPv4: typed error before mutation.
- Explicit address outside the subnet or reserved: invalid-request error.
- Exhausted candidates: allocation error without claiming an address.
- Unprivileged run: privilege error naming the missing capability with rollback.
- Forced mid-attempt failure: zero attempt-owned host resources remain; shared
  chains and unrelated rules preserved.

## Verification commands

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Host integration (TAP/route/proxy/`iptables`/SSH guest setup) is
capability-gated and runs only with privilege on Linux; the default suite uses
injected ports for allocation, idempotency, reconcile skip/repair, rollback, and
failure-path coverage without host mutation.

Details live in [data-model.md](./data-model.md) and
[contracts/sdk-routed-lan.md](./contracts/sdk-routed-lan.md).
