# Feature Specification: Create and Initially Configure a MicroVM

**Feature Branch**: `003-create-microvm`

**Created**: 2026-09-17

**Status**: Draft

**Input**: User description: "Add an SDK-only workflow that creates and initially configures a MicroVM from a pre-downloaded registry distribution image selected by the caller, with host-only or LAN-reachable networking, a dedicated writable root volume, and SSH access prepared for a future start."

## Clarifications

### Session 2026-09-17

- Q: Should the creation volume path represent the writable root disk or a separate data disk? → A: It is the per-VM volume directory; inside it the SDK stores a `.ext4` copy of the downloaded distribution image as the root disk passed to `firectl`, together with the VM's exclusive keys, socket, and other files.
- Q: When a distribution has more than one compatible image available, which image should the SDK use for boot? → A: The caller must provide the specific image ID to the SDK.
- Q: How should the SDK determine the minimum allowed disk size for the selected .ext4 image? → A: Use the registry-reported `size_bytes`, which is the original image file size.
- Q: What should happen when the requested disk size is below the image's registry-reported `size_bytes`? → A: Reject the request.
- Q: Which units and format should the public SDK use for disk size and RAM? → A: Disk and RAM use integer bytes; vCPU uses an integer count.
- Q: When LAN exposure is enabled, how should the SDK attach the VM to the local network? → A: The SDK manages a bridge connected to the host's detected uplink; the VM receives a LAN address through DHCP, address conflicts or unavailable DHCP are errors, and the SDK never falls back to host-only mode.
- Q: When LAN exposure is disabled, how should the SDK provide host-only networking? → A: The SDK allocates an exclusive private /30 per VM, creates a TAP interface, makes the guest reachable from the host, and provides outbound connectivity through host NAT.
- Q: How should SSH credentials be created and tracked? → A: The SDK generates one Ed25519 key pair per VM, installs only the public key in the guest, keeps the private key on the host, and persists the private-key path as the VM's credential reference without storing or exposing the private-key contents.
- Q: Which SSH user and port must the SDK use for readiness and returned connection metadata? → A: Use the root user on the default SSH port 22; the selected image already provides the SSH server and access configuration, so the SDK only injects the generated public key into the expected location.
- Q: Where should the Firecracker control socket and other VM-exclusive files live? → A: They must live inside the VM volume directory; the socket path is `{vm_volume_path}/firecracker.sock`, alongside the copied `.ext4` image, SSH keys, and other files exclusive to that VM.
- Q: What guest path must receive the generated public SSH key? → A: Every image must use `/root/.ssh/authorized_keys`; the SDK injects the key only into the per-VM `.ext4` copy and does not require a registry or caller-provided path.
- Q: Should Firecracker and `firectl` be acquired and tracked as one package or as separate artifacts? → A: Track them as independent artifacts and resolve a verified compatible pair during creation; neither binary requires the other to be distributed in the same package.
- Q: How should independently tracked Firecracker and `firectl` packages be selected when their versions differ? → A: Follow the existing artifact-selection behavior: a package may provide both components or the components may come from separate packages; select host-architecture-compatible packages with valid semantic versions independently for each required component, choosing the highest valid version deterministically. The package versions do not need to be equal.
- Q: How should the SDK configure the guest's IP address and gateway? → A: Use standardized boot parameters: static IP, gateway, and route for host-only mode, and DHCP for LAN mode; the SDK does not run a per-VM DHCP server.
- Q: What should remain when creation fails after creating VM resources? → A: Roll back all resources created by the attempt, including process, network, socket, SSH keys, and copied `rootfs.ext4`; preserve the source image and pre-existing caller-owned data.
- Q: In which public operations should the SDK return the persisted private-key path? → A: Return it in the creation result and persist it for future SDK operations; this feature is limited to initial creation/configuration, so listing, inspection/status, deletion, start, stop, and reboot operations are out of scope. The SDK must also expose an independently callable network-configuration operation that reconciles existing host state, skips already-correct items, repairs only missing or stale items, and leaves the VM stopped before returning.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Create and Configure a Host-Only MicroVM (Priority: P1)

As an application developer, I want to create and configure a MicroVM from a distribution already
acquired by the SDK so that the returned machine is stopped, isolated from the LAN by default, and
ready for a future start with SSH access prepared.

