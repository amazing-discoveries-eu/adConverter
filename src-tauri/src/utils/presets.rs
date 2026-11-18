use std::{
    env,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::serde_as;
use tokio::{fs, io::AsyncWriteExt};
use ts_rs::TS;

use crate::{ProcessError, cli::IDENTIFIER};

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[ts(export, export_to = "backend.d.ts")]
pub struct Preset {
    pub name: String,
    pub title: String,
    pub tooltip: String,
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lufs: Option<bool>,
    pub filter_video: Value,
    pub filter_audio: Value,
    pub video: Value,
    pub audio: Value,
    #[ts(type = "string")]
    #[serde(default)]
    pub container_video: Option<String>,
    #[ts(type = "string")]
    #[serde(default)]
    pub container_audio: Option<String>,
    #[ts(type = "string")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<PathBuf>,
    #[ts(type = "bool")]
    #[serde(skip_serializing, skip_deserializing)]
    pub finished: Arc<AtomicBool>,
}

impl Preset {
    pub async fn save(&mut self, path: &Path) -> Result<(), ProcessError> {
        let preset_path = path.join(format!("{}.json", self.name));
        self.output_path = None;

        let json = serde_json::to_string_pretty(&self)?;
        let mut file = fs::File::create(preset_path).await?;
        file.write_all(json.as_bytes()).await?;

        Ok(())
    }
}

pub fn preset_path(app: &Option<PathBuf>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut directory = if cfg!(debug_assertions) {
        env::current_dir()?.join("assets")
    } else if let Some(a) = app {
        a.clone()
    } else {
        dirs::data_dir()
            .unwrap_or(env::current_dir()?)
            .join(IDENTIFIER)
    };

    directory = directory.join("presets");

    Ok(directory)
}

pub async fn collect_presets(
    app: &Option<PathBuf>,
) -> Result<Vec<Preset>, Box<dyn std::error::Error>> {
    let path = preset_path(app)?;
    let mut entries = fs::read_dir(path).await?;
    let mut presets = vec![];

    while let Some(entry) = entries.next_entry().await? {
        let extension = entry
            .path()
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();

        if extension == "json" {
            let contents = fs::read_to_string(entry.path()).await?;
            let preset: Preset = serde_json::from_str(&contents)?;

            presets.push(preset);
        }
    }

    presets.sort_by_key(|a| a.name.to_lowercase());

    Ok(presets)
}
