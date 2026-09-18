use std::fmt;

/// Durable lifecycle states used while preparing a MicroVM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MicroVmState {
    /// The SDK has claimed the VM and is applying its configuration.
    Creating,
    /// A temporary runtime is active for an operation such as DHCP discovery.
    Running,
    /// The VM is fully configured and stopped.
    Configured,
}

impl MicroVmState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Running => "running",
            Self::Configured => "configured",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "creating" => Some(Self::Creating),
            "running" => Some(Self::Running),
            "configured" => Some(Self::Configured),
            _ => None,
        }
    }
}

impl fmt::Display for MicroVmState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Network mode requested for a MicroVM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkMode {
    /// Isolated host-only `/30` networking with host NAT.
    HostOnly,
    /// LAN networking through an SDK-managed bridge and external DHCP.
    Lan,
}

impl NetworkMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::HostOnly => "host_only",
            Self::Lan => "lan",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "host_only" => Some(Self::HostOnly),
            "lan" => Some(Self::Lan),
            _ => None,
        }
    }
}

impl fmt::Display for NetworkMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