**Why this priority**: This is the primary value of the feature: turning a valid local set of
artifacts into one fully configured host-local machine without requiring callers to orchestrate
runtime preparation, storage, networking, or credentials themselves.

**Independent Test**: Prepare a valid distribution, a caller-selected image, its required kernel,
and compatible Firecracker and `firectl` artifacts in the local inventory, request creation with
LAN exposure disabled, and verify that one independent VM reaches the configured and stopped state
with a persisted private network address, writable root volume, injected public key, and no running
Firecracker process left behind.

**Acceptance Scenarios**:

1. **Given** a unique VM name, a valid distribution registry ID, a caller-selected image registry
   ID, a completely downloaded and verified image, its required kernel, and compatible runtime
   binaries,
   **When** the caller requests creation without LAN exposure, **Then** the SDK creates and fully
   configures one VM,
   assigns it a unique host-only /30 address through a VM-specific TAP interface with host NAT for
   outbound connectivity, creates its dedicated writable root volume by copying
   the downloaded distribution image into a VM-specific `.ext4` file, injects the SSH public key,
   and prepares the runtime with that file as its `firectl` root drive. The SDK MAY start the VM
   temporarily for configuration or a health check, but MUST stop it before returning a configured
   result containing its identity, address, volume, and SSH connection metadata; no Firecracker
   process or control socket remains active after return.
2. **Given** the caller omits a VM-volume directory path, **When** creation succeeds, **Then** the
   SDK creates the VM-specific directory at the managed default path, places its copied `.ext4`
   root disk, SSH key files, `firecracker.sock` path, and other VM-exclusive files inside it, and
   persists the directory and child paths so the VM does not share data with another VM.
3. **Given** the caller supplies a valid VM-volume directory path, **When** creation succeeds,
   **Then** the SDK uses that directory for the copied `.ext4` root disk, SSH key files,
   `firecracker.sock`, and other VM-exclusive files, records the association, and does not
   overwrite or reinitialize a directory that belongs to another VM or contains caller-owned data.
4. **Given** a VM with the same identifier and identical creation configuration is already
   configured, **When** the caller repeats the creation request, **Then** the SDK returns the
   existing VM metadata without creating duplicate files, network resources, credentials, or
   runtime configuration.
5. **Given** a VM with the same identifier already exists with a different distribution,
   network mode, volume, or other immutable creation setting, **When** the caller requests
   creation, **Then** the SDK returns a typed conflict and leaves the existing VM unchanged.

### User Story 2 - Reject Missing Prerequisites Safely (Priority: P1)

As an application developer, I want creation to validate every prerequisite before changing the
host so that a missing or stale artifact produces an actionable error instead of a partial or
unusable VM.

**Why this priority**: Creation depends on several independently managed artifacts. Failing
early protects the host and makes the required preparation step explicit.

**Independent Test**: Run creation repeatedly with one prerequisite missing from the inventory,
missing from disk, corrupted, or incompatible, and verify that each attempt returns the matching
   typed error without starting a process, changing network configuration, or publishing a
   configured VM.

**Acceptance Scenarios**:

1. **Given** the distribution ID is unknown, absent from the local inventory, incomplete, or
   its file is missing or no longer matches its verified metadata, **When** creation is
   requested, **Then** the SDK returns an artifact-not-ready error that identifies the
   distribution and instructs the caller to download it before retrying.
2. **Given** the distribution's required kernel is absent, stale, unavailable for the host, or
   not fully verified, **When** creation is requested, **Then** the SDK returns an actionable
   kernel precondition error and performs no runtime, network, volume, or SSH setup.
3. **Given** either required runtime binary is absent, stale, non-executable, or incompatible
   with the host, **When** creation is requested, **Then** the SDK returns an actionable runtime
   prerequisite error that identifies the missing component and does not start the VM.
4. **Given** any preflight check fails, **When** the caller retries after acquiring the required
   artifact, **Then** the SDK can perform a fresh preflight without relying on stale in-memory
   state from the failed attempt.

### User Story 3 - Configure a LAN-Reachable MicroVM (Priority: P1)

As an application developer, I want to opt a VM into LAN exposure so that authorized devices on
the host's local network can reach it after a future start using a persisted address, while the
default remains host-only access.

