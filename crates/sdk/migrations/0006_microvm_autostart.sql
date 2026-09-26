CREATE TABLE IF NOT EXISTS vm_autostart (
    microvm_id INTEGER PRIMARY KEY REFERENCES microvms(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    max_start_attempts INTEGER NOT NULL CHECK (max_start_attempts BETWEEN 1 AND 10),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
