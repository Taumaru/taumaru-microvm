ALTER TABLE distribution_images
    ADD COLUMN minimum_size_bytes INTEGER
    CHECK (minimum_size_bytes IS NULL OR minimum_size_bytes > 0);

CREATE TABLE IF NOT EXISTS network_bridges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bridge_name TEXT NOT NULL,
    uplink_name TEXT NOT NULL,
    ownership TEXT NOT NULL,
    reference_count INTEGER NOT NULL CHECK (reference_count >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (bridge_name, uplink_name),
    UNIQUE (bridge_name)
);

CREATE TABLE IF NOT EXISTS microvms (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK (state IN ('creating', 'running', 'configured')),
    distribution_id TEXT NOT NULL,
    image_id TEXT NOT NULL,
    kernel_id TEXT NOT NULL,
    firecracker_package_id TEXT NOT NULL,
    firectl_package_id TEXT NOT NULL,
    disk_size_bytes INTEGER NOT NULL CHECK (disk_size_bytes > 0),
    memory_requested_bytes INTEGER NOT NULL CHECK (memory_requested_bytes > 0),
    memory_effective_mib INTEGER NOT NULL CHECK (memory_effective_mib > 0),
    vcpu_count INTEGER NOT NULL CHECK (vcpu_count > 0),
    volume_path TEXT NOT NULL UNIQUE,
    rootfs_path TEXT NOT NULL UNIQUE,
    socket_path TEXT NOT NULL UNIQUE,
    expose_on_lan INTEGER NOT NULL CHECK (expose_on_lan IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS vm_networks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    microvm_id INTEGER NOT NULL UNIQUE REFERENCES microvms(id) ON DELETE CASCADE,
    mode TEXT NOT NULL CHECK (mode IN ('host_only', 'lan')),
    guest_ip TEXT NOT NULL,
    prefix_length INTEGER NOT NULL CHECK (prefix_length BETWEEN 0 AND 128),
    gateway_ip TEXT,
    host_ip TEXT,
    tap_name TEXT NOT NULL UNIQUE,
    guest_mac TEXT NOT NULL UNIQUE,
    bridge_id INTEGER REFERENCES network_bridges(id) ON DELETE RESTRICT,
    uplink_name TEXT,
    dhcp_lease_reference TEXT,
    desired_boot_parameters TEXT NOT NULL,
    bridge_created_by_sdk INTEGER NOT NULL DEFAULT 0 CHECK (bridge_created_by_sdk IN (0, 1)),
    uplink_attached_by_sdk INTEGER NOT NULL DEFAULT 0 CHECK (uplink_attached_by_sdk IN (0, 1)),
    forwarding_enabled_by_sdk INTEGER NOT NULL DEFAULT 0 CHECK (forwarding_enabled_by_sdk IN (0, 1)),
    nat_table_created_by_sdk INTEGER NOT NULL DEFAULT 0 CHECK (nat_table_created_by_sdk IN (0, 1)),
    nat_chain_created_by_sdk INTEGER NOT NULL DEFAULT 0 CHECK (nat_chain_created_by_sdk IN (0, 1)),
    host_address_specs TEXT NOT NULL DEFAULT '[]',
    default_route_specs TEXT NOT NULL DEFAULT '[]',
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS vm_network_resources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    microvm_id INTEGER NOT NULL REFERENCES microvms(id) ON DELETE CASCADE,
    resource_kind TEXT NOT NULL,
    resource_identity TEXT NOT NULL,
    desired_fingerprint TEXT NOT NULL,
    ownership TEXT NOT NULL,
    adapter_handle TEXT,
    last_observed TEXT NOT NULL,
    UNIQUE (microvm_id, resource_kind)
);

CREATE TABLE IF NOT EXISTS vm_credentials (
    microvm_id INTEGER PRIMARY KEY REFERENCES microvms(id) ON DELETE CASCADE,
    private_key_path TEXT NOT NULL,
    public_key_path TEXT NOT NULL,
    guest_authorized_keys_path TEXT NOT NULL,
    key_type TEXT NOT NULL CHECK (key_type = 'ed25519'),
    ssh_user TEXT NOT NULL CHECK (ssh_user = 'root'),
    ssh_port INTEGER NOT NULL CHECK (ssh_port = 22),
    public_key_fingerprint TEXT NOT NULL,
    file_mode TEXT NOT NULL,
    UNIQUE (private_key_path),
    UNIQUE (public_key_path)
);

CREATE TABLE IF NOT EXISTS vm_runtime (
    microvm_id INTEGER PRIMARY KEY REFERENCES microvms(id) ON DELETE CASCADE,
    firecracker_path TEXT NOT NULL,
    firectl_path TEXT NOT NULL,
    socket_path TEXT NOT NULL,
    process_id INTEGER,
    process_state TEXT NOT NULL CHECK (process_state IN ('stopped', 'starting', 'running')),
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS microvms_state ON microvms(state);
CREATE INDEX IF NOT EXISTS vm_networks_guest_ip ON vm_networks(guest_ip);
CREATE INDEX IF NOT EXISTS vm_network_resources_identity
    ON vm_network_resources(resource_kind, resource_identity);
CREATE INDEX IF NOT EXISTS network_bridges_uplink ON network_bridges(uplink_name);
