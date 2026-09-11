use std::path::Path;

use futures_util::TryStreamExt;

use crate::openhuman::config::Config;
use crate::openhuman::inference::model_ids;
use tracing::{debug, trace};

use crate::openhuman::inference::local::provider::{provider_from_config, LocalAiProvider};
use crate::openhuman::inference::paths::{resolve_tts_voice_path, tts_model_target_path};
use crate::openhuman::inference::presets::{self, VisionMode};
use crate::openhuman::inference::types::{
    LocalAiAssetStatus, LocalAiAssetsStatus, LocalAiDownloadProgressItem, LocalAiDownloadsProgress,
};

use super::LocalAiService;
include!("assets_impl_01_part_01.rs");
include!("assets_impl_01_part_02.rs");
