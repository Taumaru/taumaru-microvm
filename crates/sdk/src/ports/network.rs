use std::net::{IpAddr, Ipv4Addr};

use crate::domain::lifecycle::NetworkMode;
use crate::domain::microvm::{NetworkResource, PersistedNetwork};
use crate::error::SdkError;

/// Uplink identity detected from the host default route.
#[derive(Clone, Debug)]
pub(crate) struct UplinkIdentity {
    pub interface: String,
    pub address: Ipv4Addr,
    pub prefix_length: u8,
    pub gateway: Option<Ipv4Addr>,
    pub cidr: String,
}

/// LAN address offer committed by one creation attempt.
#[derive(Clone, Debug)]
pub(crate) struct LanAddressOffer {
    pub address: Ipv4Addr,
    pub source: LanOfferSource,
    pub uplink_cidr: String,
}

/// How a LAN address offer was selected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LanOfferSource {
    ExplicitOverride,
    PreviousAssignment,
    AutomaticSearch,
}

/// Network inputs that are stable for one VM creation attempt.
#[derive(Clone, Debug)]
pub(crate) struct NetworkRequest {
    pub vm_name: String,
    pub mode: NetworkMode,
    pub guest_mac: String,
    /// Explicit LAN override. `None` selects automatically for LAN mode.
    pub lan_address_override: Option<Ipv4Addr>,
    /// Exact archived private guest address for preserve-policy restore.
    pub guest_address_override: Option<Ipv4Addr>,
    /// Exact archived private prefix for preserve-policy restore.
    pub prefix_length_override: Option<u8>,
    /// Exact archived private gateway for preserve-policy restore.
    pub gateway_override: Option<Ipv4Addr>,
    /// Preserved-policy marker that distinguishes an archived absent gateway from default allocation.
    pub exact_network_values: bool,
}

/// Network result with internal ownership and boot metadata.
#[derive(Clone, Debug)]
pub(crate) struct NetworkOutcome {
    pub persisted: PersistedNetwork,
    pub applied: Vec<NetworkResource>,
    pub skipped: Vec<NetworkResource>,
}

/// Replaceable host-network reconciliation boundary.
pub(crate) trait NetworkController: Send + Sync {
    fn detect_uplink(&self) -> Result<UplinkIdentity, SdkError>;

    fn select_lan_offer(
        &self,
        uplink: &UplinkIdentity,
        lan_override: Option<Ipv4Addr>,
        previous: Option<Ipv4Addr>,
        used_addresses: &[(String, IpAddr, String)],
    ) -> Result<LanAddressOffer, SdkError>;

    fn configure(
        &self,
        request: &NetworkRequest,
        existing: Option<&PersistedNetwork>,
        used_addresses: &[(String, IpAddr, String)],
        used_lan_addresses: &[(String, IpAddr, String)],
    ) -> Result<NetworkOutcome, SdkError>;

    fn cleanup(&self, network: &PersistedNetwork) -> Result<(), SdkError>;

    /// Releases exactly the host items recorded as owned by this VM, skipping
    /// already-absent items as already removed.
    ///
    /// Unlike [`NetworkController::cleanup`], which keeps strict accounting
    /// for the creation-rollback path, this is the delete path: absent means
    /// converged, not failed. A present-but-unreleasable item is a typed
    /// error. Unowned host configuration is never addressed.
    fn cleanup_for_delete(&self, network: &PersistedNetwork) -> Result<(), SdkError> {
        self.cleanup(network)
    }
}