**Why this priority**: LAN exposure is a distinct operational mode with host-network changes
and different safety requirements. It must be explicit, predictable, and re-applicable after a
host restart for every VM.

**Independent Test**: Prepare the same valid artifacts as the primary flow, request creation
with LAN exposure enabled on a test host that has a usable LAN connection, and verify that the
VM receives a non-conflicting LAN-reachable address, the host configuration is applied, the
configuration is persisted, and no Firecracker process remains running after creation.

**Acceptance Scenarios**:

1. **Given** LAN exposure is explicitly enabled and the host has a usable LAN network,
   **When** creation succeeds, **Then** the SDK detects the host uplink, connects the VM through
   an SDK-managed bridge, obtains a non-conflicting LAN address through DHCP, persists the bridge
   and address details, and returns that address with the configured and stopped result.
2. **Given** LAN exposure is disabled or omitted, **When** creation succeeds, **Then** the SDK
   uses the host-only network mode and does not silently expose the VM on the LAN.
3. **Given** the requested LAN mode cannot be configured because the host network is unavailable,
   the address cannot be allocated safely, or the caller lacks the required permission, **When**
   creation is requested, **Then** the SDK returns a typed network error, restores any changes
   made by the attempt, and does not fall back to host-only mode.
4. **Given** another managed VM already owns a candidate address, **When** a second VM is
   created, **Then** the SDK selects another safe address or returns an address-allocation error
   without changing the first VM.

### User Story 4 - Reconcile Network Configuration for an Existing VM (Priority: P1)

As an application developer, I want to reapply network configuration for an existing VM after a
host restart so that correct resources are skipped, missing resources are repaired, and the VM is
ready for a future start without recreating its identity or credentials.

**Why this priority**: Host network state may disappear or become stale after a reboot, while VM
identity and files remain durable. Reconciliation prevents duplicate interfaces, addresses, and
rules while keeping network setup available as a focused SDK operation.

**Independent Test**: Create multiple configured VMs using different identifiers and network modes,
recreate the SDK with the same host home after simulating a host restart, call the public network
configuration operation for each VM, and verify that already-correct resources are skipped, missing
resources are repaired, and every VM remains stopped and independently addressable.

**Acceptance Scenarios**:

1. **Given** a configured VM and a host restart that removed or invalidated host-side network
   resources, **When** the caller invokes the SDK's network configuration operation, **Then** the
   SDK reads the persisted desired mapping, recreates only missing or stale resources, and leaves
   the VM stopped.
2. **Given** the VM's persisted network resources are already correct, **When** network
   configuration is invoked again, **Then** the SDK detects and skips each correct item without
   allocating a second address, TAP, bridge attachment, route, or NAT rule.
3. **Given** one VM is reconciled, **When** another VM is created or reconciled, **Then** the
   operation does not replace, reuse, or expose the first VM's files, network resources, keys, or
   socket path.

### Edge Cases

- The distribution ID is syntactically invalid, is not present in the registry metadata, or
  names a distribution with no deterministic bootable image; creation returns a typed
  validation or incompatibility error.
- A distribution has multiple images or supported kernels. The caller must provide the specific
  image ID, and the SDK must verify that it belongs to the distribution and is compatible with
  the host; the distribution's designated default kernel remains selected by the SDK. Unknown,
  unverified, or incompatible image IDs return a typed error instead of being replaced by an
  arbitrary image.
- A registry record exists but the corresponding file is absent, replaced, truncated, or has a
  different size or digest. The SDK treats the artifact as not ready and does not adopt it during
  creation.
- The local inventory contains a downloaded artifact record but the host architecture or runtime
  capabilities cannot use it. The SDK returns an incompatible-artifact error before mutation.
- The selected image or kernel does not support the standardized boot-time network parameters
  required for its requested mode. Creation returns a typed network incompatibility error before
  the VM is reported as configured.
- If the SDK temporarily starts a VM for configuration or a health check and the runtime exits,
  the guest does not reach the check condition, or SSH is not reachable within the bounded check
  period, the SDK stops the attempt, restores attempt-owned host configuration, and returns a
  typed startup or health-check error without leaving a process running.
- Key generation, public-key injection, or secure private-key storage
  fails. The VM is not reported as configured or running, and the private key is not exposed through
  logs, output, or an incomplete result.
