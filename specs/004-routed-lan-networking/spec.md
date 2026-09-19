# Feature Specification: Routed LAN Networking

**Feature Branch**: `004-routed-lan-networking`

**Created**: 2026-09-19

**Status**: Draft

**Input**: User description: "crie um spec para fazermos a migração do setup e configuração da rede para usar o mesmo padrão do meu .sh ao invés de reenventarmos a roda! E o sdk tem que considerar que vai ser rodado como sudo por padrão..."
## Clarifications

### Session 2026-09-19

- Q: How should the SDK configure the guest's LAN address, routes, and DNS inside the VM? → A: Boot with private address only, then post-boot SSH using the VM key to apply LAN address, routes, and DNS (Option B).
- Q: What should happen to VMs already stored with the old bridge-mode LAN state? → A: Ignore them; no migration or compatibility is required because the feature is pre-release with no users.
- Q: Which packet-filter tool must the SDK's implementation use for the forwarding and NAT rules? → A: Switch to the iptables backend like the proven script (Option B).
- Q: How should the VM's LAN address be chosen? → A: Automatic free-address selection by default with an optional explicit override supplied as a customized SDK creation parameter (Option A).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Create a LAN-Reachable MicroVM Over Any Uplink (Priority: P1)

As an application developer, I want to create a MicroVM that is reachable on the host's LAN even when the host's default uplink is Wi-Fi, so that other devices on the LAN can reach the VM without requiring a Linux bridge enslaving the uplink.

**Why this priority**: This is the core value of the migration. The current bridge-enslavement approach fails on Wi-Fi station interfaces by kernel design, blocking LAN mode on the most common developer host. A routed approach works identically over Wi-Fi and Ethernet.

**Independent Test**: Can be fully tested by creating one VM with LAN exposure enabled on a Wi-Fi host, then verifying from another LAN device that the VM's LAN address answers traffic, while the host itself can reach the VM over both its private address and its LAN address.

**Acceptance Scenarios**:

1. **Given** a valid creation request with LAN exposure enabled on a host whose default route points at any uplink type (Wi-Fi or Ethernet), **When** creation succeeds, **Then** the VM receives one stable private host-only address and one LAN address inside the uplink's subnet, the host routes the LAN address to the VM's TAP interface, the host answers LAN ARP queries for the VM address on behalf of the VM, and forwarding plus address translation let the VM reach the LAN and the Internet while the LAN can reach the VM at its LAN address.
2. **Given** a LAN creation request, **When** the VM boots and finishes guest configuration, **Then** the guest exposes both its private address (with a host-scoped prefix) and its LAN address (as a host route) on its primary interface, uses the TAP host address as its default gateway with the private address as source, and can resolve names and reach the Internet.
3. **Given** two VMs created with LAN exposure on the same host, **When** both are configured, **Then** each VM holds a distinct LAN address and a distinct private subnet, and traffic to one VM's LAN address never reaches the other VM.

---

### User Story 2 - Run Network Setup With Elevated Privileges By Default (Priority: P1)

As an operator running the SDK with administrative privileges, I want network setup to assume elevated privileges are present, so that TAP creation, addressing, routing, ARP proxying, and packet-filter rules succeed in the normal path and failures clearly state which privilege is missing.

**Why this priority**: Every host-network mutation requires elevated privilege. Designing for the privileged default removes an entire class of misleading failures and matches how the tool is actually operated.

**Independent Test**: Can be fully tested by running creation with effective administrative privileges (succeeds end to end) and once without them (fails fast with a typed privilege error naming the missing capability before any partial guest-visible state is reported as configured).

**Acceptance Scenarios**:

1. **Given** the SDK runs with effective administrative privileges, **When** creation configures host networking, **Then** TAP creation, address assignment, route installation, ARP proxy entries, forwarding enablement, and filter/NAT rules are all applied without privilege errors.
2. **Given** the SDK runs without the required network privilege, **When** creation reaches the first privileged mutation, **Then** it returns a typed error that names the missing privilege, stops before reporting the VM as configured, and rolls back everything the attempt created.
3. **Given** files and directories are created while running privileged, **When** creation finishes, **Then** managed paths carry ownership and modes consistent with privileged operation and remain usable by subsequent privileged SDK operations without manual repair.

---

### User Story 3 - Reconcile Routed LAN State After a Host Restart (Priority: P2)

As an application developer, I want to reapply a VM's routed LAN configuration after a host restart, so that already-correct routes, proxy entries, and filter rules are skipped while missing ones are repaired without allocating a second LAN address or disturbing other VMs.

**Why this priority**: Host network state is ephemeral while VM identity is durable. Reconciliation is what makes routed LAN survivable across reboots.

**Independent Test**: Can be fully tested by creating a LAN VM, wiping host-side state (TAP, route, proxy entry, filter rules) while keeping the database, invoking the network reconciliation operation, and verifying the VM becomes reachable again at the same LAN address with no duplicate resources.

