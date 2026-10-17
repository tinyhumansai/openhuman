use super::*;

use crate::config::Config;
pub fn resolve_yuanbao_app_secret_for_test(
    yb_cfg: tinychannels_bus::config::YuanbaoConfig,
    config: &Config,
) -> tinychannels_bus::config::YuanbaoConfig {
    resolve_yuanbao_app_secret(yb_cfg, config)
}
