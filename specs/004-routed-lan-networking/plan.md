# Implementation Plan: Routed LAN Networking

**Branch**: `004-routed-lan-networking` | **Date**: 2026-09-19 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/004-routed-lan-networking/spec.md`

## Summary

Replace bridge-enslavement LAN networking with a routed L3 design that works over
any uplink (Wi-Fi and Ethernet): per-VM TAP plus private `/30`, automatic or
explicit LAN address with duplicate detection, host `/32` route plus uplink proxy
ARP, `iptables` forwarding/NAT with ownership, and post-boot SSH guest setup
(private boot, then LAN address/routes/DNS applied with the VM key in a bounded
temporary run). Root/`CAP_NET_ADMIN` is the normal operating mode with typed
privilege errors otherwise. See [research.md](./research.md) for resolved
decisions.

## Technical Context

**Language/Version**: Rust 2024 edition, repository stable toolchain.

**Primary Dependencies**: Existing `rusqlite` (bundled SQLite), `tokio`, `serde`,
`serde_json`, `sha2`, `thiserror`, `reqwest`; typed `std::process::Command` argv
for `ip`/`iptables`/`sysctl`/`arping` with captured output; post-boot guest SSH
uses an `ssh` client invocation through the runtime/network boundary (no shell
strings, no new network daemon).

**Storage**: Existing artifact inventory plus new `0003_routed_lan.sql` migration
for routed columns; VM-local files keep `rootfs.ext4`, `ssh/id_ed25519{,.pub}`,
and `firecracker.sock` layout.

**Testing**: `cargo test` with injected network/runtime ports for allocation,
idempotency, reconcile skip/repair, rollback, and failure paths; capability-gated
host integration for TAP/route/proxy/`iptables`/SSH guest setup; existing unit,
persistence, public API, and failure-path suites extended.

**Target Platform**: Linux with KVM, `/dev/kvm` access, and privilege for TAP,
addresses, routes, proxy ARP, sysctls, and `iptables`; Wi-Fi station and Ethernet
uplinks share one routed path.

**Project Type**: Reusable SDK library in `crates/sdk`; no CLI lifecycle changes.

**Performance Goals**: Preflight before mutation; duplicate-checked LAN selection
favoring the upper subnet; idempotent repeat returns without reallocation;
bounded guest-setup wait (60s per proven pattern); reconciliation skips correct
resources; no background worker or unbounded wait.

**Constraints**: Silent typed-`Result` SDK; explicit home, no env discovery; no
implicit downloads; no caller-data mutation; no image shrink; no key-content
exposure; no foreign-resource overwrite; no active process/socket on success;
bridge enslavement is forbidden for LAN; permission-denied is never absence.

**Scale/Scope**: Multiple independent VMs per host; distinct LAN plus private
addresses per VM; this feature owns routed LAN setup, guest LAN application,
reconcile, and rollback. Future start-time guest reapplication consumes the
persisted mapping but is outside creation scope.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle behavior; CLI remains thin | PASS: SDK-only routed networking | PASS: no CLI source changes or duplicate orchestration |
| Public SDK is typed, silent, non-panicking, side-effect explicit | PASS: typed errors and adapter boundaries required | PASS: command/diagnostic shape keeps secrets out; rollback explicit |
| Domain is independent from infrastructure | PASS: domain/ports/adapters separation | PASS: `ip`/`iptables`/SSH/SQLite stay behind ports |
| SQLite is local source of truth | PASS: routed mapping persisted beside inventory | PASS: `0003` migration plus repository transactions cover mapping and ownership |
| Firecracker/firectl remain replaceable details | PASS: runtime port reused | PASS: no command/process type leaks publicly |
| Registry and artifact boundaries explicit | PASS: exact local readiness stays precondition | PASS: no download or selection-policy change |
| Multiple MicroVMs supported | PASS: per-VM TAP/MAC/addresses/keys | PASS: uniqueness plus per-VM rule ownership; no shared bridge lifetime |
| Public contracts and compatibility documented | PASS: contract/data-model/quickstart planned | PASS: Rustdoc, migration notes, tests, quickstart included |
| Project text is English | PASS | PASS |

No constitution violation or complexity exception is required.

## Phase 0 Research Summary

Recorded in [research.md](./research.md): Wi-Fi bridge impossibility, routed host
order of operations, ARP-then-fallback selection, post-boot SSH guest end-state,
privilege-by-default diagnostics, additive persistence (`0003`), host-only
unification on `iptables`, legacy bridge rows ignored.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): mapping, offer, forwarding state, guest
  end-state, migration shape, invariants.
- [contracts/sdk-routed-lan.md](./contracts/sdk-routed-lan.md): public types,
  operations, errors, rollback, usage.
- [quickstart.md](./quickstart.md): privileged validation, regression, reconcile,
  failure drills, gates.

### Public SDK boundary

Extend `MicroVmSdk` inputs/results without new operations:

- `CreateMicroVmRequest` gains `lan_address: Option<Ipv4Addr>` (explicit
  override; `None` selects automatically; `Some` with `expose_on_lan: false` is
  invalid).
- `NetworkConfiguration` gains `lan_address: Option<IpAddr>` (`Some` for LAN).
- `NetworkResource` gains `HostRoute`, `ProxyArpEntry`, `ForwardRule`, and
  `IptablesNat`; legacy bridge/DHCP kinds stay parseable for old rows only.
- Re-export deliberately from `crates/sdk/src/lib.rs` with Rustdoc. No new
  lifecycle or inventory operations.

### Preflight and idempotency

Coordinator order in `manager.rs`:

1. Validate request including the LAN override shape; default volume path below
   the explicit home; acquire the per-name lock; load any existing row.
2. Identical configured request (including override) returns the existing result
   after revalidation; mismatch conflicts without mutation.
3. Resolve exact image, default kernel, and independent Firecracker/`firectl`
   pair; check architecture, executable bits, KVM, and privilege-relevant host
   prerequisites before the provisional row.
4. Detect uplink and validate its global CIDR/gateway before host mutation.
5. Select the LAN offer (explicit, previous-valid, automatic duplicate-checked)
   and hold it under attempt ownership; insert the `creating` record.

### Volume and guest preparation

Unchanged: verified copy to `rootfs.ext4`, monotonic expansion, Ed25519 keys
with restrictive modes, offline public-key injection at
`/root/.ssh/authorized_keys`. The LAN boot argument is private-only
(`ip=<private>::<host>:255.255.255.252::eth0:off`).

### Artifact and registry integration

No change: exact local readiness stays precondition; no implicit downloads; dual
private-plus-LAN addressing is a network concern, not a registry concern.

### Network port and Linux adapter

Replace `create_lan`/reconcile/rollback internals behind the existing
`NetworkController` port:

- Uplink detection, LAN offer selection with `arping`-then-`ping` evidence,
  TAP/route/proxy setup, sysctls, and owned `iptables` rules via typed argv with
  captured stderr and privilege hints.
- Reconcile compares each desired routed item independently (skip/repair/conflict);
  keeps committed addresses; leaves the VM stopped.
- Rollback deletes in reverse dependency order with reference-checked shared
  sysctls; preserves foreign and pre-existing shared state.
- Host-only keeps its observable contract while its NAT moves to the unified
  `iptables` backend with identical semantics.

### Runtime and guest-setup adapter

Reuse the temporary-run pattern: boot private-only, wait bounded for private SSH
with the VM key, apply LAN address/routes/DNS over SSH, verify
private/LAN/egress, stop the exact process, remove the socket, and verify
stopped before committing `configured`. Secrets never enter errors or output.

### Persistence and transaction boundaries

Add `crates/sdk/migrations/0003_routed_lan.sql`, register version 3, extend
schema verification and repository load/persist for routed columns and resource
kinds. Transactions cover durable transitions without staying open across host
commands; final `configured` commits only after verification. Failures clean up
in reverse order and delete the provisional row.

### Error and rollback design

Typed variants carry LAN operation/resource identity, privilege hints with
command plus diagnostic output, duplicate/conflict owners, guest-setup
conditions, and aggregate cleanup failures. Attempt journal proves removal of
owned TAP/route/proxy/rules; shared state is preserved by reference checks.

### Test implementation

Extend SDK tests for: uplink detection failures; override validation; selection
order and duplicate handling; TAP/route/proxy/`iptables` skip/repair/conflict;
idempotent repeat and immutable conflict including the override; guest-setup
success/failure and verification; rollback provability; privilege-error shape;
host-only regression on the unified backend; multi-VM address isolation. Keep
host integration capability-gated; default `cargo test` uses injected ports.

## Implementation Sequence

Dependency order for `$speckit-tasks`:

1. Domain types plus request/config validation and idempotency-field coverage.
2. Typed error/diagnostic shape for routed operations and privilege failures.
3. Persistence migration plus repository load/persist for routed columns/kinds.
4. Linux adapter host setup (TAP/route/proxy/sysctl/`iptables`) with reconcile
   and rollback.
5. LAN selection (explicit/previous/automatic with duplicate evidence).
6. Temporary guest-setup flow (boot, SSH wait, apply, verify, stop).
7. Manager wiring (preflight order, journal, commit, idempotent return).
8. Test and doc completion (unit, reconcile, rollback, failure paths, Rustdoc,
   quickstart verification).

## Project Structure

### Documentation (this feature)

```text
specs/004-routed-lan-networking/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── sdk-routed-lan.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── error.rs
│   │   ├── manager.rs
│   │   ├── domain/
│   │   │   ├── mod.rs
│   │   │   ├── microvm.rs
│   │   │   ├── config.rs
│   │   │   ├── lifecycle.rs
│   │   │   └── artifact.rs
│   │   ├── ports/
│   │   │   ├── mod.rs
│   │   │   ├── repository.rs
│   │   │   ├── artifacts.rs
│   │   │   ├── network.rs
│   │   │   ├── runtime.rs
│   │   │   ├── storage.rs
│   │   │   └── credentials.rs
│   │   └── adapters/
│   │       ├── persistence/
│   │       │   ├── sqlite.rs
│   │       │   └── migrations.rs
│   │       └── registry/
│   │       └── runtime/
│   │       └── network/
│   │       └── storage/
│   │       └── credentials/
│   ├── migrations/
│   │   ├── 0001_artifact_inventory.sql
│   │   ├── 0002_microvm_creation.sql
│   │   └── 0003_routed_lan.sql
│   └── tests/
│       ├── public_api.rs
│       ├── lifecycle.rs
│       └── failure_paths.rs
└── cli/
    ├── Cargo.toml
    ├── src/
    └── tests/
```

**Structure Decision**: Two-crate SDK-first layout is unchanged. Routed LAN is an
SDK-internal network-adapter plus persistence plus manager-wiring change; no new
crate, CLI surface, or top-level layer is added.

## Complexity Tracking

No constitution violation or complexity exception is required.
