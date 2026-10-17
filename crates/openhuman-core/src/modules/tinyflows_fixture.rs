//! The tinyflows module linked into the test binary: the `static-link` build of
//! `tinyflows-module`, attached once to the process-wide module host.

use crate::config::schema::ModuleSourceMode;
use crate::config::Config;
use crate::modules::{linked::register_linked, ops};
use tokio::sync::OnceCell;

static ATTACHED: OnceCell<Result<(), String>> = OnceCell::const_new();

/// Attach the linked module to the process-wide module host, once.
pub(crate) async fn attach() -> Result<(), String> {
    ATTACHED
        .get_or_init(|| async {
            register_linked(super::MODULE_ID, tinyflows_module::linked_module);
            let mut config = Config::default();
            config.modules.allow_download = false;
            config.modules.source = ModuleSourceMode::Linked;
            ops::ensure_loaded(&config, super::MODULE_ID).await
        })
        .await
        .clone()
}
