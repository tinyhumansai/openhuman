//! Confidential tunnel RPC frame processing.
use super::*;

pub(super) async fn handle_encrypted_rpc_frame(
    ctx: &DispatchCtx,
    channel_id: &str,
    config: &crate::config::Config,
    frame: &[u8],
    connection_token: &str,
    connection_nonce: &str,
    expires_at_ms: u64,
) {
    let pending = PENDING_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(channel_id)
        .cloned();
    if PENDING_HANDSHAKES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(channel_id)
    {
        let Some(session) = pending else { return };
        if session.pairing_token != connection_token || session.connection_nonce != connection_nonce
        {
            return;
        }
        confirm_pairing(
            channel_id,
            ctx.tenant().ok().and_then(|tenant| tenant.agent),
            config,
            &session,
            frame,
            connection_token,
            connection_nonce,
            expires_at_ms,
        )
        .await;
        return;
    }
    let active = ACTIVE_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(channel_id)
        .cloned();
    let Some(active) = active else {
        log::warn!("[devices/bus] encrypted frame without active session channel_id={channel_id}");
        return;
    };
    if active.pairing_token != connection_token
        || !active_session_matches_connection(&active, connection_nonce)
    {
        ACTIVE_SESSIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(channel_id);
        log::warn!(
            "[devices/bus] active session belongs to an older socket channel_id={channel_id}"
        );
        return;
    }
    let Ok(client) = crate::modules::security::client(config).await else {
        return;
    };
    let Ok((event_id, _scope_id)) = crate::modules::security::device_pairing_socket_event(
        config,
        channel_id,
        &active.pairing_token,
        connection_nonce,
        expires_at_ms,
    )
    .await
    else {
        return;
    };
    let refreshed = client
        .device_pairing2(tinysecurity_bus::DevicePairingRequest2 {
            call_id: uuid::Uuid::new_v4().simple().to_string(),
            operation: tinysecurity_bus::DevicePairingOperation2::RefreshSession {
                session_id: active.session_id.clone(),
                host_event_id: event_id,
            },
        })
        .await;
    if !refreshed.is_ok_and(|response| response.session_id.as_deref() == Some(&active.session_id)) {
        log::warn!("[devices/bus] TinySecurity could not refresh session channel_id={channel_id}");
        ACTIVE_SESSIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(channel_id);
        return;
    }
    let opened = client
        .device_frame2(tinysecurity_bus::DeviceFrameRequest2 {
            scope_id: active.scope_id.clone(),
            handle: active.session_id.clone(),
            call_id: uuid::Uuid::new_v4().simple().to_string(),
            operation: tinysecurity_bus::DeviceFrameOperation2::Open {
                frame: frame.to_vec(),
            },
        })
        .await;
    let Ok(opened) = opened else {
        log::warn!("[devices/bus] TinySecurity rejected frame channel_id={channel_id}");
        return;
    };
    let envelope = match serde_json::from_slice::<
        crate::security::devices::tunnel_rpc::TunnelEnvelope,
    >(&opened.bytes)
    {
        Ok(envelope) => envelope,
        Err(error) => {
            log::warn!(
                "[devices/bus] tunnel envelope parse failed channel_id={channel_id}: {error}"
            );
            return;
        }
    };
    if envelope.kind != "request" {
        return;
    }
    let request =
        match crate::security::devices::tunnel_rpc::parse_tunnel_rpc_payload(envelope.payload) {
            Ok(request) => request,
            Err(error) => {
                send_tunnel_response(
                    channel_id,
                    config,
                    &active,
                    connection_nonce,
                    expires_at_ms,
                    &envelope.request_id,
                    "error",
                    Value::String(format!("invalid tunnel RPC payload: {error}")),
                )
                .await;
                return;
            }
        };
    let result = crate::security::devices::tunnel_rpc::dispatch_tunnel_rpc(ctx, request).await;
    match result {
        Ok(value) => {
            send_tunnel_response(
                channel_id,
                config,
                &active,
                connection_nonce,
                expires_at_ms,
                &envelope.request_id,
                "response",
                value,
            )
            .await
        }
        Err(message) => {
            send_tunnel_response(
                channel_id,
                config,
                &active,
                connection_nonce,
                expires_at_ms,
                &envelope.request_id,
                "error",
                Value::String(message),
            )
            .await
        }
    }
}

