//! CapsHost peer-manifest compatibility for callbacks served to TinyFlows.

use tinybus::{InterfaceName, InterfaceVersion, PeerManifest, Version};

/// Whether `object` is a valid v2 CapsHost service object.
pub(super) fn is_caps_host_v2(object: &impl tinybus::service::Interface) -> Result<bool, String> {
    let interface = tinybus::service::Interface::name(object);
    if interface.as_str() != tinyflows_bus::runs::CAPS_INTERFACE {
        return Ok(false);
    }
    let cancel_member = tinybus::MemberName::new(tinyflows_bus::runs::caps::AGENT_CANCEL)
        .map_err(|error| error.to_string())?;
    if !tinybus::service::Interface::members(object).contains(&cancel_member) {
        return Err("CapsHost v2 object does not expose AgentCancel".to_owned());
    }
    Ok(true)
}

/// Add CapsHost v2 to the existing peer manifest without dropping its other
/// identities, interfaces, or required contracts.
pub(super) async fn announce_caps_host_v2(connection: &tinybus::Connection) -> Result<(), String> {
    static MANIFEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _lock = MANIFEST_LOCK.lock().await;
    let interface = InterfaceName::new(tinyflows_bus::runs::CAPS_INTERFACE)
        .map_err(|error| error.to_string())?;
    let mut manifest = connection
        .manifest_of(crate::modules::tinyflows::HOST_BUS_NAME)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| PeerManifest::new("openhuman-module-host"));
    let caps_host = InterfaceVersion::provided(interface.clone(), Version::new(2, 0, 0));
    if let Some(provided) = manifest
        .provides
        .iter_mut()
        .find(|provided| provided.interface == interface)
    {
        *provided = caps_host;
    } else {
        manifest.provides.push(caps_host);
    }
    connection
        .announce(&manifest)
        .await
        .map_err(|error| format!("announcing the CapsHost v2 contract failed: {error}"))
}
