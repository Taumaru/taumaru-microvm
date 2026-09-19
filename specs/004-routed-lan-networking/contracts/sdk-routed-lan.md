# SDK Contract: Routed LAN Networking

This contract extends the creation contract with routed-LAN deltas only. It contains
no CLI command, Firecracker command line, shell string, SQLite statement,
private-key material, or registry HTTP detail. Unchanged creation behavior keeps its
existing contract.

## Public types

Re-exported from `crates/sdk/src/lib.rs` with Rustdoc invariants:

```rust
pub struct CreateMicroVmRequest {
    pub name: String,
    pub distribution_id: String,
    pub image_id: String,
    pub disk_size_bytes: u64,
    pub vcpu_count: u32,
    pub memory_bytes: u64,
    pub expose_on_lan: bool,
    // New: explicit LAN override. None selects automatically.
    pub lan_address: Option<std::net::Ipv4Addr>,
    pub volume_path: Option<std::path::PathBuf>,
}

pub struct NetworkConfiguration {
    pub mode: NetworkMode,
    pub guest_address: std::net::IpAddr,   // private guest endpoint for LAN
    pub prefix_length: u8,                 // 30 for the private /30
    pub gateway: Option<std::net::IpAddr>, // TAP host endpoint
    pub tap_name: String,
    pub bridge_name: Option<String>,       // None for routed LAN
    pub uplink_name: Option<String>,       // default-route interface
    pub lan_address: Option<std::net::IpAddr>, // Some for LAN, None for host-only
}

pub enum NetworkResource {
    Tap,
    TapAddress,
    HostRoute,      // NEW: <lan>/32 through the TAP
    ProxyArpEntry,  // NEW: proxy entry for <lan> on the uplink
    Forwarding,     // shared sysctl state (forwarding + proxy ARP)
    ForwardRule,    // NEW: per-VM iptables FORWARD rule
    Nat,            // host-only NAT (now iptables-backed)
    IptablesNat,    // NEW: LAN private-range MASQUERADE rule
    FirecrackerInterface,
    // Legacy bridge/DHCP kinds remain parseable for old rows only.
}
```

`NetworkConfigurationResult` is unchanged in shape (`applied`/`skipped` per
resource); routed kinds appear in those lists.

## Public operations

```rust
impl MicroVmSdk {
    pub async fn create_microvm(
        &self,
        request: CreateMicroVmRequest,
    ) -> Result<MicroVmCreationResult, SdkError>;
    pub async fn configure_network(
        &self,
        vm_name: &str,
    ) -> Result<NetworkConfigurationResult, SdkError>;
}
```

Required behavior deltas:

1. `lan_address: Some(_)` with `expose_on_lan: false` is an invalid request; no
   host mutation follows.
2. LAN creation detects the default uplink (interface, global CIDR, gateway)
   before mutation; missing route or address is a typed network error.
3. LAN selection order: explicit override, then previous assignment when still
   valid and free, then automatic duplicate-checked search. Out-of-subnet,
   reserved, duplicate, or exhausted outcomes are typed errors with rollback.
4. Host setup applies TAP, `<host>/30`, up state, `<lan>/32` route, uplink proxy
   ARP entry, forwarding/proxy sysctls, and owned `iptables` rules, each recorded
   for ownership-aware cleanup.
5. The guest boots private-only, then the SDK applies LAN addressing, routes, and
   DNS over post-boot SSH with the VM key inside a bounded temporary run, verifies
   private/LAN/egress reachability, stops the exact process, and removes the
   socket before returning `Configured` stopped.
6. `configure_network` restores only missing or SDK-owned stale routed items,
   keeps committed addresses stable, never disturbs another VM, and leaves the VM
   stopped.

## Error contract

Existing `SdkError` categories carry the routed deltas; no new top-level variant
is required:

| Category | Routed distinguishing data |
|---|---|
| Invalid request | Field `lan_address` with shape/scope reason (override without LAN mode, non-IPv4 use). |
| Network or permission failure | Mode `lan`, operation/resource (`detect uplink`, `allocate LAN address`, `probe duplicate`, `publish proxy entry`, `apply iptables rule`, TAP/route identity), privilege hint with command plus diagnostic output. |
| Storage conflict | Competing LAN address, TAP, or MAC owner when a foreign resource blocks setup. |
| Temporary runtime/startup failure | Guest SSH setup condition (private SSH unreachable, guest command failed, verification failed) with stopped-state result. |
| Cleanup failure | Primary failure plus each owned routed resource that could not be proven removed. |

Permission-denied output is never reported as object absence. No error carries
private-key content or guest secrets.

## Side-effect and rollback contract

- Silent public operations; typed `Result` errors only.
- Rollback order: guest-visible claims, owned `iptables` rules, proxy entry, host
  route, TAP, keys/rootfs, provisional row. Shared sysctls and chains persist
  while another configured VM references them.
- Idempotent repeat returns the existing configured VM after revalidation;
  differing immutable fields (including the LAN override) conflict without
  mutation.
- Successful LAN creation leaves the VM stopped with an inactive socket and a
  host configuration usable by a future start.

## Example usage

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
        lan_address: None, // or Some("192.168.3.50".parse()?),
        volume_path: None,
    })
    .await?;

assert_eq!(created.network.uplink_name.is_some(), true);
assert!(created.network.lan_address.is_some());

let reconciled = sdk.configure_network("lan_vm").await?;
assert_eq!(reconciled.state, MicroVmState::Configured);
```
