use std::sync::{Arc, atomic::AtomicBool};

use dirs::data_dir;

use crate::{
    Config, Task,
    cli::{IDENTIFIER, args::Args},
    collect_presets,
    ffmpeg::probe::MediaProbe,
    utils::errors::ProcessError,
};

pub fn read_config(args: &Args) -> Result<Config, ProcessError> {
    let config_path = data_dir()
        .expect("Data dir")
        .join(IDENTIFIER)
        .join("config.json");

    let mut config = super::config::load(
        &super::config::global_path(),
        &config_path,
        args.config.as_deref(),
        args.no_global_config,
    )?;
    if let Some(path) = &args.transcript_model_dir {
        config.transcript_model_dir = Some(path.clone());
    }

    if let Some(ffmpeg_path) = &config.ffmpeg_path {
        if ffmpeg_path.is_file() {
            config.ffmpeg_path = ffmpeg_path.parent().map(std::path::Path::to_path_buf);
        } else if !ffmpeg_path.is_dir() {
            config.ffmpeg_path = None;
        }
    }

    Ok(config)
}

pub async fn create_tasks(config: &Config, args: Args) -> Vec<Task> {
    let preset_list = collect_presets(&None).await.unwrap_or_default();
    let selected_presets = args.presets.unwrap_or_default();
    let mut presets = vec![];
    let mut tasks = vec![];

    for preset in preset_list {
        if selected_presets.contains(&preset.name) {
            presets.push(preset);
        }
    }

    for (index, file) in args.files.into_iter().enumerate() {
        let task = Task {
            id: format!("cli-{index}"),
            path: file.clone(),
            url: None,
            r#in: 0.0,
            out: 0.0,
            fade: args.fade.unwrap_or_default(),
            lufs: args.lufs.unwrap_or_default(),
            transcript: args.lang.clone(),
            probe: MediaProbe::new(config, file).await.unwrap_or_default(),
            presets: presets.clone(),
            template: None,
            target: None,
            target_subfolder: false,
            publish: None,
            active: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
        };
        tasks.push(task);
    }

    tasks
}
