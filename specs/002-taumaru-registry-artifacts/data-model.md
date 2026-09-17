# Data Model: Taumaru Registry Artifact Integration

## Modeling principles

The inventory distinguishes a logical registry object from each physical file on disk:

- `downloads` is the single source for physical path, transfer, and verification facts.
- Logical tables retain registry metadata and point to the physical row that was adopted or
  downloaded.
- A file is installed only when its physical bytes are currently verified and its required
  logical relationship exists. A logical registry row by itself is not an installed artifact.
- Package and distribution readiness is derived from all of their member rows; one successful
  member cannot make an incomplete aggregate appear ready.
- Every foreign key is enforced by SQLite. Stable registry IDs, artifact keys, paths, and logical
  relationships have uniqueness constraints so versions, architectures, and components cannot
  overwrite one another accidentally.

## Entities and relationships

```text
schema_migrations

downloads ──────── 0..1 ──────── kernels
    │                              │
    ├──────────── 1:1 ───── binary_files ───── N:1 ───── binary_packages
    │
    └──────────── 1:1 ───── distribution_images ───── N:1 ───── distributions
                                                                  │
                                                                  ├── 1:N ── distribution_boot_args
                                                                  └── N:M ── distribution_kernels ── N:1 ── kernels

downloads ── 1:0..1 ── elf_metadata ── 1:N ── elf_needed_libraries
distribution_images ── 1:0..1 ── image_filesystems
image_filesystems ── 1:N ── image_filesystem_features
distribution_images ── 1:N ── image_capabilities
```

The `kernels.download_id` relation is nullable so a distribution can preserve a compatibility
reference to a kernel before that kernel is downloaded. A kernel is considered installed only
when the relation is non-null and the referenced `downloads` row is verified on disk.

## Tables

### `schema_migrations`

Tracks applied SQL files and protects the local schema from silent migration drift.

| Column | Type | Rules |
|--------|------|-------|
| `version` | INTEGER | Primary key; monotonically increasing migration number. |
| `name` | TEXT | Required migration filename. |
| `checksum` | TEXT | Required lowercase SHA-256 of the migration content. |
| `applied_at` | INTEGER | Required UTC Unix timestamp. |

### `downloads`

One row per physical registry member managed below the SDK home. This table is the common
physical-file record referenced by logical content tables.

| Column | Type | Rules |
|--------|------|-------|
| `id` | INTEGER | Primary key. |
| `artifact_key` | TEXT | Required and unique; deterministic key such as `kernel:<registry-id>` or `binary:<package-id>:<component>`. |
| `artifact_type` | TEXT | Required; `kernel`, `binary`, or `distribution_image`. |
| `registry_path` | TEXT | Required registry-relative path. |
| `registry_url` | TEXT | Required source URL. |
| `filename` | TEXT | Required validated filename. |
| `relative_path` | TEXT | Required path relative to the normalized SDK home; unique and never absolute. |
| `absolute_path` | TEXT | Required normalized path below the SDK home; unique. |
| `expected_size_bytes` | INTEGER | Required non-negative registry size. |
| `expected_sha256` | TEXT | Required lowercase 64-hex-character registry digest. |
| `actual_size_bytes` | INTEGER | Required for an inventory row; only verified files are retained. |
| `actual_sha256` | TEXT | Required for an inventory row; only verified files are retained. |
| `verification_status` | TEXT | Required and currently `verified`; failed or invalid files have no inventory row. |
| `created_at` | INTEGER | Required UTC Unix timestamp for first inventory insertion. |
| `updated_at` | INTEGER | Required UTC Unix timestamp for the last metadata/state update. |
| `last_verified_at` | INTEGER | Nullable; latest successful disk verification timestamp. |

Constraints and indexes:

- `artifact_key`, `relative_path`, and `absolute_path` are unique.
- `expected_size_bytes` and `actual_size_bytes` cannot be negative.
- Every retained row has non-null actual size and digest; the repository also rechecks the
  physical file instead of trusting only the row.
- Index `(artifact_type, verification_status)` supports cache and inventory queries.
- Index on `expected_sha256` supports integrity diagnostics without making the digest the identity.

### `kernels`

One logical row per registry kernel ID. It stores the registry description and optionally relates
it to the one physical file downloaded for that kernel.

