//! Host-owned optional ML callback served to the TinyJuice module.

use tinybus::ObjectPath;

// The module calls *out* to this one: the ML plain-text compressor is the
// host's, not the module's. The names come from the contract so the two sides
// cannot drift — a mismatch here is a `NameHasNoOwner` the module swallows by
// falling back to a compressor that needs no ML runtime, which is a silent
// loss of compression rather than a failure anyone sees.
use tinyjuice_bus::names::{ML_HOST_NAME as NAME, ML_HOST_PATH as PATH};

#[derive(Clone)]
struct MlHost;

#[tinybus::interface(name = "ai.tinyhumans.tinyjuice.MlHost")]
impl MlHost {
    async fn compress(
        &self,
        text: String,
        options: serde_json::Value,
    ) -> tinybus::Result<Option<String>> {
        let options = serde_json::from_value(options).map_err(method_error)?;
        crate::openhuman::inference::tokenjuice::ml::compress(&text, &options)
            .await
            .map_err(method_error)
    }
}

fn method_error(error: impl std::fmt::Display) -> tinybus::Error {
    tinybus::Error::MethodFailed {
        name: "ai.tinyhumans.tinyjuice.Error.Host".to_string(),
        message: error.to_string(),
    }
}

pub(super) async fn install(connection: &tinybus::Connection) -> tinybus::Result<()> {
    connection.serve_at(ObjectPath::new(PATH)?, MlHost).await?;
    connection.request_name(NAME).await
}
