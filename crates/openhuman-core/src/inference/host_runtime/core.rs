use std::sync::Arc;

use crate::config::Config;

use tinyinference_local::service::LocalAiService;

static LOCAL_AI: once_cell::sync::OnceCell<Arc<LocalAiService>> = once_cell::sync::OnceCell::new();

pub fn global(config: &Config) -> Arc<LocalAiService> {
    let runtime = crate::inference::local_runtime_config(config);
    LOCAL_AI
        .get_or_init(|| Arc::new(LocalAiService::new(&runtime)))
        .clone()
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