| Column | Type | Rules |
|--------|------|-------|
| `id` | INTEGER | Primary key. |
| `registry_id` | TEXT | Required and unique registry kernel ID. |
| `download_id` | INTEGER | Nullable unique foreign key to `downloads(id)`; non-null only for an installed/adopted kernel. |
| `name` | TEXT | Required registry name. |
| `display_name` | TEXT | Required display name. |
| `version` | TEXT | Required version. |
| `architecture` | TEXT | Required registry architecture. |
| `registry_path` | TEXT | Required registry path. |
| `registry_url` | TEXT | Required registry URL. |
| `filename` | TEXT | Required filename. |
| `size_bytes` | INTEGER | Required registry size. |
| `sha256` | TEXT | Required registry digest. |
| `format` | TEXT | Required kernel format. |
| `mime_type` | TEXT | Required MIME type. |
| `modified_at` | TEXT | Required registry timestamp. |
| `created_at` / `updated_at` | INTEGER | Required local UTC timestamps. |

Foreign-key behavior uses `ON DELETE RESTRICT` for `download_id`; cleanup is a separate feature
and must not orphan a logical installed record. Optional ELF metadata is held in `elf_metadata`
through the same physical `download_id`.

### `binary_packages`

One logical row per versioned, architecture-specific package from `binaries`.

| Column | Type | Rules |
|--------|------|-------|
| `id` | INTEGER | Primary key. |
| `registry_id` | TEXT | Required and unique package ID. |
| `name` | TEXT | Required machine-friendly product name. |
| `display_name` | TEXT | Required display name. |
| `description` | TEXT | Nullable. |
| `version` | TEXT | Required product version. |
| `architecture` | TEXT | Required registry architecture. |
| `created_at` / `updated_at` | INTEGER | Required local UTC timestamps. |

Package readiness is not a manually trusted flag. It is complete only when every child in
`binary_files` has a unique verified `downloads` row and the corresponding physical path still
passes size and digest validation.

### `binary_files`

One logical package component per row, such as `firecracker` or `jailer`.

| Column | Type | Rules |
|--------|------|-------|
| `id` | INTEGER | Primary key. |
| `binary_package_id` | INTEGER | Required foreign key to `binary_packages(id)`. |
| `download_id` | INTEGER | Required unique foreign key to `downloads(id)`. |
| `component_name` | TEXT | Required component name; unique within a package. |
| `registry_path` / `registry_url` / `filename` | TEXT | Required validated registry/file metadata. |
| `size_bytes` | INTEGER | Required non-negative registry size. |
| `sha256` | TEXT | Required registry digest. |
| `mime_type` | TEXT | Required MIME type. |
| `executable` | INTEGER | Required boolean representation (`0` or `1`). |
| `mode` | TEXT | Nullable numeric Unix mode from the registry. |
| `permissions` | TEXT | Nullable human-readable permissions. |
| `format` | TEXT | Nullable registry format. |
| `modified_at` | TEXT | Required registry timestamp. |
| `created_at` / `updated_at` | INTEGER | Required local UTC timestamps. |

The unique key `(binary_package_id, component_name)` prevents a package from mapping one named
component to two physical files. The unique `download_id` prevents one physical download from
being silently mapped to multiple unrelated components.

### `distributions`

One logical row per distribution release. It does not have a single `download_id` because the
registry represents a distribution as a group of images.

| Column | Type | Rules |
|--------|------|-------|
| `id` | INTEGER | Primary key. |
| `registry_id` | TEXT | Required and unique distribution ID. |
| `name` / `display_name` / `description` | TEXT | Required registry metadata. |
| `distribution` | TEXT | Required machine-friendly distribution name. |
| `version` / `codename` | TEXT | Required release metadata. |
| `architecture` | TEXT | Required registry architecture. |
| `vendor` / `homepage` | TEXT | Required registry metadata. |
| `default_kernel_registry_id` | TEXT | Required registry kernel ID; also represented by `distribution_kernels.is_default`. |
| `root_device` | TEXT | Required boot root device. |
| `min_memory_mb` | INTEGER | Required non-negative minimum memory. |
| `min_vcpus` | INTEGER | Required positive minimum vCPU count. |
| `created_at` / `updated_at` | INTEGER | Required local UTC timestamps. |

### `distribution_boot_args`

Preserves ordered `boot.kernel_args` values without flattening or losing order.

| Column | Type | Rules |
|--------|------|-------|
| `distribution_id` | INTEGER | Foreign key to `distributions(id)`. |
| `position` | INTEGER | Zero-based order; part of the primary key. |
| `argument` | TEXT | Required argument value. |

### `distribution_images`

One logical image per distribution `images` member and one physical `downloads` row per image.

