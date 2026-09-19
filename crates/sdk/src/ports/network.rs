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
}

/// Network result with internal ownership and boot metadata.
#[derive(Clone, Debug)]
pub(crate) struct NetworkOutcome {
    pub persisted: PersistedNetwork,
    pub applied: Vec<NetworkResource>,
    pub skipped: Vec<NetworkResource>,
    pub requires_temporary_runtime: bool,
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
    ) -> Result<NetworkOutcome, SdkError>;

    fn cleanup(&self, network: &PersistedNetwork) -> Result<(), SdkError>;

    fn apply_guest_routed_setup(
        &self,
        private_key_path: &std::path::Path,
        private_address: Ipv4Addr,
        lan_address: Ipv4Addr,
        gateway: Ipv4Addr,
    ) -> Result<(), SdkError>;
}
