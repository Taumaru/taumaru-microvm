# Phase 0 Research: Routed LAN Networking

## Research goals

Replace the bridge-enslavement LAN path with the proven routed pattern (per-VM TAP,
private host-only address plus LAN address, host `/32` route, uplink proxy ARP,
`iptables` forwarding/NAT, post-boot SSH guest setup), assuming effective root as the
normal operating mode. Host-only behavior, SSH key injection, artifact prerequisites,
idempotency, and the Firecracker process boundary keep their contracts except where LAN
networking explicitly changes.

All four clarification answers are locked: guest setup via boot-private then post-boot
SSH (Option B), legacy bridge records ignored (pre-release, no users), `iptables`
backend (Option B), automatic LAN selection with an explicit SDK creation parameter
override (Option A).

## Findings

### Why bridge enslavement cannot work on Wi-Fi

- `create_lan` moves host addresses/routes onto a bridge and runs
  `ip link set dev <uplink> master <bridge>` (`ensure_uplink_on_bridge` /
  `move_uplink_to_bridge` in `crates/sdk/src/adapters/network/linux.rs`). On a Wi-Fi
  station interface (`type managed`, e.g. `wlp1s0`) the kernel refuses with
  `Error: Device does not allow enslaving to a bridge` (exit 2). The access point only
  accepts the associated client MAC, so L2 bridging is impossible by 802.11 design, not
  by privilege or naming.
- The routed pattern never touches the uplink at L2: the AP only sees host-sourced
  frames, the host answers ARP for the VM address (proxy ARP) and routes a `/32` to
  the TAP. It works identically over Wi-Fi and Ethernet.

### Routed host setup (proven pattern)

- Detect the default uplink from the first default route (`ip route show default`):
  interface, gateway, plus the interface's global IPv4 CIDR
  (`ip -o -4 addr show dev <uplink> scope global`). No default route or no global
  IPv4 is a typed preflight error before mutation. Wireless detection via
  `/sys/class/net/<iface>/wireless` is informational only.
- Per LAN VM, from one shared private pool (existing `172.30.0.0/16`, `/30` per VM,
  collision-checked against persisted plus live state): deterministic TAP name
  `tm-<hash>` (existing `tap_name`, fits 15 chars), TAP host endpoint, private guest
  endpoint, locally administered MAC `02:fc:…` (existing `guest_mac`).
- Host operations in order: create TAP (no `user` binding needed under root), flush
  stale addresses, assign `<tap-host>/30`, bring up, `ip route replace <lan>/32 dev
  <tap>`, `sysctl net.ipv4.ip_forward=1`, `sysctl net.ipv4.conf.<uplink>.proxy_arp=1`,
  `ip neigh replace proxy <lan> dev <uplink>`.
