mod builder;
mod definition;
mod draft;
mod run;

pub(in crate::flows::schemas) use builder::*;
pub(in crate::flows::schemas) use definition::*;
pub(in crate::flows::schemas) use draft::*;
pub(in crate::flows::schemas) use run::*;

use crate::core::Outcome;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

pub(in crate::flows::schemas) fn read_required<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

pub(in crate::flows::schemas) fn to_json<T: serde::Serialize>(
    outcome: Outcome<T>,
) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
