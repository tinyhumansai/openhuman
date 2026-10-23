use tinysecurity_bus::CallerContext;

/// Caller identity supplied exclusively by trusted host dispatch. This wrapper
/// intentionally has no Deserialize implementation or model-facing controller.
pub(crate) struct VerifiedCaller(pub(crate) CallerContext);

impl VerifiedCaller {
    /// Wrap already authenticated identity and authority, never model arguments.
    pub(crate) fn from_host(context: CallerContext) -> Result<Self, String> {
        if context.agent.trim().is_empty() || context.call_id.trim().is_empty() {
            return Err("TinySecurity caller identity is missing".into());
        }
        Ok(Self(context))
    }
}

/// A failed module call, and whether it must latch the shared client.
///
/// Only a failure of the channel (transport, attestation, timeout, a vanished
/// peer) latches: nothing it says can be trusted afterwards. The module
/// answering that it refused this one request does not, so one hostile or odd
/// request cannot disable the module for everyone.
#[derive(Debug)]
pub(crate) struct CallError {
    pub(crate) message: String,
    pub(crate) fault: bool,
}

impl CallError {
    /// A host-side refusal of one request; the client stays usable.
    pub(crate) fn refused(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fault: false,
        }
    }

    /// A channel failure.
    pub(crate) fn faulted(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fault: true,
        }
    }

    /// Classify a bus error: a method that ran and failed, or arguments the
    /// module could not accept, refuse the request; anything else faults.
    pub(crate) fn from_bus(message: &'static str, error: &tinybus::Error) -> Self {
        let refused = matches!(
            error,
            tinybus::Error::MethodFailed { .. } | tinybus::Error::BadArguments { .. }
        );
        Self {
            message: message.to_owned(),
            fault: !refused,
        }
    }
}

impl From<CallError> for String {
    fn from(error: CallError) -> Self {
        error.message
    }
}
