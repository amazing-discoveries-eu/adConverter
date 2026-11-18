use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::events::AppEvents;
use chrono::{Datelike, Local};
use indicatif::ProgressBar;
use serde_json::Value;
use shlex::split;
use tokio::{
    fs,
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};

use super::{FFmpegProgress, analyze::Lufs, filter::filter_chain, probe::MediaProbe};
use crate::{
    Config,
    utils::{
        Sources,
        logging::{CommandLogger, log_command},
    },
};
use crate::{ProcessError, Task, transcript, vec_strings};

#[cfg(target_os = "macos")]
use crate::MACOS_PATH;

const IGNORE_LINES: &[&str] = &[
    "SEI type 1 size",
    "Late SEI is not implemented",
    "If you want to help, upload a sample of this file to",
];

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.is_empty(),
        Value::Array(arr) => arr.is_empty(),
        _ => false,
    }
}

fn to_vec(value: Value) -> Vec<String> {
    let mut params = Vec::new();

    if let Value::Object(map) = value {
        for (key, val) in map {
            params.push(key);

            match val {
                Value::String(s) => params.push(s),
                _ => params.push(val.to_string()),
            }
        }
    }

    params
}

async fn calc_duration(config: &Config, task: &Task) -> (f64, f64, f64) {
    let mut duration_intro = 0.0;
    let mut duration_outro = 0.0;

    if let Some(template) = &task.template {
        if let Some(intro) = &template.intro
            && let Ok(probe) = MediaProbe::new(config, &intro).await
        {
            duration_intro = probe.format_duration();
        };

        if let Some(outro) = &template.outro
            && let Ok(probe) = MediaProbe::new(config, &outro).await
        {
            duration_outro = probe.format_duration();
        };
    }

    let mut duration = task.probe.format.duration.unwrap_or_default();

    match (task.r#in > 0.0, task.out > 0.0) {
        (true, true) => duration = (task.out - task.r#in).max(0.0),
        (true, false) => duration = (duration - task.r#in).max(0.0),
        (false, true) => duration = (duration - task.out).max(0.0),
        _ => {}
    }

    (duration_intro, duration, duration_outro)
}

pub async fn work(
    app: Option<AppEvents>,
    config: Config,
    child: Arc<Mutex<Option<Child>>>,
    is_running: Arc<AtomicBool>,
    task: Task,
    progress_bar: Option<ProgressBar>,
) -> Result<(), ProcessError> {
    let app_some = app.is_some();
    let mut task_clone = task.clone();
    // let mut presets = mem::take(&mut task_clone.presets);
    let sources = Sources::new(&task.path).await;
    let path = Path::new(&task.path);
    let (intro_dur, video_dur, outro_dur) = calc_duration(&config, &task).await;
    let duration = intro_dur + video_dur + outro_dur;
    let year = Local::now().year();
    let cmd_logger = CommandLogger::new();
    let mut audio_pos = -1;
    let mut has_audio = !task.probe.audio.is_empty();
    let mut has_video = false;

    let mut task_args = vec_strings![
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

    if app.is_some() {
        task_args.push("level+info".to_string());
    } else {
        task_args.push("level+warning".to_string());
    }

    let seek = if task.r#in > 0.0 {
        vec_strings!["-ss", task.r#in]
    } else {
        vec![]
    };

    let length = if task.out > 0.0 {
        vec_strings!["-t", task.out - task.r#in]
    } else {
        vec![]
    };

    let mut audio_path = String::new();

    if let Ok(src) = &sources {
        if let Some(video) = &src.video {
            audio_path = video.clone();
            audio_pos += 1;
            task_args.extend(seek.clone());
            task_args.extend(vec_strings!["-i", video]);
            task_args.extend(length.clone());
        }

        if let Some(audio) = &src.audio {
            audio_path = audio.clone();
            audio_pos += 1;
            has_audio = true;
            task_args.extend(seek.clone());
            task_args.extend(vec_strings!["-i", audio]);
            task_args.extend(length.clone());
        }
    }

    let mut transcript_src = Some(PathBuf::from(&audio_path));

    let lufs = if task.lufs {
        let mut src_cmd = seek.clone();
        src_cmd.extend(vec_strings!["-i", audio_path]);
        src_cmd.extend(length);

        Lufs::new(
            app.clone(),
            &config,
            video_dur,
            is_running.clone(),
            child.clone(),
            src_cmd,
            cmd_logger.clone(),
            progress_bar.clone(),
        )
        .await?
    } else {
        Lufs::default()
    };

    if !is_running.load(Ordering::SeqCst) {
        return Ok(());
    }

    task_args.extend(vec_strings![
        "-map_chapters",
        "-1",
        "-map_metadata",
        "-1",
        "-metadata",
        format!("year={year}")
    ]);

    if !config.copyright.is_empty() {
        task_args.extend(vec_strings![
            "-metadata",
            format!("copyright={}", config.copyright)
        ]);
    }

    for i in 0..task_clone.presets.len() {
        let preset = task_clone.presets[i].clone();
        let mut args = task_args.clone();
        let running = is_running.clone();
        let running_clone = is_running.clone();
        let app_clone1 = app.clone();
        let title = preset.title.clone();
        let finished = preset.finished.clone();
        let mut cmd_logger = cmd_logger.clone();
        let progress_clone = progress_bar.clone();
        let ff_bin = ff_bin.clone();

        let parent_path = path.parent().expect("Path should have a parent");
        let file_stem = path
            .file_stem()
            .ok_or("Path should have a valid file stem")?
            .to_string_lossy();
        let extension = preset
            .container_video
            .clone()
            .or(preset.container_audio.clone())
            .unwrap_or_default();

        if let Some(input_args) = &preset.input {
            let i_args = split(input_args).unwrap_or_default();

            if let Some(pos) = args.iter().position(|x| x == "-i") {
                // Insert before input "-i"
                args.splice(pos..pos, i_args);
            }
        }

        let file_name = format!("{} # {}.{}", file_stem, preset.title, extension);

        let output = match task.target.as_ref() {
            Some(target) => {
                let mut op = PathBuf::from(target);

                if task.target_subfolder {
                    let sub = parent_path
                        .file_name()
                        .expect("Parent path should have a file name");
                    op = op.join(sub);

                    fs::create_dir_all(&op).await?;
                }

                op.join(&file_name)
            }
            None => parent_path.join(&file_name),
        };

        if transcript_src == Some(PathBuf::from(&audio_path))
            && has_audio
            && !is_empty(&preset.audio)
        {
            transcript_src = Some(
                preset
                    .container_audio
                    .as_ref()
                    .map(|audio_ext| output.with_extension(audio_ext))
                    .unwrap_or_else(|| output.clone()),
            );
        }

        let temp_out = env::temp_dir().join(&file_name);
        task_clone.presets[i].output_path = Some(output.clone());

        if sources.as_ref().map(|s| s.video.clone()).is_ok()
            && let Value::Object(map) = &preset.video
            && !map.is_empty()
        {
            has_video = true;
        }

        let mut filter = filter_chain(
            &config, &task, &preset, &lufs, has_audio, has_video, audio_pos,
        )
        .await;
        args.extend(filter.cmd());

        if let Some(video_ext) = &preset.container_video {
            if has_video {
                args.extend(filter.map_video());
                args.extend(to_vec(preset.video.clone()));
            }

            if video_ext.eq_ignore_ascii_case("mp4") {
                args.extend(vec_strings!["-movflags", "+faststart"]);
            }
        }

        if let Some(audio_ext) = &preset.container_audio {
            if has_video {
                args.push(temp_out.to_string_lossy().to_string());
            }

            if has_audio {
                args.extend(filter.map_audio());
                args.extend(to_vec(preset.audio.clone()));
            }

            args.push(
                temp_out
                    .with_extension(audio_ext)
                    .to_string_lossy()
                    .to_string(),
            );
        } else {
            if has_audio {
                args.extend(filter.map_audio());
                args.extend(to_vec(preset.audio.clone()));
            }

            if has_video {
                args.push(temp_out.to_string_lossy().to_string());
            }
        }

        log_command(
            &format!("Preset: {}", preset.title),
            Some(ff_bin.to_string_lossy().to_string()),
            args.clone(),
        );

        if let Some(a) = &app {
            a.emit("preset-start", &preset).expect("Emit Preset");
        }

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

            while let Some(line) = reader.next_line().await.expect("Read line") {
                if !running.load(Ordering::SeqCst) {
                    break;
                }

                if !IGNORE_LINES.iter().any(|&s| line.contains(s))
                    && !line.is_empty()
                    && !line.starts_with("encoded")
                    && (app_some || !line.contains("[info]"))
                {
                    cmd_logger.log(Some("[ffmpeg]"), &line);
                }
            }
        });

        let mut stat_map = HashMap::new();
        stat_map.insert("title".to_string(), title.clone());

        if let Some(ref current) = progress_clone {
            current.set_prefix("Encode ");
        }

        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();

            while let Some(line) = reader.next_line().await.expect("Read line") {
                if !running_clone.load(Ordering::SeqCst) {
                    break;
                }

                let mut process = String::new();

                if let Some((key, value)) = line.split_once('=') {
                    process = value.trim().to_string();

                    stat_map.insert(key.trim().to_string(), process.clone());
                }

                if line.starts_with("progress") {
                    let progress = FFmpegProgress::new(&stat_map, duration);

                    stat_map.clear();
                    stat_map.insert("title".to_string(), title.clone());

                    if &process != "end" {
                        match &app_clone1 {
                            Some(a) => a.emit("preset-progress", &progress).expect("Emit progress"),

                            None => {
                                if let Some(ref current) = progress_clone {
                                    let msg = if progress.fps > 0.0 {
                                        format!("{} FPS", progress.fps)
                                    } else if progress.speed > 0.0 {
                                        format!("{} Speed", progress.speed)
                                    } else {
                                        String::new()
                                    };
                                    current.set_message(msg);
                                    current.set_position(progress.elapsed_pct);
                                }
                            }
                        };
                    } else if let Some(ref current) = progress_clone {
                        current.finish_with_message("done...");
                    }
                }
            }
        });

        stderr_task.await?;
        stdout_task.await?;

        if let Some(proc) = child.lock().await.as_mut() {
            proc.wait().await?;
        }

        let mut output_files = Vec::with_capacity(2);

        if has_video && preset.container_video.is_some() {
            output_files.push((temp_out.clone(), output.clone()));
        }

        if has_audio && let Some(audio_ext) = &preset.container_audio {
            output_files.push((
                temp_out.with_extension(audio_ext),
                output.with_extension(audio_ext),
            ));
        }

        for (temp_file, output_file) in output_files {
            fs::copy(&temp_file, &output_file).await?;
            fs::remove_file(temp_file).await?;
        }

        finished.store(true, Ordering::SeqCst);

        if let Some(a) = &app {
            a.emit("preset-finish", &preset).expect("Emit progress");
        }
    }

    if let Some(src) = transcript_src
        && task
            .transcript
            .as_ref()
            .is_some_and(|lang| lang.to_lowercase() != "none")
    {
        transcript::run(
            app.clone(),
            config,
            child.clone(),
            is_running.clone(),
            cmd_logger.clone(),
            &src,
            &task,
            progress_bar,
        )
        .await?;
    }

    *child.lock().await = None;

    // if task.publish.is_some() && is_running.load(Ordering::SeqCst) {
    //     publisher::peertube::publish(app, &task_clone, is_running).await?;
    // }

    Ok(())
}