- The default VM-volume directory cannot be created, a caller-selected VM-volume directory path
  is inaccessible, or a directory is already owned by another VM. Creation returns a typed
  storage conflict or filesystem error and preserves caller-owned data.
- A LAN address is already in use, the host has no usable default network, or host permissions
  are insufficient. No partial LAN exposure remains after the failed attempt.
- Two callers request the same VM identifier or the same address concurrently. The SDK
  serializes the conflicting operation and returns one successful result plus a typed conflict or
  idempotent result for the other caller.
- A database write or inventory migration fails after host resources have been prepared. The SDK
  tears down resources created by the attempt and must not leave a record claiming that the VM is
  configured or running.
- The SDK home is invalid, unavailable, or changed between operations. The SDK returns a typed
  path error and never discovers a replacement home from environment variables.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The SDK MUST expose one cohesive public creation operation that accepts a unique,
  caller-selected path-friendly VM name, a distribution registry ID, a specific image registry
  ID, disk size in bytes, vCPU count, RAM size in bytes, an explicit LAN-exposure choice, and an optional
  VM-volume directory path. The VM name is the stable identifier used for persistence, future
  deletion, and its managed directory. The feature MUST modify only SDK behavior; no CLI
  lifecycle implementation or separate orchestration path is part of this feature.
- **FR-002**: The SDK MUST receive its host-local managed home explicitly from the caller and MUST
  use that home for the VM inventory, runtime data, generated credentials, default VM-volume directories, and
  other managed files. It MUST NOT read or override the home using `HOME`, `TAUMARU_HOME`, or
  another environment variable.
- **FR-003**: The creation operation MUST use the supplied distribution ID and specific image ID
  to resolve the registry metadata and corresponding local inventory records. The selected image
  MUST belong to the requested distribution and satisfy the host's architecture and filesystem
  constraints. Callers MUST NOT need to parse registry data, assemble runtime commands, or locate
  arbitrary host files themselves.
- **FR-004**: Before making host or runtime changes, the SDK MUST verify that the caller-selected
  image has a complete local inventory relationship and is present on disk with the recorded
  size and digest.
- **FR-005**: Before making host or runtime changes, the SDK MUST resolve and verify the
  distribution's designated default kernel, including presence, integrity, host compatibility,
  and any distribution compatibility relationship required for boot.
- **FR-006**: Before making host or runtime changes, the SDK MUST resolve one compatible installed
  Firecracker artifact and one compatible installed `firectl` artifact independently. Both
  artifacts MUST be present in the local inventory, present at their recorded paths, verified
  against their recorded integrity metadata, and executable when required. The selected pair MUST
  satisfy the host, architecture, and compatibility constraints even when the artifacts were
  acquired separately. One package MAY provide both components; otherwise the SDK MUST select
  separate packages independently. The package versions MAY differ, and selection MUST use valid
  semantic versions, host architecture, required component coverage, and a deterministic highest
  version per component rather than requiring equal versions.
- **FR-007**: If a distribution, kernel, runtime binary, or required supporting record is missing,
  stale, corrupted, or incompatible, the SDK MUST return a typed error that distinguishes the
  failing precondition, identifies the affected artifact, and tells the caller to acquire or
  repair it before retrying. Creation MUST NOT download artifacts automatically.
- **FR-008**: The SDK MUST treat an artifact as ready for creation only when both its local
  inventory relationship and its physical file are present and verified. A file that exists on
  disk without the expected inventory record MUST NOT be silently adopted by the creation
  operation.
- **FR-009**: The SDK MUST preserve explicit lifecycle transitions for a VM creation attempt. A
  VM MUST move from absent or retryable failed state through creation to `configured` only after
  volume preparation, network setup, public-key injection, and required configuration checks
  succeed. The final `configured` state MUST be stopped: no Firecracker process or active control
  socket may remain. A temporary `running` state is permitted only while the SDK performs an
  explicitly required configuration or health check and MUST transition back to `configured`
  before the operation returns.
- **FR-010**: VM identifiers MUST be unique within the managed home. Repeating an identical
  request for an already-configured VM MUST be idempotent; requesting a different immutable
  configuration for the same identifier MUST return a typed conflict without mutating the
  existing VM.
