# Research: Image Artifact Download

## Scope

This research covers the additive single-image acquisition in the SDK and the rebuilt CLI
flow around one image multi-select under `microvm artifacts download`. The existing
whole-distribution operations, SQLite schema, cache and integrity behavior, runtime-bundle
policy, progress callback shape, cancellation token, and `create_microvm` prerequisites
are reused unchanged. No new workspace dependencies are introduced.

## Decisions

### 1. New SDK result type `DownloadedDistributionImage`

Decision: add a dedicated result struct alongside the two new operations:

```rust
pub struct DownloadedDistributionImage {
    pub distribution: Distribution,
    pub image: DistributionImage,
    pub file: DownloadedFile,
}

impl MicroVmSdk {
    pub async fn download_distribution_image(
        &self,
        distribution_id: &str,
        image_id: &str,
    ) -> Result<DownloadedDistributionImage, SdkError>;

    pub async fn download_distribution_image_with_cancellation(
        &self,
        distribution_id: &str,
        image_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: impl FnMut(DownloadProgress) + Send,
    ) -> Result<DownloadedDistributionImage, SdkError>;
}
```

The struct carries the distribution metadata (needed by `persist_distribution_image`
for boot arguments and kernel references), the exact image metadata, and the single
verified file. The implementation reuses the existing private
`distribution_image_member(distribution, image, kernels)` plus `download_member`,
with a single-member `ProgressTracker` — the same path `download_distribution` uses
per image, minus the loop.

Alternatives considered:

- Reuse `DownloadedDistribution` with a one-element `images` vector. Rejected: the type
  name and plural contract imply whole-distribution acquisition, and callers could not
  distinguish "this distro has one image" from "only one image was requested".
- Return a bare `DownloadedFile`. Rejected: drops the distribution and image metadata
  the CLI review step and future callers need, and diverges from the
  `DownloadedKernel { kernel, file }` convention.
- Add an image filter parameter to `download_distribution`. Rejected: changes existing
  method semantics and risks breaking the Taumaru Agent consumer the spec keeps
  compatible.

Error semantics mirror the existing operations: blank IDs fail `validate_requested_id`;
an unknown distribution returns `NotFound { kind: "distribution", .. }`; an image ID
absent from that distribution returns `NotFound { kind: "distribution image", .. }`;
transport, integrity, persistence, incompatibility, and `Cancelled` paths flow through
the shared `download_member` machinery. No host-architecture check is added in the SDK —
matching the existing `download_*` methods — so CLI-side filtering remains the single
compatibility gate and scripted callers get the same behavior as interactive ones.

### 2. No persistence or creation changes

Decision: reuse `persist_distribution_image`, `resolve_distribution_image`, the
`distribution_image:distro:image` artifact key, and the existing
`artifacts/rootfs/{distro}/{image}/{filename}` layout with zero migrations.
`create_microvm` already resolves `distribution_id + image_id` and the distribution
default kernel, so it needs no change.

Single-image acquisition writes exactly one `distribution_images` row (plus the shared
distribution, boot-args, and kernel-reference upserts that row already performs).
Whole-distribution acquisition is preserved verbatim for its current consumers. A
single-image download followed by a whole-distribution download of the same
distribution converges on the same rows; verification is per file, so no cross-image
state leaks.

Alternative considered: a separate per-image table or key namespace. Rejected: the
current key already isolates images, and a second namespace would split
`resolve_distribution_image` callers for no behavioral gain.

### 3. Flattened, sorted, deduplicated image catalog in the CLI

Decision: build the selection list by filtering distributions to the host architecture,
then flattening `(distribution, image)` pairs and sorting by
`(distribution.id, image.id)`. Registry IDs — not display names — drive the order, so
review, progress, transfer, and tests are deterministic regardless of selection order.
Repeated picks of the same pair (manual re-select or repeated `--image` flag) collapse
to one entry transferred once, consistent with the existing unique-kernel dedup.

`DistributionImage` carries no architecture field; it inherits its distribution's
architecture. No per-image arch check beyond the distribution gate is needed. Duplicate
image IDs are scoped per distribution (validated in `RegistryCatalog::new` per
distribution, unchanged), so the pair — never a bare image ID — is the plan key. This
is why the explicit flag keeps the scoped `DISTRIBUTION=IMAGE` form.

Alternatives considered:

- Sort by display name. Rejected: display names are human text, not guaranteed unique
  or stable across registry edits; IDs are.
- Preserve click order. Rejected: makes review output, progress order, and byte totals
  depend on input order, breaking deterministic tests and scripted comparisons.
- Reject duplicates as errors. Rejected: contradicts the clarification decision and
  punishes harmless script repetition; dedup is idempotent and matches kernel sharing.

### 4. Default-kernel resolution with no override

Decision: for each affected distribution, resolve `distribution.default_kernel` against
the catalog's kernel list and require host-architecture compatibility
(kernel arch == host arch == distribution arch, reusing the existing
`compatible_kernels` intersection). A missing or incompatible default is a plan
validation failure before confirmation, naming the distribution and the default kernel.
Unique defaults execute once (sorted by kernel ID); a kernel failure skips only its
dependent images while independent images continue.

This matches `create_microvm`, which resolves the same default kernel at creation
time — so every downloaded image is creation-satisfiable with no extra kernel step.
There is deliberately no `--custom-kernel` flag: a download-time custom kernel would
be misleading because creation would still resolve the default.

