use clap::Parser;
use inquire::{Confirm, MultiSelect, Select};
use std::path::PathBuf;

use crate::{Config, ProcessError, collect_presets};

#[derive(Parser, Debug, Clone)]
#[clap(version,
    about = "adConverter CLI",
    long_about = None)]
pub struct Args {
    #[clap(
        long,
        help = "Use this configuration instead of global and user configuration"
    )]
    pub config: Option<PathBuf>,

    #[clap(long, help = "Ignore the optional system-wide configuration")]
    pub no_global_config: bool,

    #[clap(long, help = "Override the transcription model cache directory")]
    pub transcript_model_dir: Option<PathBuf>,

    #[clap(short, long, help = "Files to encode", num_args = 1..)]
    pub files: Vec<String>,

    #[clap(short, long, help = "Language to transcript")]
    pub lang: Option<String>,

    #[clap(long, help = "Fade in/out video and audio")]
    pub fade: Option<bool>,

    #[clap(long, help = "Apply loudnorm filter")]
    pub lufs: Option<bool>,

    #[clap(short, long, help = "Encoding presets", num_args = 0..)]
    pub presets: Option<Vec<String>>,
}

impl Args {
    pub async fn init(self, config: &Config) -> Result<Self, ProcessError> {
        let mut obj = self;

        obj.files = obj
            .files
            .into_iter()
            .map(|file| {
                let path = std::path::Path::new(&file);
                if path.is_relative() {
                    std::env::current_dir()
                        .unwrap_or_default()
                        .join(path)
                        .to_string_lossy()
                        .to_string()
                } else {
                    file
                }
            })
            .collect();

        let lang_list: Vec<String> = config
            .transcript_lang
            .iter()
            .map(|r| r.name.clone())
            .collect();

        let mut preset_list: Vec<String> = collect_presets(&None)
            .await?
            .iter()
            .map(|r| r.name.clone())
            .collect();

        preset_list.insert(0, "None".to_string());

        if obj.lang.is_none() {
            let lang = Select::new("Transcript Language:", lang_list).prompt()?;

            obj.lang = Some(lang);
        }

        if obj.fade.is_none() {
            let fade = Confirm::new("Apply fade [Y/n]:").prompt()?;

            obj.fade = Some(fade);
        }

        if obj.lufs.is_none() {
            let lufs = Confirm::new("Apply loudnorm [Y/n]:").prompt()?;

            obj.lufs = Some(lufs);
        }

        if obj.presets.is_none() {
            let presets = MultiSelect::new("Encoding presets:", preset_list).prompt()?;

            let presets = presets
                .into_iter()
                .filter(|p| p != "None")
                .collect::<Vec<String>>();

            obj.presets = Some(presets);
        }

        obj.lang = obj.lang.map(|lang| config.code_from(&lang));

        Ok(obj)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_flags_are_optional_and_parse_with_processing_options() {
        let defaults = Args::try_parse_from(["adconverter-cli"]).unwrap();
        assert!(defaults.config.is_none());
        assert!(!defaults.no_global_config);
        assert!(defaults.transcript_model_dir.is_none());

        let args = Args::try_parse_from([
            "adconverter-cli",
            "--config",
            "/custom/config.json",
            "--no-global-config",
            "--transcript-model-dir",
            "/shared models",
            "--files",
            "video.mp4",
            "--lang",
            "de",
            "--fade",
            "false",
            "--lufs",
            "false",
            "--presets",
        ])
        .unwrap();
        assert_eq!(args.config, Some(PathBuf::from("/custom/config.json")));
        assert!(args.no_global_config);
        assert_eq!(
            args.transcript_model_dir,
            Some(PathBuf::from("/shared models"))
        );
        assert_eq!(args.lang.as_deref(), Some("de"));
        assert_eq!(args.presets, Some(vec![]));
    }
}
