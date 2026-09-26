//! Development-only TinySearch registration. Published archive pins are added
//! only after a verified release; local overrides work meanwhile.
use crate::modules::types::{LoadPolicy, ModuleRecord};

pub(crate) const TINYSEARCH: ModuleRecord = ModuleRecord {
    id: "tinysearch",
    description: "Search provider tools through TinySearch",
    bus_name: tinysearch_bus::names::INTERFACE,
    object_path: tinysearch_bus::names::OBJECT_PATH,
    version: "0.2.1",
    release_url: "https://github.com/tinyhumansai/tinysearch/releases/tag/v0.2.1",
    assets: &[],
    load: LoadPolicy::Lazy,
};