Alternatives considered:

- Keep a per-distribution kernel prompt. Rejected by the spec: the prompt was the
  confusion being removed, and any non-default choice would be dead weight at
  creation.
- Let the CLI skip kernel download when creation "might" fetch it. Rejected:
  creation never downloads; an image without its default kernel is an incomplete
  preparation.

### 5. Nested `artifacts download` command with scoped `--image` flag

Decision: introduce a Clap parent subcommand so the entry becomes
`microvm artifacts download`, with `DownloadArgs { images: Vec<String>,
non_interactive: bool }` where `--image` repeats as `DISTRIBUTION_ID=IMAGE_ID`.
`--distribution` and `--kernel` are removed; Clap's unknown-argument error plus the
command help point to the new form. The old top-level `download` path is deleted, not
aliased — the spec calls the rename a deliberate breaking change with migration
guidance.

Explicit mode triggers on `non_interactive || !images.is_empty()`; partial explicit
input never falls back to prompts. Non-TTY without explicit selections errors with the
required flag form, as before.

Alternatives considered:

- Bare `--image IMAGE` assuming global uniqueness. Rejected: image IDs are unique per
  distribution, not globally; the scoped form is unambiguous and mirrors the
  interactive entry (which always names the parent distribution).
- Keep `--distribution` as "all images of this distro". Rejected by clarification
  (option A): two selection models would reintroduce the whole-distribution semantics
  being removed.
- Hidden alias for old `download`. Rejected: contradicts the spec's explicit
  replace-not-alias rule and would preserve the old contract in help output.

### 6. Per-image plan members, labels, and execution order

Decision: replace the `DistributionImages { distribution_id }` group member with one
`DistributionImage { distribution_id, image_id, expected_bytes }` member per selected
image. Execution order after confirmation: runtime packages (sorted), unique default
kernels (sorted by ID), selected images (sorted by distribution then image). Labels
follow the SDK progress shape: `distribution/{distro}/{image}` for images (identical
to the existing `artifact_label` rendering of `DistributionImage` progress events),
`kernel/{id}`, `runtime/{id}`. The internal `ArtifactClient` boundary gains
`download_distribution_image` and drops `download_distribution`; `VerifiedArtifact`
gains a per-image variant holding `DownloadedDistributionImage`.

Cancellation helpers become `append_cancelled_after_runtime`,
`append_cancelled_after_kernel` (remaining kernels, then remaining images), and a new
`append_cancelled_after_image` for the image tail. Byte accounting stays checked-add
per member with `plan.expected_bytes` covering only selected artifacts.

Alternatives considered:

- One member per distribution containing its selected images. Rejected: couples
  failure granularity back to the distribution (one image failure would taint siblings)
  and complicates the sorted global order.
- Keep `distribution/{id}` labels with an image suffix elsewhere. Rejected: diverges
  from the SDK progress label the renderer already emits for the same file.

### 7. Keep `inquire` MultiSelect and `indicatif` renderer, retargeted at images

Decision: one `inquire::MultiSelect` over flattened image entries with labels carrying
image identity, parent distribution, variant or capabilities, and size; no `Select`
kernel step. The existing `ProgressForwarder`/`normalize_progress` and `indicatif`
aggregate renderer are reused unchanged — per-image SDK events already carry
`(artifact_id=distro, member_name=image)`, so no progress-shape change is needed.
Review lists each image row with its resolved default kernel; the resolved kernels are
shown grouped, not chosen.

Alternatives considered (`crossterm` raw handling, `ratatui` fullscreen, `dialoguer`)
were already rejected in the prior feature's research for the same reasons: larger
surface, owned terminal lifecycle, harder deterministic non-TTY behavior. Nothing
about image selection changes that tradeoff.

### 8. Test strategy: fake client retarget, fixture-backed SDK tests

Decision: extend the existing seams rather than adding harnesses. CLI unit tests keep
the SDK-shaped `RecordingClient` fake (now recording `image:distro/image` calls),
covering catalog flattening/filtering, `--image` parsing and dedup, default-kernel
resolution and failure, sorted ordering, kernel-failure skips only dependents, and
cancellation tails. SDK tests use the existing `FixtureServer` + manifest (which
already serves two images: `alpine-test-minimal`, `alpine-test-debug`) to prove
single-image download stores only the requested image, whole-distribution still
stores both, and not-found/cancelled paths return typed errors. Command-surface tests
assert the renamed help path and the removed-flag rejection.

## Repository findings used by the plan

- `MicroVmSdk::download_distribution_with_cancellation` (`crates/sdk/src/manager.rs`)
  builds members via `distribution_image_member` in a loop — the new method is that
  loop body for one validated pair.
- `persist_distribution_image` (`crates/sdk/src/adapters/persistence/sqlite.rs`) and
  `resolve_distribution_image(distro, image)` already isolate images; no migration.
- `create_microvm` resolves `distribution.default_kernel` and `request.image_id`
  (`manager.rs` creation prerequisites) — no creation change.
- CLI planning/execution/output seams (`commands/download.rs`, `output/human.rs`,
  `RecordingClient` fake, `command_surface.rs`) are all retargetable without new
  crates or dependencies.
- Fixture manifest already publishes two images per distribution with servable
  payloads, so single-vs-whole assertions need no new fixture data.
