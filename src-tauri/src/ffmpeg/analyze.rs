use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::events::AppEvents;
use indicatif::ProgressBar;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};

use super::FFmpegProgress;
use crate::{
    Config, ProcessError,
    utils::logging::{CommandLogger, log_command},
    vec_strings,
};

#[cfg(target_os = "macos")]
use crate::MACOS_PATH;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Lufs {
    #[serde(deserialize_with = "deserialize_as_f64")]
    pub input_i: f64,
    #[serde(deserialize_with = "deserialize_as_f64")]
    pub input_lra: f64,
    #[serde(deserialize_with = "deserialize_as_f64")]
    pub input_tp: f64,
    #[serde(deserialize_with = "deserialize_as_f64")]
    pub input_thresh: f64,
    #[serde(deserialize_with = "deserialize_as_f64")]
    pub output_i: f64,
    #[serde(deserialize_with = "deserialize_as_f64")]
    pub output_lra: f64,
    #[serde(deserialize_with = "deserialize_as_f64")]
    pub output_tp: f64,
    #[serde(default)]
    pub target_i: f64,
    #[serde(default)]
    pub target_lra: f64,
    #[serde(default)]
    pub target_tp: f64,
    #[serde(default, deserialize_with = "deserialize_as_f64")]
    pub output_thresh: f64,
    pub normalization_type: String,
    #[serde(deserialize_with = "deserialize_as_f64")]
    pub target_offset: f64,
}

impl Lufs {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        app: Option<AppEvents>,
        config: &Config,
        duration: f64,
        is_running: Arc<AtomicBool>,
        child: Arc<Mutex<Option<Child>>>,
        src_cmd: Vec<String>,
        mut cmd_logger: CommandLogger,
        progress_bar: Option<ProgressBar>,
    ) -> Result<Self, ProcessError> {
        let progress_clone = progress_bar.clone();
        let running = is_running.clone();
        let running_clone = is_running.clone();
        let app_clone1 = app.clone();
        let log_ffmpeg_output = app.is_some();
        let lufs_c = config.lufs.clone();
        let lufs_stats = Arc::new(Mutex::new(Self {
            ..Default::default()
        }));
        let lufs_clone = lufs_stats.clone();
        let mut args = vec_strings![
            "-hide_banner",
            "-progress",
            "pipe:1",
            "-stats_period",
            "1",
            "-nostats",
            "-y",
            "-v"
        ];

        let ff_bin = config
            .ffmpeg_path
            .as_deref()
            .map(|p| p.join("ffmpeg"))
            .unwrap_or(PathBuf::from("ffmpeg"));

        args.push("level+info".to_string());
        args.extend(src_cmd);
        args.extend(vec_strings![
            "-vn",
            "-af",
            format!(
                "loudnorm=I={}:TP={}:LRA={}:print_format=json",
                lufs_c.i, lufs_c.tp, lufs_c.lra
            ),
            "-f",
            "null",
            "-"
        ]);

        log_command(
            "Analyze LUFS",
            Some(ff_bin.to_string_lossy().to_string()),
            args.clone(),
        );

        let mut cmd = Command::new(ff_bin);

        cmd.args(args).stderr(Stdio::piped()).stdout(Stdio::piped());

        #[cfg(target_os = "macos")]
        cmd.env("PATH", MACOS_PATH);

        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000);

        let mut proc = cmd.spawn()?;

        let stderr = proc.stderr.take().expect("Failed to capture stderr");
        let stdout = proc.stdout.take().expect("Failed to capture stdout");

        *child.lock().await = Some(proc);

        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            let mut stats = String::new();
            let mut is_object = false;

            while let Some(line) = reader.next_line().await.expect("Read line") {
                if !running.load(Ordering::SeqCst) {
                    break;
                }

                if log_ffmpeg_output || is_warning_or_error(&line) {
                    cmd_logger.log(Some("[ffmpeg]"), &line);
                }

                if line == "{" {
                    is_object = true;
                }

                if is_object {
                    stats.push_str(&line);
                }

                if line == "}" {
                    is_object = false;

                    *lufs_clone.lock().await =
                        serde_json::from_str(&stats).expect("Deserialize LUFS stats");
                }
            }
        });

        let mut stat_map = HashMap::new();
        stat_map.insert("title".to_string(), "LUFS".to_string());

        if let Some(ref current) = progress_bar {
            current.set_prefix("Analyze");
        }

        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();

            while let Some(line) = reader.next_line().await.expect("Read line") {
                if !running_clone.load(Ordering::SeqCst) {
                    break;
                }

                if let Some((key, value)) = line.split_once('=') {
                    stat_map.insert(key.trim().to_string(), value.trim().to_string());
                }

                if line.starts_with("progress") {
                    let progress = FFmpegProgress::new(&stat_map, duration);

                    stat_map.clear();
                    stat_map.insert("title".to_string(), "LUFS".to_string());

                    match &app_clone1 {
                        Some(app) => app.emit("lufs-progress", &progress).expect("Emit progress"),
                        None => {
                            if let Some(ref current) = progress_bar {
                                let msg = if progress.fps > 0.0 {
                                    format!("Analyzing LUFS {} FPS", progress.fps.round())
                                } else if progress.speed > 0.0 {
                                    format!("Analyzing LUFS {:.2} Speed", progress.speed)
                                } else {
                                    String::new()
                                };
                                current.set_message(msg);
                                current.set_position(progress.elapsed_pct);
                            }
                        }
                    }
                }
            }
        });

        stderr_task.await?;
        stdout_task.await?;

        if let Some(proc) = child.lock().await.as_mut() {
            proc.wait().await?;
        }

        if let Some(ref current) = progress_clone {
            current.finish_with_message("Analyzing done...");
        }

        *child.lock().await = None;
        let mut lufs = lufs_stats.lock().await.clone();
        lufs.target_i = lufs_c.i;
        lufs.target_tp = lufs_c.tp;
        lufs.target_lra = lufs_c.lra;

        Ok(lufs)
    }
}

fn is_warning_or_error(line: &str) -> bool {
    let line = line.to_lowercase();

    line.contains("warning") || line.contains("error") || line.contains("fatal")
}

fn deserialize_as_f64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    match Value::deserialize(deserializer)? {
        Value::String(s) => s.parse().map_err(|_| D::Error::custom("String to f64")),
        Value::Number(num) => num
            .as_f64()
            .ok_or_else(|| D::Error::custom("Number to f64")),
        _ => Err(D::Error::custom("Type mismatch")),
    }
}