**Acceptance Scenarios**:

1. **Given** a configured LAN VM whose host-side resources were lost, **When** the caller invokes network reconciliation, **Then** the SDK restores only the missing or stale items, keeps the previously assigned LAN and private addresses, and leaves the VM stopped.
2. **Given** a configured LAN VM whose host-side resources are already correct, **When** reconciliation runs, **Then** the SDK skips each correct item without allocating a new address, TAP, route, proxy entry, or filter rule.
3. **Given** one VM is reconciled, **When** another VM is created or reconciled, **Then** the operation never reuses, replaces, or exposes the first VM's addresses, TAP, proxy entry, or filter rules.

---

### User Story 4 - Roll Back Routed LAN Attempts Cleanly (Priority: P2)

As an application developer, I want a failed LAN creation to leave no routable trace of the attempt, so that retries and other VMs start from a clean host state.

**Why this priority**: Routed mode touches shared host state (routes, ARP proxy table, filter chains). Leftovers would hijack traffic or block retries.

**Independent Test**: Can be fully tested by forcing a failure after network setup (e.g., invalid downstream step) and verifying the TAP, host route, proxy entry, and attempt-owned filter rules are gone while pre-existing shared containers are preserved.

**Acceptance Scenarios**:

1. **Given** a LAN creation attempt fails after applying host network resources, **When** the operation returns, **Then** the TAP, the host-side LAN route, the proxy ARP entry, and every filter/NAT rule owned by the attempt are removed, and shared tables or chains that pre-existed are left intact.
2. **Given** a LAN creation attempt fails, **When** the caller retries with the same request, **Then** the retry can select the same LAN address when it is still free and succeeds without conflicting with leftovers from the first attempt.

### Edge Cases

- The host has no default route or no global IPv4 address on the default interface: creation returns a typed network error before mutating host state.
- The requested explicit LAN address lies outside the uplink subnet, equals the network/broadcast address, or equals the host or gateway address: creation rejects it as invalid input.
- No apparently free LAN address exists (every candidate answers duplicate detection): creation returns a typed allocation error without claiming an address.
- A candidate LAN address becomes taken between selection and configuration: the attempt fails with a typed conflict and rolls back; it never configures a duplicate address.
- The uplink is Wi-Fi station mode: creation succeeds through the routed path and never attempts bridge enslavement of the wireless interface.
- The process lacks the required privilege mid-attempt: the attempt fails with a typed privilege error and rolls back applied items; pre-existing shared filter containers are preserved.
- The packet-filter backend is iptables: existence probes, rule ownership, and cleanup use iptables semantics.
- A previous release's bridge-mode LAN record exists in the database: it is ignored because the feature is pre-release with no users; no migration or compatibility is provided.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The SDK MUST expose LAN exposure as an explicit per-VM choice that results in one stable private host-only address plus one LAN address inside the default uplink's subnet. LAN creation MUST accept an optional customized creation parameter carrying an explicit LAN address override; when omitted, the SDK selects an apparently free address automatically. The feature MUST modify only SDK behavior; no CLI lifecycle implementation or separate orchestration path is part of this feature.
- **FR-002**: For LAN mode the SDK MUST NOT enslave the host uplink to a Linux bridge. It MUST attach the VM through a VM-specific TAP interface and make the VM's LAN address reachable by routing a host-side host route for that address through the TAP plus host proxy-ARP answers for that address on the uplink interface.
- **FR-003**: The SDK MUST detect the default uplink interface, its global IPv4 address and prefix, its gateway, and the LAN subnet derived from them before allocating a LAN address. Missing default route or missing global IPv4 address MUST produce a typed network error before host mutation.
- **FR-004**: The SDK MUST allocate a LAN address that belongs to the uplink subnet, is neither the network nor the broadcast address, and is not the host or gateway address. It MUST verify apparent availability through duplicate detection (ARP-based when available, with a documented fallback) and MUST support a caller-requested explicit LAN address validated the same way, plus reuse of the VM's previously assigned LAN address when it is still valid and free.
- **FR-005**: For each LAN VM the SDK MUST create and configure a VM-specific TAP interface with a deterministic name fitting Linux interface limits, assign the TAP host endpoint address, bring the interface up, install a host route for the VM's LAN address through the TAP, and publish a proxy ARP entry for the VM's LAN address on the uplink interface.
- **FR-006**: The SDK MUST enable IPv4 forwarding and proxy ARP on the uplink as part of LAN setup, and MUST install iptables packet-forwarding rules that permit TAP-to-uplink forwarding, established return traffic to the private guest address, and uplink-to-TAP traffic destined for the VM's LAN address, plus iptables address translation confined to the private host-only source range so LAN-bound traffic keeps the VM's LAN identity.
- **FR-007**: After boot, the guest MUST end with its private address (host-scoped prefix) and its LAN address (host route) on its primary interface, a default route through the TAP host address with the private address as source, and working name resolution. The SDK MUST boot the guest with only the private address configured, then connect post-boot over SSH with the VM's generated key to apply the LAN address, routes, and name resolution, without requiring the caller to configure the guest manually.
- **FR-008**: The SDK MUST persist per VM the private and LAN addresses, TAP identity, uplink identity, guest hardware address, forwarding/NAT/proxy ownership flags, and every host-side detail required to reconcile, inspect, and tear down the mapping later. Repeating an identical creation request MUST be idempotent and return the existing VM without duplicating addresses, interfaces, routes, proxy entries, or rules.
- **FR-009**: The SDK MUST reconcile persisted routed-LAN state: skip already-correct items, repair only missing or stale items, keep previously assigned addresses stable, never disturb another VM's resources, and leave the VM stopped on return.
- **FR-010**: On any failed LAN attempt the SDK MUST roll back everything the attempt created (TAP, host route, proxy entry, attempt-owned filter rules, guest-visible addressing claims) while preserving pre-existing shared tables, chains, and unrelated rules. Insufficient privilege, allocation failure, or unreachable uplink MUST surface as typed errors, never as a silently degraded mode or an automatic fallback to another network mode.
- **FR-011**: The SDK MUST assume elevated network privilege is the normal operating mode. Privileged host mutations MUST be attempted directly; when the effective privilege is absent the SDK MUST return a typed privilege error naming the missing capability, including the failed command and its diagnostic output. Permission-denied results MUST never be interpreted as "object absent" during existence probes.
- **FR-012**: Failure diagnostics for host commands MUST include the executed command, its exit status, and its diagnostic output, plus an privilege hint when the output indicates a privilege denial, so operators can distinguish missing privilege from syntax, state-conflict, and not-found failures without re-running commands by hand.
- **FR-013**: Host-only mode MUST keep its current observable contract: exclusive private `/30` per VM, VM-specific TAP, host NAT for outbound traffic, guest reachable from the host, and no LAN route or proxy entry.