- **FR-011**: The SDK MUST create an isolated VM-volume directory and MUST persist the VM name,
  distribution ID, selected image and kernel references, disk size in bytes, vCPU count, RAM size
  in bytes, creation configuration, VM-volume directory, root-disk path, Firecracker socket path,
  SSH key paths, runtime paths, process metadata, lifecycle state, and timestamps needed to
  identify the VM and support future SDK operations.
- **FR-012**: When no VM-volume directory path is supplied, the SDK MUST use the exact default
  directory {taumaru_home}/vms/{vm_name} below the managed home. It MUST create that directory
  and a deterministic writable `rootfs.ext4` file inside it by copying the caller-selected
  distribution image. The directory MUST contain the VM-exclusive root disk, generated SSH key
  files, `firecracker.sock`, and other files owned exclusively by that VM. When a directory path
  is supplied, the SDK MUST use it as the VM-volume directory, create the same VM-exclusive files
  inside it, validate and persist the association, prevent ownership by more than one VM, and MUST
  NOT overwrite or reinitialize a directory that belongs to another VM or contains caller-owned
  data. A previously created SDK-owned directory and its files may be reused only when its VM
  ownership and source-image identity match the persisted record. The source image MUST remain
  unchanged. The registry-reported `size_bytes` is the original `.ext4` file size and is the
  minimum disk size. If the requested disk size is smaller, creation MUST fail before host
  mutation with a typed disk-size error. The requested disk size MUST also be at least the
  verified local image size; otherwise creation MUST return a typed disk-size error before
  copying. After the copy is created, if the requested size
  is greater than the copied image's current size, the SDK MUST grow `rootfs.ext4` and its
  filesystem to the requested size. The resulting `rootfs.ext4` file MUST be passed to firectl as
  the VM root drive.
- **FR-013**: The SDK MUST configure the VM with the caller-selected image, the distribution's
  boot settings and default kernel, the requested disk size in bytes, vCPU count, RAM size in bytes,
  writable `rootfs.ext4` root volume, and requested network mode through the internal MicroVM runtime
  boundary. It MUST provide the selected mode's standardized guest boot parameters through that
  boundary. Runtime and firectl mechanics MUST remain inside the SDK and its replaceable adapters.
- **FR-014**: The SDK MUST support a host-only network mode. This mode MUST be the default when
  LAN exposure is omitted or disabled. For each VM, it MUST allocate an exclusive non-overlapping
  private `/30`, create and configure a VM-specific TAP interface, assign the host and guest
  endpoints, make the guest address reachable from the host, and provide outbound connectivity
  through host NAT. It MUST persist the subnet, addresses, TAP identity, NAT ownership, and other
  host-side details required for later inspection and lifecycle operations. Exhaustion, overlap,
  or inability to configure any required private-network resource MUST return a typed error.
- **FR-015**: When LAN exposure is enabled, the SDK MUST detect a usable host uplink and configure
  an SDK-managed bridge attached to that uplink. The VM MUST attach to that bridge and obtain its
  LAN address through external LAN DHCP, requested through the standardized guest boot parameters.
  The SDK MUST validate that the resulting address does not conflict
  with the host, another managed VM, or an active LAN peer, and MUST persist the bridge, uplink,
  lease, and address details required to reuse that mapping safely. Missing uplink, unavailable
  DHCP, address conflict, or insufficient permission MUST produce a typed error.
- **FR-016**: The SDK MUST treat LAN exposure as explicit opt-in. If LAN configuration cannot be
  completed safely, it MUST return a typed network or permission error, undo changes made by the
  attempt, and MUST NOT silently fall back to host-only mode.
- **FR-017**: The SDK MUST generate a distinct Ed25519 SSH key pair for each VM and MUST store the
  key files inside the VM-volume directory, for example under `ssh/id_ed25519` and
  `ssh/id_ed25519.pub`. It MUST inject the public key into the per-VM `.ext4` copy before
  reporting the VM as configured. The private key MUST remain on the host with restrictive
  permissions. The SDK MUST
  persist the private-key path as the VM's credential reference and MUST expose only that safe path
  or reference rather than private-key contents; private-key material MUST NOT be stored in SQLite,
  returned in plaintext, or emitted through logs or errors.