| Column | Type | Rules |
|--------|------|-------|
| `id` | INTEGER | Primary key. |
| `distribution_id` | INTEGER | Required foreign key to `distributions(id)`. |
| `download_id` | INTEGER | Required unique foreign key to `downloads(id)`. |
| `registry_id` | TEXT | Required image ID; unique within a distribution. |
| `name` / `display_name` / `description` | TEXT | Required registry metadata. |
| `variant` / `format` | TEXT | Required image metadata. |
| `registry_path` / `registry_url` / `filename` | TEXT | Required source/file metadata. |
| `size_bytes` | INTEGER | Required non-negative registry size. |
| `sha256` | TEXT | Required registry digest. |
| `mime_type` / `modified_at` | TEXT | Required registry metadata. |
| `created_at` / `updated_at` | INTEGER | Required local UTC timestamps. |

The unique key `(distribution_id, registry_id)` prevents duplicate images. A distribution is
fully ready only when every expected image has a verified physical relation.

### `distribution_kernels`

Many-to-many compatibility relation between logical distributions and logical kernels.

| Column | Type | Rules |
|--------|------|-------|
| `distribution_id` | INTEGER | Required foreign key to `distributions(id)`. |
| `kernel_id` | INTEGER | Required foreign key to `kernels(id)`. |
| `is_default` | INTEGER | Required boolean (`0` or `1`); at most one default per distribution. |

The primary key is `(distribution_id, kernel_id)`. A partial unique index on
`(distribution_id) WHERE is_default = 1` enforces one default kernel. The relation can exist when
the kernel row is only a registry reference (`kernels.download_id IS NULL`); it does not claim
that the kernel is installed.

### `elf_metadata` and `elf_needed_libraries`

These optional tables preserve the nested ELF metadata shared by kernel and binary registry
entries without putting a polymorphic JSON blob in the logical tables.

`elf_metadata` has one row per `downloads` row with ELF data:

- `download_id` primary/foreign key
- `class`, `endianness`, `elf_type`, `machine`, and `entry_point`
- nullable `build_id`, `interpreter`, `linkage`, and `stripped`

`elf_needed_libraries` has `(download_id, position)` as its primary key and stores each ordered
`needed_libraries` value.

### `image_filesystems`, `image_filesystem_features`, and `image_capabilities`

These tables preserve the nested distribution image metadata:

- `image_filesystems` is one-to-one with `distribution_images` and stores filesystem type, UUID,
  block size/counts, and free block/inode counts.
- `image_filesystem_features` stores ordered filesystem features by `(image_filesystem_id,
  position)`.
- `image_capabilities` stores ordered image capabilities by `(distribution_image_id, position)`.

## Cache decision and state transitions

### Physical download state

```text
untracked ── correct existing file ──> verified (adopted)
untracked ── missing/wrong file ─────> transfer ──> verified
verified  ── missing/wrong file ─────> remove ────> transfer ──> verified
verified  ── correct file + relation ─> verified (skipped)
transfer  ── transfer/integrity error ─> untracked
```

The transition rules are:

1. Resolve the current manifest member and validate its ID, URL, path, expected size, and digest.
2. Derive the deterministic target below the normalized SDK home.
3. Read the target's actual metadata and calculate SHA-256 incrementally if the file exists.
4. Query whether the `downloads` row and the required logical/content row both exist and match
   the current registry identity and expected metadata.
5. If the file is correct and both rows are complete/verified, return `SkippedExisting` without a
   network request.
6. If the file is correct but the relationship is absent or incomplete, insert/update the rows in
   one short transaction and return `AdoptedExisting` without a network request.
7. If the file is missing or differs in size/digest, remove any existing physical and logical
   inventory rows and remove the invalid target only after validating that it belongs below the
   SDK home, transfer into a unique temporary sibling, and verify it. If the invalid file is a
   downloaded kernel, delete its `distribution_kernels` rows before deleting the kernel row so
   no compatibility relationship can reference the invalid artifact. A kernel row created only
   as a registry reference, without an invalid local file, may remain.
8. After successful verification, flush/sync and atomically rename the temporary file, apply the
   required executable mode, and commit the `downloads` plus logical content relationship.
9. If any step fails, return a typed error. No temporary, invalid, or stale inventory record is
   exposed as installed; successfully verified members of other package/distribution requests may
   remain reusable.

For packages and distributions, each member follows the same state machine. The aggregate result
is complete only when all members are verified. Valid member rows may remain available for a later
retry after another member fails.

## Migration invariants

- `MicroVmSdk::new` creates the SDK home and `state` directory before opening the database, then
  runs migrations before returning.
- Each migration file is applied once by version and checksum, inside a transaction.
- Re-running SDK construction is a no-op for matching applied migrations and preserves all rows.
- Schema changes never drop or overwrite existing inventory data automatically.
- An applied migration whose file name or checksum differs from `schema_migrations` fails with a
  typed migration-drift error.
- Required tables, columns, indexes, foreign keys, and constraints are checked after the initial
  migration so an incompatible pre-existing database is reported rather than silently accepted.
