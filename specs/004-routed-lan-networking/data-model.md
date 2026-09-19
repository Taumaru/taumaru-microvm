# Phase 1 Data Model: Routed LAN Networking

## Domain records

### `CreateMicroVmRequest` (extended)

New optional field alongside the existing validated fields:

| Field | Type | Rules |
|---|---|---|
| `lan_address` | `Option<Ipv4Addr>` | `None` selects automatically; `Some` is an explicit LAN override. Only meaningful with `expose_on_lan: true`; present with `false` is an invalid request. Domain validation checks shape only (IPv4 literal); subnet membership, reserved addresses, and availability are adapter concerns. Part of the idempotency comparison. |

Existing fields keep their contracts (`name`, `distribution_id`, `image_id`,
`disk_size_bytes`, `vcpu_count`, `memory_bytes`, `expose_on_lan`, `volume_path`).

### `RoutedLanMapping`

The per-VM unit of allocation, persistence, reconciliation, and teardown:

| Field | Meaning |
|---|---|
| `vm_id` / `name` | Stable VM identity; one mapping per VM. |
| `private_host` | TAP host endpoint of the VM's exclusive `/30` (also the guest gateway). |
| `private_guest` | Guest endpoint of the same `/30`; SSH target during setup. |
| `lan_address` | VM's LAN address inside the uplink subnet; `/32` on host and guest. |
| `tap_name` | Deterministic `tm-<hash>` interface, unique across VMs. |
| `guest_mac` | Locally administered `02:fc:…` MAC, unique across VMs. |
| `uplink_name` | Default-route interface at setup time (e.g. `wlp1s0`, `enp0s3`). |
| `uplink_cidr` | Host CIDR the LAN address was selected from; used to validate reuse. |
| `gateway` | Uplink gateway; guest DNS/default-route context, not moved. |

Invariants: `lan_address` is inside `uplink_cidr`, is not network/broadcast/host/gateway,
is unique across configured VMs, and pairs 1:1 with one TAP and one MAC.

### `LanAddressOffer`

Candidate plus evidence; exactly one offer is committed per VM:

| Field | Meaning |
|---|---|
| `candidate` | Proposed LAN address. |
| `source` | `explicit_override`, `previous_assignment`, or `automatic_search`. |
| `availability` | Duplicate-detection outcome (ARP result or documented fallback). |
| `uplink_cidr` | Subnet the candidate was validated against. |

Selection order: explicit override, then previous assignment when still valid and
free, then automatic search preferring the upper subnet range.

### `HostForwardingState`

Host-side resources owned by one mapping:

| Item | Ownership rule |
|---|---|
| TAP device + `<host>/30` + up state | Per-VM; created, reconciled, deleted by name. |
| Host route `<lan>/32 dev <tap>` | Per-VM; `replace` on setup, `del` on rollback. |
| Proxy ARP entry `<lan> dev <uplink>` | Per-VM; `replace` on setup, `del` on rollback. |
| `net.ipv4.ip_forward=1` | Shared sysctl; restore only when no configured VM claims it. |
| `net.ipv4.conf.<uplink>.proxy_arp=1` | Shared sysctl; same reference rule. |
| `iptables` FORWARD/NAT rules with `sdk:taumaru:<tap>` comment | Per-VM exact rules; `-C`/`-A`/`-D` by spec. |

Skipped when already correct, repaired when missing or SDK-owned stale, never
touching foreign rules or chains.

### `GuestNetworkEndState`

Observable guest configuration after setup:

| Field | Value |
|---|---|
| `private_address` | `<private-guest>/30` on the primary interface. |
| `lan_address` | `<lan>/32` on the same interface. |
| `default_route` | `via <private-host> dev <guest-iface> src <private-guest>`. |
| `dns` | Public resolvers (`1.1.1.1`, `8.8.8.8`). |

Reached by booting private-only (`ip=<private>::<host>:255.255.255.252::eth0:off`)
then post-boot SSH with the VM key. Verified from outside (private ping, LAN ping,
guest egress) rather than by guest introspection.

