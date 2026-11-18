use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::{NoneAsEmptyString, serde_as};
use ts_rs::TS;

pub mod cli;
#[cfg(feature = "desktop")]
mod desktop;
mod events;
mod ffmpeg;
mod macros;
mod publisher;
mod transcript;
mod utils;

#[cfg(feature = "desktop")]
pub use desktop::run;
use ffmpeg::{probe::MediaProbe, worker};
pub use publisher::Publish;
pub use utils::{
    Sources, delete_files,
    errors::ProcessError,
    logging::init_logging,
    presets::{Preset, collect_presets},
    template::Template,
};
#[cfg(feature = "desktop")]
pub use utils::{copy_assets, download, update};

#[cfg(target_os = "macos")]
const MACOS_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[ts(export, export_to = "backend.d.ts")]
pub struct Task {
    #[serde(default)]
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub url: Option<String>,
    pub r#in: f64,
    pub out: f64,
    pub fade: bool,
    pub lufs: bool,
    #[serde(default)]
    pub transcript: Option<String>,
    #[serde(default)]
    pub probe: MediaProbe,
    pub presets: Vec<Preset>,
    #[serde(default)]
    pub template: Option<Template>,
    #[ts(type = "string")]
    #[serde_as(as = "NoneAsEmptyString")]
    pub target: Option<String>,
    #[serde(default)]
    pub target_subfolder: bool,
    #[serde(default)]
    pub publish: Option<Publish>,
    #[ts(type = "bool")]
    pub active: Arc<AtomicBool>,
    #[ts(type = "bool")]
    pub finished: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export, export_to = "backend.d.ts")]
pub struct Config {
    pub copyright: String,
    pub ffmpeg_path: Option<PathBuf>,
    #[serde(default)]
    pub download_path: Option<PathBuf>,
    #[serde(default)]
    pub download_args: String,
    #[serde(default)]
    pub yt_dlp_path: Option<PathBuf>,
    pub lufs: LufsConfig,
    pub transcript_cmd: String,
    pub transcript_lang: Vec<LangConfig>,
    #[serde(default)]
    pub publish_preset: Option<String>,
    #[serde(default)]
    pub publisher: Option<Value>,
}

impl Config {
    pub fn code_from(&self, lang: &str) -> String {
        self.transcript_lang
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(lang) || l.code.eq_ignore_ascii_case(lang))
            .map(|l| l.code.clone())
            .unwrap_or_else(|| lang.to_string())
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export, export_to = "backend.d.ts")]
pub struct LangConfig {
    pub name: String,
    pub code: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[ts(export, export_to = "backend.d.ts")]
pub struct LufsConfig {
    pub i: f64,
    pub lra: f64,
    pub tp: f64,
}
