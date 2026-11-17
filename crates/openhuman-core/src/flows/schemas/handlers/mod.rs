mod builder;
mod definition;
mod draft;
mod run;

pub(in crate::flows::schemas) use builder::*;
pub(in crate::flows::schemas) use definition::*;
pub(in crate::flows::schemas) use draft::*;
pub(in crate::flows::schemas) use run::*;

use crate::util::params::to_json;
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
