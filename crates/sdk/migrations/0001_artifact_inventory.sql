CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    checksum TEXT NOT NULL CHECK (
        length(checksum) = 64
        AND checksum = lower(checksum)
        AND checksum NOT GLOB '*[^0-9a-f]*'
    ),
    applied_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS downloads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    artifact_key TEXT NOT NULL UNIQUE,
    artifact_type TEXT NOT NULL CHECK (artifact_type IN ('kernel', 'binary', 'distribution_image')),
    registry_path TEXT NOT NULL,
    registry_url TEXT NOT NULL,
    filename TEXT NOT NULL,
    relative_path TEXT NOT NULL UNIQUE,
    absolute_path TEXT NOT NULL UNIQUE,
    expected_size_bytes INTEGER NOT NULL CHECK (expected_size_bytes >= 0),
    expected_sha256 TEXT NOT NULL CHECK (
        length(expected_sha256) = 64
        AND expected_sha256 = lower(expected_sha256)
        AND expected_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    actual_size_bytes INTEGER NOT NULL CHECK (actual_size_bytes >= 0),
    actual_sha256 TEXT NOT NULL CHECK (
        length(actual_sha256) = 64
        AND actual_sha256 = lower(actual_sha256)
        AND actual_sha256 NOT GLOB '*[^0-9a-f]*'
    ),
    verification_status TEXT NOT NULL CHECK (verification_status = 'verified'),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_verified_at INTEGER,
    CHECK (actual_size_bytes IS NOT NULL AND actual_sha256 IS NOT NULL)
);

CREATE TABLE IF NOT EXISTS kernels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    registry_id TEXT NOT NULL UNIQUE,
    download_id INTEGER UNIQUE REFERENCES downloads(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    display_name TEXT NOT NULL,
    version TEXT NOT NULL,
    architecture TEXT NOT NULL,
    registry_path TEXT NOT NULL,
    registry_url TEXT NOT NULL,
    filename TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256 TEXT NOT NULL,
    format TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS binary_packages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    registry_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT,
    version TEXT NOT NULL,
    architecture TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS binary_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    binary_package_id INTEGER NOT NULL REFERENCES binary_packages(id) ON DELETE CASCADE,
    download_id INTEGER NOT NULL UNIQUE REFERENCES downloads(id) ON DELETE RESTRICT,
    component_name TEXT NOT NULL,
    registry_path TEXT NOT NULL,
    registry_url TEXT NOT NULL,
    filename TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256 TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    executable INTEGER NOT NULL CHECK (executable IN (0, 1)),
    mode TEXT,
    permissions TEXT,
    format TEXT,
    modified_at TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (binary_package_id, component_name)
);

CREATE TABLE IF NOT EXISTS distributions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    registry_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT NOT NULL,
    distribution TEXT NOT NULL,
    version TEXT NOT NULL,
    codename TEXT NOT NULL,
    architecture TEXT NOT NULL,
    vendor TEXT NOT NULL,
    homepage TEXT NOT NULL,
    default_kernel_registry_id TEXT NOT NULL,
    root_device TEXT NOT NULL,
    min_memory_mb INTEGER NOT NULL CHECK (min_memory_mb >= 0),
    min_vcpus INTEGER NOT NULL CHECK (min_vcpus > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS distribution_boot_args (
    distribution_id INTEGER NOT NULL REFERENCES distributions(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    argument TEXT NOT NULL,
    PRIMARY KEY (distribution_id, position)
);

CREATE TABLE IF NOT EXISTS distribution_images (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    distribution_id INTEGER NOT NULL REFERENCES distributions(id) ON DELETE CASCADE,
    download_id INTEGER NOT NULL UNIQUE REFERENCES downloads(id) ON DELETE RESTRICT,
    registry_id TEXT NOT NULL,
    name TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT NOT NULL,
    variant TEXT NOT NULL,
    format TEXT NOT NULL,
    registry_path TEXT NOT NULL,
    registry_url TEXT NOT NULL,
    filename TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256 TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (distribution_id, registry_id)
);

CREATE TABLE IF NOT EXISTS distribution_kernels (
    distribution_id INTEGER NOT NULL REFERENCES distributions(id) ON DELETE CASCADE,
    kernel_id INTEGER NOT NULL REFERENCES kernels(id) ON DELETE CASCADE,
    is_default INTEGER NOT NULL CHECK (is_default IN (0, 1)),
    PRIMARY KEY (distribution_id, kernel_id)
);

CREATE TABLE IF NOT EXISTS elf_metadata (
    download_id INTEGER PRIMARY KEY REFERENCES downloads(id) ON DELETE CASCADE,
    class TEXT NOT NULL,
    endianness TEXT NOT NULL,
    elf_type TEXT NOT NULL,
    machine TEXT NOT NULL,
    entry_point TEXT NOT NULL,
    build_id TEXT,
    interpreter TEXT,
    linkage TEXT,
    stripped INTEGER CHECK (stripped IN (0, 1))
);

CREATE TABLE IF NOT EXISTS elf_needed_libraries (
    download_id INTEGER NOT NULL REFERENCES elf_metadata(download_id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    library TEXT NOT NULL,
    PRIMARY KEY (download_id, position)
);

CREATE TABLE IF NOT EXISTS image_filesystems (
    distribution_image_id INTEGER PRIMARY KEY REFERENCES distribution_images(id) ON DELETE CASCADE,
    filesystem_type TEXT NOT NULL,
    uuid TEXT NOT NULL,
    block_size INTEGER NOT NULL CHECK (block_size >= 0),
    block_count INTEGER NOT NULL CHECK (block_count >= 0),
    free_blocks INTEGER NOT NULL CHECK (free_blocks >= 0),
    inode_count INTEGER NOT NULL CHECK (inode_count >= 0),
    free_inodes INTEGER NOT NULL CHECK (free_inodes >= 0)
);

CREATE TABLE IF NOT EXISTS image_filesystem_features (
    image_filesystem_id INTEGER NOT NULL REFERENCES image_filesystems(distribution_image_id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    feature TEXT NOT NULL,
    PRIMARY KEY (image_filesystem_id, position)
);

CREATE TABLE IF NOT EXISTS image_capabilities (
    distribution_image_id INTEGER NOT NULL REFERENCES distribution_images(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    capability TEXT NOT NULL,
    PRIMARY KEY (distribution_image_id, position)
);

CREATE UNIQUE INDEX IF NOT EXISTS one_default_kernel_per_distribution
    ON distribution_kernels (distribution_id)
    WHERE is_default = 1;

CREATE INDEX IF NOT EXISTS downloads_type_status
    ON downloads (artifact_type, verification_status);

CREATE INDEX IF NOT EXISTS downloads_expected_sha256
    ON downloads (expected_sha256);

CREATE INDEX IF NOT EXISTS binary_files_package
    ON binary_files (binary_package_id);

CREATE INDEX IF NOT EXISTS distribution_images_distribution
    ON distribution_images (distribution_id);

CREATE INDEX IF NOT EXISTS distribution_kernels_kernel
    ON distribution_kernels (kernel_id);