### `VmNetwork` (persisted, extended)

Existing `vm_networks` row gains routed columns via `0003_routed_lan.sql`:

| Column | Content |
|---|---|
| existing | `mode`, `guest_ip` (private guest for LAN), `prefix_length`, `gateway_ip`, `host_ip`, `tap_name`, `guest_mac`, `desired_boot_parameters` (private-only `ip=`), resource rows; unchanged semantics. |
| `lan_ip` | Committed LAN address (new, required for LAN). |
| `uplink_cidr` | Subnet the LAN address was selected from (new). |
| `proxy_arp_enabled_by_sdk` | Whether this VM caused proxy ARP enablement (new). |
| legacy | `bridge_id`, `bridge_created_by_sdk`, `uplink_attached_by_sdk`, `nat_table_created_by_sdk`, `nat_chain_created_by_sdk`, `host_address_specs`, `default_route_specs`, `dhcp_lease_reference`; untouched columns, ignored for new routed rows. |

`NetworkConfiguration` exposes the LAN address for LAN mode; `bridge_name` is
`None` for routed VMs. `NetworkResource` gains `HostRoute`, `ProxyArpEntry`,
`ForwardRule`, and `IptablesNat` kinds for reconcile/rollback identity.

### `VmNetworkResource` (ownership)

Each row keeps VM key, resource kind, stable identity, desired fingerprint,
`sdk:taumaru` ownership marker, optional adapter handle (iptables rule spec or
comment), and last-observed state. Identity/fingerprint comparison drives
skip/repair/conflict decisions; legacy `nft` rows are never matched by new code.

### Unchanged records

`MicroVmRecord`, `VmCredential`, `VmRuntime`, artifact inventory, and host-only
addressing keep their contracts. Host-only NAT moves to `iptables` with identical
observable semantics.

## Lifecycle and ownership transitions

```text
absent
  │ preflight (artifacts, uplink, LAN offer, locks) succeeds
  ▼
creating ── host net (TAP, route, proxy, iptables) ──► creating+net
   │                                                          │ boot private-only
   │                                                          ▼
   │                                              guest setup over SSH (LAN/DNS)
   │                                                          │ verify, stop, socket removed
   │                                                          ▼
   └────────────────────────────────────────────────────► configured (stopped)
   │
   └─ any failure ── rollback owned items (rules, proxy, route, TAP, files, row) ──► absent
```

Rules:

1. Addresses are committed in SQLite only after host verification; the LAN offer
   is held by attempt ownership until commit.
2. Identical repeat requests return the existing configured record after
   revalidation; differing immutable fields (including the LAN override) conflict.
3. `running` exists only inside the bounded temporary guest-setup run.
4. Reconciliation never allocates a second LAN address for a configured VM and
   never disturbs another VM's mapping.
5. Rollback deletes in reverse dependency order and preserves shared sysctls and
   chains/rules it did not create.

## SQLite migration shape

Add `crates/sdk/migrations/0003_routed_lan.sql`, register as version 3 in
`adapters/persistence/migrations.rs`, and extend required-schema verification.
The migration adds routed columns to `vm_networks` with checks (valid LAN IP
present for LAN mode, consistent prefix expectations) and keeps every legacy
column. Repository methods grow symmetric load/persist coverage for the new
columns; `NetworkResource::parse`/`as_str` cover the new kinds.

Foreign keys keep cascading cleanup for VM-owned children. Bridge-table rows are
no longer written by new LAN code; existing rows are left alone.

## Validation and invariants

- Every persisted path stays normalized and inside the explicit SDK home unless
  the caller supplied an allowed external volume path.
- TAP, MAC, private `/30`, and LAN address are unique across configured VMs.
- LAN override without `expose_on_lan` is rejected; out-of-subnet, reserved, or
  duplicate selections are typed errors before guest-visible commitment.
- A configured LAN VM has exactly one committed LAN address and one private
  `/30`; guest SSH metadata still references `root:22` and the private key path.
- Rollback provability: attempt-owned TAP/route/proxy/rules are verified absent
  after failure; shared state is verified preserved.
