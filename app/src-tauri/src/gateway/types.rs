//! What a gateway is, and what activating one produces.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The port `openhuman-core` serves RPC on inside a box.
///
/// Fixed rather than configurable: it is an address in the box's own namespace,
/// so it cannot collide with anything on this machine, and the port that
/// actually matters — the one on this side — is chosen when the forward opens.
pub const CORE_PORT_IN_BOX: u16 = 7788;

/// Where the frontend's RPC calls should go.
///
/// Every gateway resolves to one of these and nothing else, which is what keeps
/// the rest of the app out of this: `core_rpc_url` and `core_rpc_token` answer
/// from here, so a screen calling `openhuman.app_state_snapshot` cannot tell a
/// container on another continent from the core in this process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveGateway {
    /// The gateway this came from.
    pub id: String,
    /// The full JSON-RPC endpoint, e.g. `http://127.0.0.1:54321/rpc`.
    pub rpc_url: String,
    /// The bearer every request must carry, when the core requires one.
    pub token: Option<String>,
}

/// How to reach a machine.
///
/// Mirrors tinybox's `HostRef` axis. It is *only* reach: what confines the
/// core once it is there is [`Confinement`], and the two compose freely — which
/// is why `Ssh` + [`Confinement::Docker`] needs no variant of its own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Reach {
    /// This machine.
    #[default]
    Local,
    /// Another machine, over SSH.
    Ssh(SshReach),
}

/// Everything `ssh` needs that the user's `~/.ssh/config` does not already say.
///
/// Deliberately thin. Jump hosts, multiplexing, and per-host keys already live
/// in that file and work better there; re-modelling them here would mean
/// maintaining a second, worse copy of OpenSSH's configuration language.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshReach {
    /// `machine`, `user@machine`, or a name from the user's SSH config.
    pub destination: String,
    /// A non-default port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// An explicit private key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<PathBuf>,
    /// Trust an *unknown* host key on first connect.
    ///
    /// Never ignores a *changed* one — that is the case that means something is
    /// wrong, and tinybox does not offer a way to wave it through.
    #[serde(default)]
    pub accept_new_host_key: bool,
}

/// What confines the core on whichever machine it runs on.
///
/// Mirrors tinybox's `SandboxRef` axis. Only two of tinybox's four sandboxes
/// appear: `namespace` and `microvm` decline `Capability::Detach`, so neither
/// can host a server between commands. Offering them would be offering a
/// gateway that cannot work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Confinement {
    /// None. The core runs as an ordinary process on the target machine.
    Passthrough {
        /// The `openhuman-core` binary over there.
        binary: PathBuf,
        /// The directory it runs in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<PathBuf>,
    },
    /// A Docker container built from an image that ships the core.
    Docker {
        /// The image reference.
        image: String,
    },
}

/// One way of reaching a core, as the user configured it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GatewaySpec {
    /// The core running inside this process. The default, and what the app has
    /// always done.
    Desktop,
    /// A core someone else is running, reached over HTTP.
    ///
    /// This is the pre-existing "cloud" mode, expressed through the same seam
    /// rather than alongside it. Nothing is provisioned: the URL is the answer.
    Remote {
        /// The JSON-RPC endpoint.
        url: String,
        /// The bearer it expects, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    /// A core this app provisions and runs in a tinybox box.
    ///
    /// The two axes are independent, so this one variant covers a container
    /// here, a bare process on a remote machine, and a container on a remote
    /// machine — the last of which needs no code naming that pairing.
    Box {
        /// Which machine.
        #[serde(default)]
        reach: Reach,
        /// What confines it there.
        confinement: Confinement,
        /// Variables for the core process, layered over the box's own.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
}

impl GatewaySpec {
    /// A short, stable word for this kind, for logs and for the UI's picker.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Remote { .. } => "remote",
            Self::Box {
                reach: Reach::Local,
                confinement: Confinement::Docker { .. },
                ..
            } => "docker",
            Self::Box {
                reach: Reach::Ssh(_),
                confinement: Confinement::Docker { .. },
                ..
            } => "ssh+docker",
            Self::Box {
                reach: Reach::Ssh(_),
                ..
            } => "ssh",
            Self::Box { .. } => "local-process",
        }
    }

    /// Whether activating this needs a box provisioned and a core started.
    #[must_use]
    pub const fn provisions(&self) -> bool {
        matches!(self, Self::Box { .. })
    }
}