async fn confirm_pairing(
    channel_id: &str,
    agent: Option<String>,
    config: &crate::config::Config,
    session: &crate::security::devices::types::PairingSession,
    frame: &[u8],
    connection_token: &str,
    connection_nonce: &str,
    expires_at_ms: u64,
) {
    if !pending_session_matches_socket(session, channel_id, connection_token, connection_nonce)
        || !pairing_session_is_live(session)
    {
        return;
    }
    let Ok((event_id, scope_id)) = crate::modules::security::device_pairing_socket_event(
        config,
        channel_id,
        &session.pairing_token,
        connection_nonce,
        expires_at_ms,
    )
    .await
    else {
        return;
    };
    let Ok(client) = crate::modules::security::client(config).await else {
        return;
    };
    let result = client
        .device_pairing2(tinysecurity_bus::DevicePairingRequest2 {
            call_id: uuid::Uuid::new_v4().simple().to_string(),
            operation: tinysecurity_bus::DevicePairingOperation2::Confirm {
                receipt: session.pairing_receipt.clone(),
                host_event_id: event_id,
                frame: frame.to_vec(),
            },
        })
        .await;
    let Ok(result) = result else {
        log::warn!(
            "[devices/bus] TinySecurity rejected pairing confirmation channel_id={channel_id}"
        );
        return;
    };
    let (Some(pairing_id), Some(session_id), Some(device_public_key)) = (
        result.pairing_id,
        result.session_id,
        result.device_public_key,
    ) else {
        log::warn!("[devices/bus] malformed TinySecurity confirmation channel_id={channel_id}");
        return;
    };
    if !result.frame.is_empty() {
        return;
    }
    let token_hash = hash_session_token(&session.pairing_token);
    let renewal_pairing_id = PENDING_RENEWALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(channel_id)
        .cloned();
    let persisted = if let Some(expected_pairing_id) = renewal_pairing_id.as_deref() {
        match store::get_device(config, channel_id) {
            Ok(Some(existing))
                if renewal_device_matches(
                    &existing,
                    expected_pairing_id,
                    &pairing_id,
                    &device_public_key,
                ) =>
            {
                store::touch_device(config, channel_id).and_then(|()| {
                    store::get_device(config, channel_id)
                        .map(|device| {
                            device.ok_or_else(|| anyhow::anyhow!("renewed device disappeared"))
                        })
                        .and_then(|device| device)
                })
            }
            _ => Err(anyhow::anyhow!("renewed device metadata mismatch")),
        }
    } else {
        store::insert_device_with_pairing_id(
            config,
            channel_id,
            &session.label,
            &device_public_key,
            &token_hash,
            &pairing_id,
        )
    };
    match persisted {
        Ok(device) => {
            crate::security::devices::owner::remember(channel_id, agent);
            BUS.publish(DomainEvent::DevicePaired {
                channel_id: channel_id.to_owned(),
                device_pubkey: device_public_key,
                label: Some(device.label),
            });
            ACTIVE_SESSIONS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    channel_id.to_owned(),
                    crate::security::devices::rpc::ActiveDeviceSession {
                        scope_id,
                        session_id,
                        pairing_token: session.pairing_token.clone(),
                        connection_nonce: connection_nonce.to_owned(),
                    },
                );
            PENDING_SESSIONS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(channel_id);
            PENDING_RENEWALS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(channel_id);
            PENDING_HANDSHAKES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(channel_id);
            let _ = device;
            log::info!(
                "[devices/bus] pairing confirmed channel_id={channel_id} pairing_id={pairing_id}"
            );
        }
        Err(error) => {
            log::error!("[devices/bus] could not persist paired device {channel_id}: {error}")
        }
    }
}

async fn send_tunnel_response(
    channel_id: &str,
    config: &crate::config::Config,
    session: &crate::security::devices::rpc::ActiveDeviceSession,
    _connection_nonce: &str,
    _expires_at_ms: u64,
    request_id: &str,
    kind: &str,
    payload: Value,
) {
    let response =
        crate::security::devices::tunnel_rpc::response_envelope(request_id, kind, payload);
    let Ok(plaintext) = serde_json::to_vec(&response) else {
        return;
    };
    let Ok(client) = crate::modules::security::client(config).await else {
        return;
    };
    let sealed = client
        .device_frame2(tinysecurity_bus::DeviceFrameRequest2 {
            scope_id: session.scope_id.clone(),
            handle: session.session_id.clone(),
            call_id: uuid::Uuid::new_v4().simple().to_string(),
            operation: tinysecurity_bus::DeviceFrameOperation2::Seal { plaintext },
        })
        .await;
    let Ok(sealed) = sealed else {
        log::error!("[devices/bus] TinySecurity failed to seal response channel_id={channel_id}");
        return;
    };
    if let Err(error) = emit_frame(channel_id, &encode_frame(&sealed.bytes)).await {
        log::error!("[devices/bus] tunnel response emit failed channel_id={channel_id}: {error}");
    }
}

pub(super) fn decode_frame(value: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .ok()
}

pub(super) fn encode_frame(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
