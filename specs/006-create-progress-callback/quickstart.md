# Quickstart: MicroVM Creation Progress Observer (realtime)

This feature is consumed through the Rust SDK. It adds an optional realtime progress
observer to the existing creation operation. It adds no CLI command and changes no
persistence, registry, or runtime behavior. Full creation prerequisites still apply —
see `specs/003-create-microvm/quickstart.md` for host setup and artifact preparation.

## Prerequisites

- A built workspace (`cargo check -p taumaru-microvm` passes from the repository root).
- A seeded SDK home with the distribution image, its default kernel, and verified
  Firecracker/`firectl` binaries (creation never downloads; see the 003 quickstart).
- Test execution uses the deterministic manager fakes for unit coverage and the fixture
  registry server for integration coverage; no KVM, TAP, or DHCP host work is needed.

## Observe a creation in real time

```rust
use taumaru_microvm::{CreateMicroVmRequest, CreationProgress, CreationStage, MicroVmSdk};

let sdk = MicroVmSdk::new("/var/lib/taumaru")?;
let mut events: Vec<CreationProgress> = Vec::new();
let created = sdk
    .create_microvm(
        CreateMicroVmRequest {
            name: "build_vm".to_owned(),
            distribution_id: "ubuntu-24.04".to_owned(),
            image_id: "ubuntu-24.04-minimal".to_owned(),
            disk_size_bytes: 16 * 1024 * 1024 * 1024,
            vcpu_count: 2,
            memory_bytes: 2 * 1024 * 1024 * 1024,
            expose_on_lan: false,
            lan_address: None,
            volume_path: None,
        },
        Some(|event: CreationProgress| {
            // A single progress bar needs only this field:
            // bar.set_position(event.overall_percent);
            events.push(event);
        }),
    )
    .await?;

assert_eq!(created.state, taumaru_microvm::MicroVmState::Configured);
// Starts + byte ticks + finishes, then one completed terminal at 100%:
assert_eq!(events.first().expect("stream").overall_percent, 0);
assert_eq!(events.last().expect("terminal").overall_percent, 100);
assert!(events.windows(2).all(|pair| pair[1].overall_percent >= pair[0].overall_percent));
```

Expected stream shape: every stage emits `Started` then `Finished` in stage order;
artifact verification reads and the rootfs copy emit `InProgress` ticks with byte
counters while bytes advance; `overall_percent` rises monotonically 0 → 100. See
[contract](./contracts/sdk-create-progress.md) for the full emission table and
[data-model](./data-model.md) for the field invariants.

## Run without an observer

```rust
let created = sdk
    .create_microvm(request, None::<fn(CreationProgress)>)
    .await?;
```

Results, typed errors, idempotency, conflicts, and rollback are identical to the
previous one-argument behavior; no events are emitted.

## Check failure and idempotent paths

- Inject a missing prerequisite with an observer attached: the stream ends with a
  single `failed` terminal naming the failed stage (N/6 for the N finished stages),
  the typed error matches the no-observer error, and rollback removes attempt-owned
  resources.
- Repeat an identical creation with an observer: a `Started` event plus a single
  `already-configured` terminal at 0/6, no finishes, no host changes.
- Request with conflicting settings under an existing name: a `Started` event plus a
  single `failed` terminal at `Validation`, 0/6, plus the existing typed conflict error.

## Verification commands

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test -p taumaru-microvm --all-targets --all-features
```

Unit coverage lives in the manager tests module (recording closure over injected
ports); public-export assertions live in `crates/sdk/tests/public_api.rs`; the
silent/typed-error surface with an observer is covered by
`crates/sdk/tests/creation_progress.rs`. Real host integration (KVM, TAP/bridge, DHCP)
remains capability-gated and outside the default suite.