- **FR-018**: The selected image MUST already provide a usable SSH server and the `root` guest
  account on the default port `22`. The SDK MUST inject the generated public key into the fixed
  guest path `/root/.ssh/authorized_keys` inside the per-VM `.ext4` copy, use `root:22` for any
  temporary authenticated health check, and return that account, port, address, and persisted
  private-key reference needed by a caller to establish a connection. If the fixed key path is
  unavailable, creation MUST return a typed incompatibility error. If the SDK performs a temporary
  authenticated health check and the preconfigured SSH service cannot be reached, it MUST return
  a typed startup or health-check error after stopping the temporary runtime.
- **FR-019**: The creation operation MUST NOT return success until the VM is in the `configured`
  state and stopped, its selected `rootfs.ext4` root volume exists and is usable at the requested
  disk size, its requested vCPU and RAM capacities are persisted, its network address and host
  configuration are persisted, and its public SSH key has been injected. If a temporary runtime
  start or health check was used, the SDK MUST stop it and remove the active control socket before
  returning success.
- **FR-020**: If any step after preflight fails, the SDK MUST stop any process started by the
  attempt, remove the attempt's socket, SSH keys, copied `rootfs.ext4`, VM-volume directory when
  it was created by the SDK, and host network resources, and remove any provisional inventory
  record for the attempt. Cleanup MUST be safe to retry and MUST preserve the source image,
  pre-existing caller-owned files and directories, and pre-existing host network configuration.
  The failed attempt MUST NOT remain published as a usable or running VM.
- **FR-021**: The SDK MUST persist enough local inventory to distinguish multiple VMs, including
  stable identity, distribution and artifact IDs, VM-volume directory, `rootfs.ext4` path,
  Firecracker socket path, SSH key paths and credential reference, network mode, guest address,
  host-side network details, runtime process metadata, and lifecycle state. Recreating the SDK
  with the same home MUST recover these records without relying on process-local memory.
- **FR-022**: The SDK MUST revalidate persisted process, file, volume, network, and credential
  references before returning a configured VM result or applying network configuration after SDK
  recreation. Stale references MUST produce typed integrity or configuration errors rather than
  being treated as usable resources.
- **FR-023**: Expected validation, inventory, filesystem, permission, registry, integrity,
  network, runtime-process, SSH, startup, concurrency, and cleanup failures MUST be returned as
  typed SDK errors. The SDK MUST NOT intentionally panic, terminate the host, write to stdout or
  stderr, emit logs or tracing events, or hide failures in global state.
- **FR-024**: The feature MUST include SDK tests for valid host-only creation, valid LAN creation,
  missing and stale prerequisites, distribution and kernel compatibility, binary resolution,
  volume ownership, SSH credential handling, address uniqueness, idempotent creation, conflicting
  creation, rollback after runtime/network/SSH/inventory failure, SDK recreation, idempotent
  network reconciliation with skip and repair behavior, multiple VMs, stopped post-creation state,
  and absence of unsolicited output or panic.
- **FR-025**: The SDK public facade MUST expose the initial VM creation operation and an
  independently callable network-configuration operation for an existing VM. The network
  operation MUST read the persisted desired configuration, inspect current host resources, skip
  each item that is already correct, repair only missing or stale attempt-owned items, persist the
  resulting state, and return a typed result describing applied and skipped items. It MUST be
  idempotent, MUST preserve correct resources, and MUST leave the VM stopped before returning.
  It MAY start the VM temporarily only when required to validate guest-side network configuration,
  and MUST stop it again before returning.
  Listing, inspection/status, deletion, start, stop, reboot, and other lifecycle operations are
  outside this feature.

### Key Entities *(include if feature involves data)*

- **MicroVM Record**: The durable identity and lifecycle record for one independently managed VM,
  including its creation configuration, artifact references, runtime metadata, and status.
- **Creation Request**: The caller's requested path-friendly VM name, distribution registry ID,
  specific image registry ID, disk size in bytes, vCPU count, RAM size in bytes, LAN exposure choice,
  optional VM-volume directory path, and any supported resource settings.
- **Distribution Selection**: Registry metadata and local inventory relationships identifying the
  bootable image, default kernel, boot settings, architecture, and capabilities used by the VM.
- **Runtime Prerequisite Set**: One verified compatible Firecracker artifact and one verified
  compatible `firectl` artifact, tracked independently and selected together for VM creation.
- **VM Volume Directory**: The per-VM directory at {taumaru_home}/vms/{vm_name} by default or
  at an explicitly supplied path. It contains all files exclusive to the VM, including the root
  disk, SSH key files, Firecracker socket, and other runtime files. Its path and child resource
  paths are persisted for later lifecycle operations.
