//! Paired-device operations on the TinySecurity client.

use super::{Client, SecurityError};

impl Client {
    /// Run a paired-device lifecycle operation inside TinySecurity.
    ///
    /// The operation is confidential because it may carry a handshake frame,
    /// an opaque pairing capability, or a host event identifier.
    pub async fn device_pairing2(
        &self,
        request: tinysecurity_bus::DevicePairingRequest2,
    ) -> Result<tinysecurity_bus::DevicePairingResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::DEVICE_PAIRING2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Admit the exact native approval scope into TinySecurity's paired-device
    /// owner. The module derives its secret and persistence bindings from the
    /// scope already registered at initialization.
    pub async fn register_device_pairing_scope2(
        &self,
        request: tinysecurity_bus::RegisterDevicePairingScopeRequest2,
    ) -> Result<tinysecurity_bus::DevicePairingScopeReceipt2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::REGISTER_DEVICE_PAIRING_SCOPE2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Start an audited HumanOnce review for one native device operation.
    pub async fn evaluate_device2(
        &self,
        request: tinysecurity_bus::EvaluateDeviceRequest2,
    ) -> Result<tinysecurity_bus::ApprovalRow2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::EVALUATE_DEVICE2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Execute only the exact native device operation reviewed by TinySecurity.
    pub async fn execute_device2(
        &self,
        request: tinysecurity_bus::ExecuteDeviceRequest2,
    ) -> Result<tinysecurity_bus::DeviceResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::EXECUTE_DEVICE2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Seal or open one live tunnel frame using TinySecurity's opaque session.
    pub async fn device_frame2(
        &self,
        request: tinysecurity_bus::DeviceFrameRequest2,
    ) -> Result<tinysecurity_bus::DeviceFrameResponse2, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::DEVICE_FRAME2, (request,))
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }
}
