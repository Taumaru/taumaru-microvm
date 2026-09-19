-- Routed LAN columns for vm_networks. Legacy bridge/nftables columns stay
-- untouched; pre-release bridge rows are ignored and never migrated.
ALTER TABLE vm_networks ADD COLUMN lan_ip TEXT;
ALTER TABLE vm_networks ADD COLUMN uplink_cidr TEXT;
ALTER TABLE vm_networks
    ADD COLUMN proxy_arp_enabled_by_sdk INTEGER NOT NULL DEFAULT 0
    CHECK (proxy_arp_enabled_by_sdk IN (0, 1));
ALTER TABLE vm_networks
    ADD COLUMN host_route_created_by_sdk INTEGER NOT NULL DEFAULT 0
    CHECK (host_route_created_by_sdk IN (0, 1));
ALTER TABLE vm_networks
    ADD COLUMN proxy_arp_entry_created_by_sdk INTEGER NOT NULL DEFAULT 0
    CHECK (proxy_arp_entry_created_by_sdk IN (0, 1));
ALTER TABLE vm_networks ADD COLUMN iptables_forward_specs TEXT NOT NULL DEFAULT '[]';
ALTER TABLE vm_networks ADD COLUMN iptables_nat_spec TEXT NOT NULL DEFAULT '[]';