- **VM Root Volume**: The VM-volume directory's `rootfs.ext4`, a writable copy of the
  caller-selected distribution image, resized to the requested disk size when larger than the
  copied image's current size and passed to firectl as the VM root drive. The registry-declared
  value is the policy minimum; the image's current size is an additional physical floor because
  the SDK never shrinks the image. If the registry does not declare its minimum, creation fails.
- **Firecracker Control Socket**: The per-VM control socket at
  `{vm_volume_path}/firecracker.sock`, owned by that VM and tracked in local inventory. It is
  cleaned up when the VM is definitively stopped or creation rolls back, while its expected path
  remains available for future lifecycle and network-reconciliation operations.
- **Network Attachment**: The persisted host-only or LAN-exposed network mode, guest address,
  host-side configuration, private `/30` and TAP/NAT ownership when host-only, bridge and uplink
  references when LAN-exposed, DHCP lease details, and ownership information for one VM.
- **SSH Credential**: A per-VM Ed25519 key pair and the fixed guest access identity `root:22`; the
  public and private key files are stored inside the VM-volume directory, the public key is
  injected into the VM, and the private key remains protected with restrictive permissions. The
  persisted credential reference is the private-key path, never the private-key contents.
- **Lifecycle State**: The explicit state of the VM creation and runtime, including retryable
  failure, creating, temporarily running during setup or health checks, and `configured` as the
  successful stopped state returned by this feature.
- **Artifact Readiness**: The relationship between a registry ID, its local inventory record, and
  a physical file that has passed integrity and compatibility checks.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In 100% of valid host-only creation tests, one request produces exactly one
  independently identifiable VM in the configured and stopped state with the requested vCPU and
  RAM capacities, a persisted unique host-only address, a usable root volume at the requested
  size, an injected public key, and SSH connection metadata.
- **SC-002**: In 100% of valid LAN-mode creation tests on a prepared test network, the VM receives
  a non-conflicting LAN-reachable address, the future SSH configuration is complete, and no
  Firecracker process remains running after creation; no test silently falls back to host-only
  mode.
- **SC-003**: In 100% of missing, stale, corrupted, and incompatible prerequisite tests, the
  request returns a typed actionable error before starting a runtime or publishing a configured VM.
- **SC-004**: In 100% of failed post-preflight creation tests, no process started by the attempt
  remains running, no attempt-owned network exposure remains active, and no inventory record
  claims that the VM is configured or running.
- **SC-005**: In 100% of repeated identical creation tests, the SDK returns the existing VM
  without creating a duplicate process, volume association, SSH key pair, or network address.
- **SC-006**: In 100% of tests creating at least 10 VMs on one host, every VM has a distinct
  identity, root-volume association, SSH credential, and managed network address, with no
  cross-VM resource substitution.
- **SC-007**: In 100% of SDK recreation and network-reconciliation tests, every previously
  configured VM retains the correct persisted state and resource metadata, already-correct host
  resources are skipped, missing resources are repaired, and the VM remains stopped.
- **SC-008**: In 100% of credential-handling tests, the guest receives only the public key, the
  private key is stored with restrictive host permissions, and neither key is emitted through SDK
  output, logs, or error text.
- **SC-009**: An application can complete the primary workflow—select a registry distribution,
  verify local prerequisites, create and configure a stopped VM, and obtain its SSH connection
  metadata—using only public SDK operations, without implementing artifact validation, network
  orchestration, runtime command assembly, or local inventory management itself.

## Assumptions

- The caller resolves the host-local base directory and supplies it when constructing the SDK.
  The SDK owns everything below that directory, including inventory, artifact references, runtime
  data, credentials, default VM-volume directories under {taumaru_home}/vms/{vm_name}, locks,
  and temporary files.
- The create operation is synchronous for this feature. A successful return means the VM has
  reached the configured and stopped condition; the SDK may start it temporarily for required
  preparation or a health check, but queued creation, progress callbacks, cancellation, and
  background reconciliation are outside this feature.
- Artifact acquisition is a separate workflow. Creation never downloads a distribution, kernel,
  or runtime binary implicitly; callers must use the existing artifact workflow first.