### Key Entities

- **Routed LAN Mapping**: The per-VM association of private guest address, LAN address, TAP interface, uplink interface, guest hardware address, and gateway. It is the unit of allocation, persistence, reconciliation, and teardown.
- **LAN Address Offer**: A candidate LAN address plus its availability evidence (duplicate-detection outcome, explicit request vs. automatic selection, previous-assignment reuse). Only one offer is committed per VM.
- **Host Forwarding State**: The set of host-side resources owned by one VM mapping: TAP device and addresses, host route for the LAN address, uplink proxy ARP entry, forwarding and proxy sysctls state, and packet-filter rules. Ownership flags decide what setup creates, what reconciliation skips, and what rollback removes.
- **Guest Network End-State**: The observable guest configuration: private address, LAN address, default route with private source address, and name resolution. It is verified from outside the guest (reachability) rather than by inspecting guest internals.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a Wi-Fi host, a LAN creation completes and the VM's LAN address answers inbound traffic from another LAN device within 2 minutes of the request.
- **SC-002**: After creation, the host reaches the VM at both its private and LAN addresses, and the guest reaches public names and addresses, on the first attempt in 95 of 100 runs on a quiet LAN.
- **SC-003**: After deleting host-side state and reconciling, the VM becomes reachable again at the same LAN address with no duplicate host network resources in 100% of trials.
- **SC-004**: A failed LAN attempt leaves zero attempt-owned host network resources (verified by host network inspection) in 100% of injected-failure trials.
- **SC-005**: Running without the required privilege produces an error naming the missing privilege within 30 seconds, before any VM is reported as configured.
- **SC-006**: Two LAN VMs on one host hold distinct LAN and private addresses with no cross-delivery of traffic in every trial.

## Assumptions

- The SDK runs with effective administrative privilege (root or equivalent capabilities) as the default operating mode; unprivileged execution is a diagnosed error path, not a supported setup path.
- Files and directories created under privilege are expected to be root-owned; subsequent SDK operations run under the same privilege and handle that ownership without manual repair.
- The host is Linux with one default IPv4 uplink (Wi-Fi or Ethernet), exactly one LAN subnet of interest, and standard host tooling for links, addresses, routes, ARP proxy entries, sysctls, duplicate detection, and packet filtering available.
- The packet-filter backend is iptables; observable rule semantics (forwarding scope, translation confined to the private source range, ownership tracking, safe cleanup) MUST match the proven operational pattern.
- The guest's primary interface name and its ability to hold a private address plus a LAN host route with a default gateway are stable platform facts recorded during planning; guest images are expected to support them.
- Records created by the previous bridge-mode LAN implementation need no migration or compatibility handling because the feature is pre-release with no users.
- Host-only behavior, SSH key injection, artifact prerequisites, idempotency semantics, and the Firecracker process boundary keep their existing contracts except where this spec explicitly changes LAN networking.