- `iptables` rules per VM (idempotent `-C` check, `-A` add, `-D` delete, each tagged
  `-m comment --comment "sdk:taumaru:<tap>"` for ownership):
  - `FORWARD -i <tap> -o <uplink> -j ACCEPT`
  - `FORWARD -i <uplink> -o <tap> -d <private-guest>/32 -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT`
  - `FORWARD -i <uplink> -o <tap> -d <lan>/32 -j ACCEPT`
  - `POSTROUTING -t nat -s <private-/30> -o <uplink> -j MASQUERADE` (translation
    confined to the private range; LAN-bound traffic keeps the VM's LAN identity).
- Rollback and reconcile are the exact reverse: delete owned rules first, then proxy
  entry, host route, TAP; restore sysctls only when no other configured VM still
  claims them (DB reference check). Shared chains are never flushed; only exact
  owned rules are deleted.

Alternatives considered: keeping `nftables` and translating rule semantics (rejected:
two backends, proven pattern is `iptables`, pre-release allows unification); ebtables
or routed `proxy_arp_pvlan` variants (rejected: non-standard tooling, same observable
result with more moving parts).

### LAN address selection

- Candidate space is the uplink subnet minus network, broadcast, host, and gateway
  addresses. Preference order: explicit caller override (validated identically), then
  the VM's previously assigned LAN address when still valid and free, then automatic
  search preferring the upper end of the subnet (proven pattern scans the top ~200
  addresses downward).
- Availability evidence is duplicate detection at selection time: `arping -D -I
  <uplink> -c 2 -w 2 <candidate>` when the tool exists (exit 0 means free), else
  `ping -c 1 -W 1` fallback with its weaker guarantee documented. Persisted LAN
  addresses of other VMs plus live probes both count as conflicts.
- A candidate taken between selection and configuration is a typed conflict with
  full rollback; the SDK never configures a duplicate. Concurrent same-address
  races resolve to one success plus a typed conflict or a different free address.

Alternatives considered: DHCP for LAN (rejected: requires L2 adjacency, the exact
Wi-Fi failure being removed); SDK-owned DHCP server on the TAP (rejected: new
daemon, lease scope, and client compat work for zero gain over static routed
addresses).

### Guest end-state via post-boot SSH

- Boot the guest with only the private address configured, reusing the existing
  static `ip=` format (`ip=<private-guest>::<tap-host>:255.255.255.252::eth0:off`
  with `GUEST_INTERFACE = "eth0"`). No DHCP boot parameter remains for LAN.
- After boot, connect over SSH as `root` to the private address with the VM's
  generated key (`-i <volume>/ssh/id_ed25519`, `StrictHostKeyChecking=no`,
  `UserKnownHostsFile=/dev/null`, `ConnectTimeout=2`, `BatchMode=yes`,
  `LogLevel=ERROR`), wait bounded (60s, mirroring the proven script's
  `wait_for_ssh`), then apply exactly:
  - `ip link set eth0 up`
  - `ip addr replace <private>/30 dev eth0`
  - `ip addr replace <lan>/32 dev eth0`
  - `ip route replace default via <tap-host> dev eth0 src <private>`
  - `/etc/resolv.conf` with `nameserver 1.1.1.1`, `nameserver 8.8.8.8`,
    `options single-request-reopen`
- Verify from the host before stopping: private ping, LAN ping, and one guest
  egress check through the temporary run; then stop the exact process, remove the
  socket, and return `configured` stopped.
- Guest runtime addressing does not survive reboot, so the persisted mapping
  (private + LAN + gateway + DNS intent) is the source of truth for the future
  start flow, which must reapply the same guest commands on every boot. Creation-time
  verification plus host-side reconciliation are this feature's scope; persistent
  in-image network units are rejected as distro-specific complexity.

Alternatives considered: boot-parameters-only dual addressing (rejected: one `ip=`
entry cannot express private `/30` plus LAN `/32` with source routing);
offline image file edits via `debugfs` (rejected: distro-specific renderer
matrix for netplan/networkd/ifupdown, fragile against image variety).

### Privilege-by-default operation

- Attempt privileged mutations directly with typed `Command` argv and captured
  stderr. Existing `host_command_error` + `PRIVILEGE_HINT` already names the
  command, exit status, stderr excerpt, and the `CAP_NET_ADMIN` hint; keep and reuse
  it for every new `ip`/`iptables`/`sysctl`/`arping` call.
- Permission-denied output is never treated as object absence in existence probes
  (established `is_nft_missing` lesson carries over to `iptables -C`/`-S` parsing:
  only explicit "no such rule/chain" counts as absent).
- Root-owned volume files are the expected steady state; subsequent privileged
  operations must handle that ownership without manual repair. Unprivileged runs
  fail fast at the first privileged mutation with a typed privilege error and full
  rollback.

### Persistence shape

- New migration `0003_routed_lan.sql` adds routed columns to `vm_networks`
  (`lan_ip`, `uplink_cidr`, `proxy_arp_enabled_by_sdk`, rule-ownership detail)
  and keeps existing bridge/`nft` columns untouched but unused for new rows.
  Pre-release legacy bridge rows are ignored, never reinterpreted; no data
  migration of old LAN rows is required.
- `NetworkResource` gains routed kinds (host route, proxy ARP entry, iptables
  forward rules, iptables NAT rule); bridge/DHCP kinds stay for reading legacy
  rows only. `PersistedNetwork` carries the LAN address, uplink CIDR, and
  per-resource fingerprints; `NetworkConfiguration` exposes the LAN address
  publicly while `bridge_name` is `None` for routed VMs.
- Public `CreateMicroVmRequest` gains an optional `lan_address: Option<Ipv4Addr>`
  override (`None` selects automatically). Struct-literal callers add one field;
  validation rejects an override without `expose_on_lan` and rejects out-of-subnet
  or reserved values as invalid input; semantic availability is checked in the
  network adapter, not in domain validation. Idempotency comparisons include the
  override.

### Host-only preservation and unification

- Host-only keeps its observable contract (`/30`, TAP, static `ip=`, host NAT, no
  LAN route/proxy). Its NAT rule moves to the unified `iptables` backend with
  identical semantics (`POSTROUTING -s <private-/30> ! -o <tap> -j MASQUERADE`
  with ownership comment) so one backend, one cleanup path, and one privilege
  story cover both modes. Pre-release status makes orphaned `nft` rules from prior
  runs acceptable; documentation notes the one-time manual cleanup.

## Resolved design decisions

1. Routed L3 LAN replaces bridge enslavement for all uplinks; Wi-Fi and Ethernet
   share one code path.
2. `iptables` is the single packet-filter backend for LAN and host-only NAT.
3. LAN addresses are automatic by default with an optional explicit SDK creation
   parameter, duplicate-checked via ARP with ping fallback.
4. Guests boot private-only and receive LAN addressing plus DNS over post-boot SSH
   with the VM key during a bounded temporary run, then stop.
5. Root/`CAP_NET_ADMIN` is the normal mode; absence is a typed, diagnosable error
   with rollback.
6. Persistence is additive (`0003` migration, new resource kinds, new optional
   request/config fields); legacy bridge rows are ignored, not migrated.
7. Reconciliation is host-side and idempotent; the persisted mapping feeds future
   start-time guest reapplication.

No unresolved technical questions remain for Phase 1 design.