- The caller supplies a specific image registry ID. The SDK validates that the image belongs to
  the requested distribution and is compatible with the host, while the distribution metadata
  supplies the default kernel. There is no implicit image selection.
- The selected image's registry-reported `size_bytes` is the original `.ext4` file size and the
  minimum allowed disk size.
- The requested disk size must be at least the selected image's registry-reported and verified
  current size. The SDK only expands the copied `.ext4` image; it never shrinks it.
- The existing local SQLite-backed inventory remains the source of truth for downloaded artifact
  readiness and host-local VM metadata. No distributed control-plane state, registry write, cloud
  scheduling, billing, or authentication behavior is added.
- The existing inventory can resolve one deterministic compatible pair from independently tracked
  Firecracker and `firectl` artifacts. A single package may provide both components, or separate
  packages may provide them with different versions. Selection uses valid semantic versions,
  host architecture, required component coverage, and the highest valid version per component. If
  either component is missing, unverified, or incompatible, or candidate selection cannot be made
  deterministic, creation returns a typed prerequisite or conflict error instead of choosing
  arbitrarily.
- The default network mode is host-only. LAN exposure means reachability from the host's local
  network; it does not promise an Internet-routable public address, port forwarding, firewall
  policy, or inbound access beyond what the host network permits.
- LAN exposure uses an SDK-managed bridge attached to the host's detected usable uplink. The guest
  obtains its LAN address through DHCP; the caller does not provide a LAN interface or fixed
  address in this initial creation workflow. If uplink discovery, bridge setup, DHCP, conflict
  validation, or required permissions fail, creation returns an error and does not fall back to
  host-only networking.
- Host-only and LAN address allocation require a Linux host with the relevant network capability
  and permissions. Unsupported environments return typed errors rather than receiving a reduced
  or simulated success.
- Host-only networking uses one exclusive private `/30` and one VM-specific TAP interface per VM.
  The host endpoint can reach the guest endpoint, and SDK-owned host NAT provides guest outbound
  connectivity. Private subnet exhaustion, overlap, TAP setup failure, or NAT permission failure
  aborts creation without leaving attempt-owned network state behind.
- Guest network configuration uses standardized boot parameters. Host-only VMs receive their
  allocated static guest IP, gateway, and route; LAN-exposed VMs request DHCP from the attached
  local network. The SDK does not provide a per-VM DHCP server, and images or kernels that cannot
  consume the required parameters are incompatible with creation.
- A caller-selected existing VM-volume directory is preserved and is never overwritten or
  reinitialized implicitly. A default VM-volume directory and its `rootfs.ext4`, keys, socket,
  or other files created for a failed attempt are removed as part of rollback when safe to do so;
  pre-existing caller-owned files and directories are preserved.
- The VM-volume directory is the home for all VM-exclusive files. The expected Firecracker socket
  is `{vm_volume_path}/firecracker.sock`; the generated SSH key files and any other per-VM
  runtime artifacts remain in that directory or its child directories, and their paths are
  persisted in the local inventory.
- The standard SSH security model is used: the public key is injected into the guest, while the
  private key stays on the host and is referenced by its persisted path. Private-key material is
  never stored in SQLite or returned in plaintext.
- The selected distribution image already contains a usable SSH server, the `root` account, and
  the SSH setup required for a ready VM. All images use `/root/.ssh/authorized_keys`; the SDK
  only injects its generated public key into that fixed path in the per-VM copy. Guest
  customization, arbitrary cloud-init behavior, password login, alternate guest users, and
  user-managed firewall rules are outside this feature.
- Creation requires explicit disk size in bytes, vCPU count, and RAM size in bytes. The SDK
  validates those values against the selected image, host capabilities, and distribution
  requirements; it does not silently choose resource defaults. Creation uses the supported
  .ext4 root-volume format. The public scope of this feature is initial creation and configuration
  only; later lifecycle operations are intentionally deferred.
  Choosing a different kernel, image variant, runtime package, or volume format is outside this
  initial workflow.
- VM listing, inspection/status, deletion, start, stop, reboot, cloning, snapshots, migration,
  automatic artifact downloads, artifact cleanup, public CLI commands, distributed orchestration,
  and changes to the registry contract are out of scope. The public SDK surface in this feature
  consists of initial creation and focused configuration operations such as network
  reconciliation.
