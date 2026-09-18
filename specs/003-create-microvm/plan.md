# Implementation Plan: Create and Initially Configure a MicroVM

**Branch**: `003-create-microvm` | **Date**: 2026-09-17 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-create-microvm/spec.md`

## Summary

Implement an SDK-only, single-awaited workflow that creates one MicroVM from a caller-selected,
already downloaded and verified registry image. The SDK will validate the exact distribution/image
selection and all independent runtime prerequisites, copy the selected `.ext4` image into a
per-VM volume directory, grow it when requested, inject a per-VM Ed25519 public key, configure
host-only or LAN networking, persist all host-local metadata in SQLite, and return a fully
configured but stopped VM with SSH connection metadata.

The public SDK surface will expose `create_microvm` and an independent `configure_network`
reconciliation operation. Firecracker, `firectl`, ext4 tools, Linux networking, credentials, and
SQLite are replaceable adapter concerns. No CLI lifecycle code or list/inspect/status/delete/start/
stop/reboot operation is part of this feature.

## Technical Context

**Language/Version**: Rust 2024 edition, using the repository's stable toolchain.

**Primary Dependencies**: Existing `rusqlite` with bundled SQLite, `tokio`, `serde`, `reqwest`,
`sha2`, and `thiserror`; add only the minimal `tokio` features needed for process/time/network
coordination and a Rust SSH-key serialization dependency with Ed25519 support. Host integration
uses typed `std::process::Command` arguments through an internal command port; no shell is used.

**Storage**: Existing verified artifact inventory in SQLite under the explicit SDK home, extended
with a migration for MicroVM, network, bridge, credential-path, and runtime metadata. VM-local
files live in a directory containing `rootfs.ext4`, SSH key files, the expected socket path, and
other exclusive runtime data.

**Testing**: Existing unit and integration tests for artifacts, SQLite, public API, persistence,
and failure paths, plus deterministic manager tests using injected artifact, storage, credential,
network, and runtime ports. Real KVM, ext4 mount, TAP/bridge, nftables, and DHCP integration is
capability-gated and is not part of the default test suite. Run the repository Cargo quality gates.

**Target Platform**: Linux hosts with KVM, readable/writable `/dev/kvm`, permission to manage TAP,
bridge, routes, forwarding, nftables, and ext4 images. Unsupported or insufficiently privileged
hosts return typed SDK errors.

**Project Type**: Reusable SDK library in `crates/sdk`; the CLI is not modified for this feature.

**Performance Goals**: Perform all artifact, host, and resource validation before host mutation;
avoid duplicate copy/key/network work on identical creation; make repeated network reconciliation
skip already-correct resources; bound any temporary LAN/SSH observation with a 30-second internal
deadline. No background worker or unbounded wait is introduced.

**Constraints**: The SDK is silent and returns typed `Result` errors. It must not read home paths
from environment variables, download artifacts implicitly, mutate caller-owned files, shrink an
image, expose private-key contents, overwrite foreign network resources, or leave a Firecracker
process or active socket after success. Multiple VMs on one host are first-class.

**Scale/Scope**: Multiple independent VMs per host, with tests covering at least 10 VMs and
concurrent same-name/address requests. This feature owns initial creation/configuration and
network reconciliation only; later lifecycle and inventory presentation operations remain future
work.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle behavior; CLI remains thin | PASS: design is SDK-only | PASS: no CLI source changes or duplicate orchestration |
| Public SDK is typed, silent, non-panicking, and side-effect explicit | PASS: typed errors and adapter boundaries required | PASS: no output/logging/global state; failures and cleanup are explicit |
| Domain is independent from infrastructure | PASS: domain/ports/adapters separation selected | PASS: Firecracker, commands, SQLite, ext4, and network stay behind ports |
| SQLite is local source of truth | PASS: VM metadata is persisted beside artifact inventory | PASS: migration and repository transactions cover identity, resources, and state |
| Firecracker/firectl remain replaceable implementation details | PASS: runtime port selected | PASS: public API exposes no command or process type |
| Registry and artifact boundaries are explicit | PASS: exact local readiness is a precondition | PASS: independent Firecracker/firectl records and image/kernel resolution are persisted |
| Multiple MicroVMs are supported | PASS: per-VM identity, locks, paths, addresses, and credentials | PASS: shared LAN bridge has separate ownership/refcount and VM attachments are isolated |
| Public contracts and compatibility changes are documented | PASS: contract/data-model artifacts planned | PASS: Rustdoc, migration notes, tests, and quickstart are included |
| Project text is English | PASS | PASS |

No constitution violation or complexity exception is required.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Use the exact caller-selected image and the distribution's default kernel from verified local
   inventory; creation never downloads or silently selects a different image.
2. Require image format/filesystem `ext4`, a registry-declared minimum size, and a requested disk
   size at least as large as both that minimum and the verified source file.
3. Copy the source into `{volume_path}/rootfs.ext4`, expand the copy and ext4 filesystem when
   needed, and pass that copy to `firectl` as a writable root drive.
4. Resolve Firecracker and `firectl` as independent verified executable artifacts. A package may
   provide both components or the components may come from separate packages; select valid
   semantic-version packages for the host architecture independently for each component, choosing
   the highest valid version deterministically. The package versions may differ.
5. Generate one Ed25519 key pair per VM, store paths under the VM directory, and inject only the
   public key at `/root/.ssh/authorized_keys` in the VM-local rootfs copy. Return/persist the
   private-key path only.
6. Use host-only `/30` + TAP + static guest boot parameters + nftables NAT by default. Use a
   shared SDK-managed bridge on the detected default-route uplink + guest DHCP when LAN exposure
   is explicitly enabled; never fall back between modes.
7. Temporarily start only when needed, principally to observe a LAN DHCP lease. Stop the exact
   process, remove the active socket, and commit only the stopped `configured` state.
8. Reconcile network state from persisted desired data, reporting correct resources as skipped and
   repairing only missing or SDK-owned stale resources.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): domain records, lifecycle, ownership, and migration shape.
- [contracts/sdk-create.md](./contracts/sdk-create.md): public Rust types, operations, errors,
  rollback, and usage contract.
- [quickstart.md](./quickstart.md): SDK usage, host prerequisites, failure behavior, and gates.

### Public SDK boundary

Extend the `MicroVmSdk` facade with:

- `create_microvm(CreateMicroVmRequest) -> Result<MicroVmCreationResult, SdkError>`;
- `configure_network(&str) -> Result<NetworkConfigurationResult, SdkError>`.

Re-export the request, result, state, network, and SSH metadata types deliberately from
`crates/sdk/src/lib.rs`. Keep the network operation's desired mode read-only from the caller's
perspective: it reads the persisted configuration for the named VM and repairs that configuration
after host drift. Do not add public lifecycle or inventory operations in this feature.

The result exposes the stable VM name, configured/stopped state, selected IDs, requested resource
capacities, VM-volume directory, copied rootfs, expected socket path, network metadata, and
`root:22` SSH metadata including the private-key path. It never exposes a private-key value or
active process handle.

### Preflight and idempotency

Implement a creation coordinator in `manager.rs` or a focused private manager module with this
order:

1. Validate the request, path rules, integer conversions, and explicit home-derived default path.
2. Acquire a per-name target lock and read any existing VM row.
3. If a configured VM exists, compare immutable request fields. For an identical request,
   revalidate its persisted files and network and return the existing result; for a mismatch,
   return a typed conflict without mutation.
4. Resolve the exact distribution and image IDs, ensure image ownership, ext4 format, registry
   minimum, and verified physical file integrity.
5. Resolve the default kernel and independently verified Firecracker/firectl pair. Check host
   architecture, executable bits, KVM, and host resource/permission prerequisites before creating
   a provisional row.
6. Validate volume ownership: create a missing directory, accept an empty caller-selected
   directory, reuse only a matching SDK-owned directory, and reject non-empty foreign data.
7. Insert a `creating` record and use an attempt ownership marker for every subsequent side effect.

The coordinator must never use in-memory state as the source of truth. A fresh SDK instance must
be able to recover every persisted path, selected artifact, desired network value, and configured
state from SQLite.

### Volume and guest preparation

Add a storage port with operations for verified copy, monotonic expansion, ext4 verification,
offline mount/unmount, and cleanup. The implementation:

- copies bytes to an attempt-local temporary file inside the selected VM directory and atomically
  renames it to `rootfs.ext4`;
- expands the file and runs the offline ext4 resize only when the requested size exceeds the
  source; it never shrinks and verifies the final size;
- creates `ssh/id_ed25519` and `ssh/id_ed25519.pub` with restrictive modes using a Rust Ed25519
  key adapter;
- mounts the VM-local copy privately, appends the generated public key once to
  `/root/.ssh/authorized_keys`, preserves existing keys, verifies guest path/modes, unmounts,
  and removes the temporary mountpoint;
- records only key paths, fixed guest path, user/port, and fingerprint in SQLite.

The adapter must refuse symlink/path escapes, preserve the verified source image, and distinguish
attempt-created paths from caller-owned paths for rollback.

### Artifact and registry integration

Read optional `minimum_size_bytes` from raw registry metadata and persist it in the artifact
inventory so missing metadata is distinguishable from zero. Keep the existing public registry image
model compatible with the SDK/CLI construction contract; update registry validation, fixtures, the
distribution-image persistence query, and migration schema without changing download behavior.

Extend artifact resolution with a method that returns a complete creation prerequisite set:

- the selected image and its verified local path;
- the distribution's default kernel and verified local path;
- one installed `firecracker` component and one installed `firectl` component from the current
  artifact inventory. A single package may provide both components; otherwise separate packages
  are selected independently using valid semantic versions, host architecture, required component
  coverage, and highest-version deterministic ordering. The package versions may differ.

If the inventory is incomplete, stale, ambiguous, or incompatible, map the result to a typed
preflight error before host mutation. Do not adopt an untracked file merely because it exists.

### Network port and Linux adapter

Define a replaceable network port that can inspect, create, compare, and remove owned resources.
The Linux adapter uses typed argument vectors for `ip` and `nft`, captures command output, and
never constructs shell strings.

Host-only reconciliation:

- deterministically scan the SDK private IPv4 pool for an unoccupied `/30`, considering persisted
  VM networks and live host routes;
- derive a VM-specific TAP name and locally administered MAC within Linux naming limits;
- create/verify TAP, host endpoint, forwarding, per-VM NAT, and the Firecracker interface;
- render the static guest `ip=` boot argument with guest address, host gateway, netmask, and
  interface name;
- persist resource fingerprints and ownership, and return applied/skipped entries.

LAN reconciliation:

- detect the interface used by the host's default route and validate that the SDK can manage it;
- create or reuse one SDK-managed bridge per uplink, preserving host addresses/routes and tracking
  shared ownership/refcounts;
- create/verify a VM-specific TAP, attach it to the managed bridge, and render `ip=dhcp`;
- temporarily start the VM through the runtime port when a DHCP observation is required, find the
  lease by the VM MAC on the bridge, validate conflicts, persist the lease/address, then stop and
  remove the active socket;
- report unsupported uplink management, DHCP failure, permission failure, or foreign resource
  ownership as typed errors, with no host-only fallback.

The `configure_network` path compares each desired resource independently. It skips matching
resources, repairs missing or SDK-owned stale resources, and refuses to delete or overwrite foreign
resources. Shared bridges are retained while another VM references them.

### Runtime adapter

Implement `ports::runtime` and `adapters::runtime::firecracker` around a private process handle.
The adapter:

- builds `firectl` argv from validated paths and values: Firecracker binary, kernel, copied
  rootfs with writable mode, vCPU count, checked effective memory MiB, distribution boot args,
  network TAP/MAC, and `{volume_path}/firecracker.sock`;
- starts only the exact selected executable, captures stdout/stderr rather than forwarding it,
  and records temporary process metadata;
- exposes bounded readiness/lease observation and an explicit stop/wait/remove-socket operation;
- verifies that no active process or socket remains before the manager commits `configured`;
- maps missing binaries, exit status, timeout, KVM, and socket failures to typed SDK errors.

The runtime port does not expose raw commands through the public API and does not add start/stop
operations beyond the internal temporary setup needed by this feature.

### Persistence and transaction boundaries

Add `crates/sdk/migrations/0002_microvm_creation.sql`, register it in
`adapters/persistence/migrations.rs`, and extend required-schema verification. Add repository
methods for:

- lookup by VM name and immutable creation configuration;
- insert/update/delete provisional VM records;
- persist network/bridge/resource ownership and reconciliation observations;
- persist credential paths/fingerprint and runtime metadata;
- load a complete configured result after SDK recreation.

SQLite transactions cover each durable state transition, but never hold a transaction open while
running external host commands. The manager writes `creating`, performs owned side effects, stops
the temporary runtime, and then commits the final `configured` row and resource observations. On
failure it cleans up in reverse dependency order and deletes the provisional row. The repository
must not report a configured VM until the final commit succeeds.

### Error and rollback design

Extend `SdkError` with typed categories for invalid VM requests, artifact readiness, disk-size
preconditions, runtime compatibility, storage ownership, VM state conflicts, network/permission,
guest filesystem/credential, temporary startup, and cleanup failures. Every variant carries only
safe diagnostic data; no private-key contents or unrestricted command output may appear.

Use an attempt cleanup journal that records ownership as each side effect succeeds. Roll back in
this order: stop process, remove active socket, detach/remove VM network resources, remove an
unused attempt-created bridge, unmount guest filesystem, remove generated keys/rootfs, remove the
attempt-created volume directory when empty, and delete the provisional row. Cleanup is idempotent
and never acts on a resource without matching ownership. If the primary and cleanup operations both
fail, return a typed aggregate cleanup error.

### Test implementation

Add or update SDK tests for:

- valid host-only and LAN creation, explicit image selection, exact artifact pair resolution, and
  configured/stopped postconditions;
- disk minimum/current-size validation, ext4 expansion, source immutability, resource conversion,
  invalid names/paths, and volume ownership;
- missing/stale/corrupt image/kernel/runtime prerequisites and no host mutation on preflight error;
- public-key-only guest injection, fixed root path, Ed25519 path/mode persistence, and absence of
  private-key content in results/errors/output;
- `/30` allocation, NAT/bridge ownership, LAN no-fallback behavior, address conflict, bridge
  sharing, and at least 10 independent VMs;
- identical creation idempotency, immutable conflict, concurrent same-name serialization, SDK
  recreation, and network reconciliation with applied/skipped/repair behavior;
- runtime/network/guest/SQLite failure rollback, stopped runtime, removed socket, preservation of
  caller-owned data, and absence of unsolicited stdout/stderr or panic.

Use injected ports to make manager success, conflict, reconciliation, and rollback behavior
deterministic. Keep host integration capability-gated so ordinary `cargo test` remains useful on
hosts without KVM or network administration privileges.

## Implementation Sequence

The dependency-ordered implementation sequence for `$speckit-tasks` is:

1. Extend registry/image metadata and add focused validation tests for minimum disk size and ext4.
2. Add domain request, result, state, network, credential, and runtime types with Rustdoc.
3. Add typed errors and path/resource validation helpers.
4. Add the SQLite migration, migration registration/schema verification, and repository methods.
5. Add artifact prerequisite resolution and independent Firecracker/firectl component-selection
   and compatibility checks.
6. Add storage/key/guest-filesystem ports and Linux adapters for copy, resize, mount, and key
   injection.
7. Add network ports and Linux host-only/LAN implementations with ownership-aware reconciliation.
8. Add runtime ports and the Firecracker/firectl adapter with bounded temporary start/stop.
9. Implement the manager's create preflight, side-effect journal, rollback, final commit, and
   idempotent existing-record path.
10. Implement `configure_network`, public re-exports, and result/error Rustdoc.
11. Add public contract, persistence, lifecycle, failure, concurrency, multi-VM, and no-output
   tests; update fixtures and quickstart.
12. Run the required quality gates and review the diff for SDK/CLI boundary, secret handling,
   cleanup, and English-only repository text.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Moving a physical uplink into a bridge can disrupt host networking | Snapshot addresses/routes, support only a validated manageable uplink path, make the operation typed/failable, and reverse every attempt-owned change on failure. |
| A LAN DHCP lease cannot be known while the VM is stopped | Temporarily start only for MAC-correlated lease observation, bound the wait, then stop and remove the socket before success. |
| ext4 helper or mount failure leaves a partial rootfs | Use an attempt-local copy, offline operations, explicit unmount/final verification, and reverse-order cleanup. |
| Firecracker/firectl API mismatch | Reuse the component-aware selection policy from the current artifact workflow, allow independently versioned split packages, validate the temporary runtime, and return a typed incompatibility error on startup failure. |
| Host reboot removes TAP/bridge/firewall state | Persist desired resource fingerprints and make `configure_network` compare, skip, and repair each item. |
| Multiple VMs collide on names, paths, addresses, or shared bridge | Use SQLite uniqueness, per-name locks, deterministic address/MAC allocation, resource ownership, and bridge refcounts. |
| Secrets leak through errors or diagnostics | Keep key contents out of domain/repository types, capture subprocess output, return paths/fingerprints only, and test output silence. |

## Post-Design Constitution Check

All gates remain PASS after Phase 1:

- The feature is implemented once in the SDK and exposed through typed public operations; the CLI
  remains a consumer and is untouched.
- Domain records and orchestration do not depend on SQLite, Linux commands, Firecracker, firectl,
  or terminal output; each dependency is behind a port/adapter.
- SQLite remains the durable host-local source of truth, while registry access and physical
  artifact readiness remain explicit boundaries.
- The design supports independent VMs, shared-but-owned LAN infrastructure, idempotent network
  repair, and safe rollback without global state or destructive cleanup.
- Public contracts, Rustdoc, migrations, tests, and quickstart documentation are planned in
  English and preserve the existing project organization.

## Complexity Tracking

No constitution violations or additional project layers require justification.
