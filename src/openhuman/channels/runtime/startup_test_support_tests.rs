use super::*;

pub fn resolve_yuanbao_app_secret_for_test(
    yb_cfg: crate::openhuman::channels::providers::yuanbao::YuanbaoConfig,
    config: &Config,
) -> crate::openhuman::channels::providers::yuanbao::YuanbaoConfig {
    resolve_yuanbao_app_secret(yb_cfg, config)
}
