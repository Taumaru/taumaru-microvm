CREATE TABLE IF NOT EXISTS vm_snapshot_metadata (
    microvm_id INTEGER PRIMARY KEY REFERENCES microvms(id) ON DELETE CASCADE,
    distribution_name TEXT NOT NULL,
    distribution_version TEXT NOT NULL,
    root_device TEXT NOT NULL,
    kernel_args_json TEXT NOT NULL,
    image_sha256 TEXT NOT NULL,
    guest_architecture TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS restore_journal (
    operation_id TEXT PRIMARY KEY,
    vm_name TEXT NOT NULL,
    staging_path TEXT NOT NULL,
    volume_path TEXT NOT NULL,
    volume_created INTEGER NOT NULL CHECK (volume_created IN (0, 1)),
    kernel_path TEXT NOT NULL,
    kernel_created INTEGER NOT NULL CHECK (kernel_created IN (0, 1)),
    network_json TEXT,
    progress_state TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS restore_journal_vm_name ON restore_journal (vm_name);