/// Whether a host is the loopback on this machine.
///
/// Reused by [`validate_remote_transport`] and by the transport layer to allow
/// plain HTTP only to the local core, never to a remote one carrying a bearer.
fn is_loopback_host(host: &str) -> bool {
    let h = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    h == "127.0.0.1" || h == "::1" || h == "localhost" || h.ends_with(".localhost")
}

/// Reject a remote gateway URL that would ship its bearer in cleartext.
///
/// A `Remote` gateway carries an optional bearer that the shell attaches to
/// every RPC request. Sending that over plain HTTP to an arbitrary host would
/// expose the credential on the wire, so a URL backed by a bearer must be
/// `https` — with the single exception of loopback addresses, where the bytes
/// never leave this machine and cleartext is already the norm for a local core.
///
/// # Errors
///
/// Returns a user-facing message when `url` is not parseable, when it is not
/// `http(s)`, or when it would transmit a bearer over an unauthenticated
/// non-loopback transport.
pub fn validate_remote_transport(url: &str, token: Option<&str>) -> Result<(), String> {
    let parsed = url
        .parse::<url::Url>()
        .map_err(|_| format!("{url} is not a valid URL"))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("{url} is not an http(s) endpoint"));
    }
    let has_bearer = token.map(str::trim).is_some_and(|t| !t.is_empty());
    if has_bearer && scheme == "http" && !is_loopback_host(parsed.host_str().unwrap_or_default()) {
        return Err(
            "a remote core with a bearer token must be reached over https, not plain http"
                .to_owned(),
        );
    }
    Ok(())
}

/// A gateway record as it is stored and shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gateway {
    /// Stable identifier, referenced by the frontend's `coreMode`.
    pub id: String,
    /// What the user called it.
    pub label: String,
    /// How to reach a core.
    pub spec: GatewaySpec,
}

/// The identifier of the always-present desktop gateway.
///
/// A record rather than a special case, so the picker has something to select
/// and "no gateways configured" is never a state the UI has to handle.
pub const DESKTOP_ID: &str = "desktop";

impl Gateway {
    /// The gateway that is always available: the core in this process.
    #[must_use]
    pub fn desktop() -> Self {
        Self {
            id: DESKTOP_ID.to_owned(),
            label: "This computer".to_owned(),
            spec: GatewaySpec::Desktop,
        }
    }
}

/// A gateway record as the renderer is allowed to see it.
///
/// Deliberately credential-free, where [`Gateway`] is not: a `Remote` spec
/// carries a bearer and a `Box`/SSH spec carries a destination and an
/// explicit identity path, none of which the list UI needs. The renderer
/// shows the id, the label and the kind badge, nothing else — every field
/// here is what `GatewaySection` reads, and the full [`Gateway`] stays in the
/// shell store (and is only passed back in by the user's own save).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewaySummary {
    /// Stable identifier, referenced by the frontend's `coreMode`.
    pub id: String,
    /// What the user called it.
    pub label: String,
    /// The short stable kind word for the badge, e.g. `"ssh+docker"`.
    ///
    /// A summary keeps the kind but drops the credential-bearing spec leaves
    /// (`token`, `identity`, `destination`), which the renderer never reads.
    pub kind: String,
}

impl From<&Gateway> for GatewaySummary {
    fn from(g: &Gateway) -> Self {
        Self {
            id: g.id.clone(),
            label: g.label.clone(),
            kind: g.spec.kind().to_string(),
        }
    }
}

/// What a gateway is doing right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum GatewayStatus {
    /// Not the active gateway.
    Inactive,
    /// Being provisioned — a box created, a core started, a tunnel opened.
    Activating {
        /// Which step, for the UI to show instead of an untimed spinner.
        step: String,
    },
    /// Active and answering.
    Connected {
        /// Where it is reachable, with any credentials removed.
        endpoint: String,
    },
    /// Activation failed, with the reason it failed.
    Failed {
        /// A message safe to show a user: no bearer, no key path.
        reason: String,
    },
}
