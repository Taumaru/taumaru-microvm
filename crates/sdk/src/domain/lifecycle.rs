use std::fmt;

/// Call-time verified state of a MicroVM.
///
/// There is no persisted lifecycle state. A machine reports [`MicroVmState::Running`]
/// if and only if its volume-local control socket answers at call time; anything
/// else reports [`MicroVmState::Stopped`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MicroVmState {
    /// The volume-local control socket answered at call time.
    Running,
    /// The socket did not answer, the probe failed, or no runtime record exists.
    Stopped,
}

impl MicroVmState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for MicroVmState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Network mode requested for a MicroVM.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
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
