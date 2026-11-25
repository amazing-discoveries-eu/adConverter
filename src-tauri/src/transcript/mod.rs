use std::{
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::events::AppEvents;
use indicatif::ProgressBar;
use tokio::{
    fs,
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};

mod process;

use crate::{
    Config, ProcessError, Task,
    transcript::process::optimize_vtt,
    utils::logging::{CommandLogger, log_command},
};

#[cfg(target_os = "macos")]
use crate::MACOS_PATH;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    app: Option<AppEvents>,
    config: Config,
    child: Arc<Mutex<Option<Child>>>,
    is_running: Arc<AtomicBool>,
    mut cmd_logger: CommandLogger,
    source: &Path,
    task: &Task,
    progress_bar: Option<ProgressBar>,
) -> Result<(), ProcessError> {
    let completion_progress = progress_bar.clone();
    let app_clone = app.clone();
    let stderr_running = is_running.clone();
    let running_clone = is_running.clone();
    let mut transcript_cmd = config.transcript_cmd.clone();
    let lang = task.transcript.as_ref().map_or("auto", |v| v);
    let file_name = source
        .file_name()
        .ok_or_else(|| ProcessError::Custom("Transcription source has no file name".into()))?;
    let source_str = source.to_string_lossy().to_string();
    let output_directory = tempfile::Builder::new()
        .prefix("adconverter-transcript-")
        .tempdir()?;
    let temp_out = output_directory
        .path()
        .join(file_name)
        .with_extension("vtt");
    let output_path = match &task.target {
        Some(p) => Path::new(p).join(file_name).with_extension("vtt"),
        None => source.with_extension("vtt"),
    };

    #[cfg(target_os = "windows")]
    let mut source_str = source_str;

    #[cfg(target_os = "windows")]
    {
        transcript_cmd = transcript_cmd.replace("\\", "\\\\");
        source_str = source_str.replace("\\", "\\\\");
    }

    if transcript_cmd.contains("%mount%")
        && let Some(parent) = source.parent()
    {
        transcript_cmd =
            transcript_cmd.replace("%mount%", &format!("\"{}\"", parent.to_string_lossy()));
    };

    transcript_cmd = transcript_cmd.replace("%lang%", lang);

    transcript_cmd = transcript_cmd.replace("%file%", &format!("\"{source_str}\""));

    if transcript_cmd.contains("%output%") {
        transcript_cmd =
            transcript_cmd.replace("%output%", &format!("{:?}", output_directory.path()));
    } else if !transcript_cmd.trim().is_empty() {
        return Err(ProcessError::Custom(
            "The transcription command must use %output% as its output directory".into(),
        ));
    }

    let mut args = shlex::split(&transcript_cmd).ok_or("No transcript command to split")?;
    if args.is_empty() {
        return Err(ProcessError::Custom(
            "No transcription command configured; set transcript_cmd in the CLI configuration"
                .into(),
        ));
    }

    log_command("Transcript", None, args.clone());

    let program = args.remove(0);

    let mut cmd = Command::new(program);
    cmd.args(args)
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    if let Some(directory) = &config.transcript_model_dir {
        cmd.env("ADCONVERTER_TRANSCRIPT_MODEL_DIR", directory);
        cmd.env("HF_HUB_CACHE", directory);
    }

    #[cfg(target_os = "macos")]
    cmd.env("PATH", MACOS_PATH);

    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    if let Some(a) = &app {
        a.emit("transcript-start", 0).expect("Emit progress");
    }

    let mut process = child.lock().await;
    if !is_running.load(Ordering::SeqCst) {
        return Ok(());
    }
    let mut proc = cmd.spawn()?;

    let stderr = proc.stderr.take().ok_or("Failed to capture stderr")?;
    let stdout = proc.stdout.take().ok_or("Failed to capture stdout")?;

    *process = Some(proc);
    drop(process);

    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buffer = Vec::new();

        while let Ok(bytes_read) = reader.read_until(b'\n', &mut buffer).await {
            if bytes_read == 0 {
                break; // EOF
            }

            if !stderr_running.load(Ordering::SeqCst) {
                break;
            }

            let log_line = String::from_utf8_lossy(&buffer);

            cmd_logger.log(Some("[transcript]"), log_line.trim());
            buffer.clear();
        }
    });

    let mut set_prefix = true;

    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut buffer = Vec::new();

        while let Ok(bytes_read) = reader.read_until(b'\n', &mut buffer).await {
            if bytes_read == 0 {
                break; // EOF
            }

            if !running_clone.load(Ordering::SeqCst) {
                break;
            }

            let progress = String::from_utf8_lossy(&buffer)
                .trim()
                .parse::<u64>()
                .unwrap_or_default();

            match &app_clone {
                Some(a) => a
                    .emit("transcript-progress", progress)
                    .expect("Emit progress"),
                None => {
                    if let Some(ref current) = progress_bar {
                        if set_prefix {
                            current.set_prefix("Dictate");
                            current.set_message("Transcribe audio");
                            set_prefix = false;
                        }

                        if progress < 100 {
                            current.set_position(progress);
                        }
                    }
                }
            };

            buffer.clear();
        }
    });

    stderr_task.await?;
    stdout_task.await?;

    let process = child.lock().await.take();
    let status = match process {
        Some(mut proc) => Some(proc.wait().await?),
        None => None,
    };
    if !is_running.load(Ordering::SeqCst) {
        return Ok(());
    }
    let status =
        status.ok_or_else(|| ProcessError::Custom("Transcription process disappeared".into()))?;
    if !status.success() {
        return Err(ProcessError::Custom(format!(
            "Transcription command failed with {status}"
        )));
    }
    let metadata = fs::metadata(&temp_out).await.map_err(|error| {
        ProcessError::Custom(format!(
            "Transcription did not produce {}: {error}",
            temp_out.display()
        ))
    })?;
    if metadata.len() == 0 || !metadata.is_file() {
        return Err(ProcessError::Custom(
            "Transcription produced an empty or invalid VTT file".into(),
        ));
    }
    let mut header = String::new();
    BufReader::new(fs::File::open(&temp_out).await?)
        .read_line(&mut header)
        .await?;
    let header = header.trim_start_matches('\u{feff}').trim_end();
    if header != "WEBVTT" && !header.starts_with("WEBVTT ") && !header.starts_with("WEBVTT\t") {
        return Err(ProcessError::Custom(
            "Transcription output has no valid WEBVTT header".into(),
        ));
    }
    let duration = (task.probe.clone().format_duration() * 1000.0) as u64;
    let parent = output_path
        .parent()
        .ok_or_else(|| ProcessError::Custom("Subtitle output has no parent directory".into()))?;
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    if let Ok(metadata) = fs::metadata(&output_path).await {
        builder.permissions(metadata.permissions());
    }
    let pending = builder.tempfile_in(parent)?.into_temp_path();
    optimize_vtt(&temp_out, &pending, duration, lang).await?;
    if !is_running.load(Ordering::SeqCst) {
        return Ok(());
    }
    pending.persist(&output_path).map_err(|error| error.error)?;

    if let Some(current) = completion_progress {
        current.finish_with_message("Transcription done...");
    }

    if let Some(a) = &app {
        a.emit("transcript-finish", lang).expect("Emit progress");
    }

    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    async fn transcribe(source: &Path, script: &Path) -> Result<(), ProcessError> {
        let child = Arc::new(Mutex::new(None));
        let config = Config {
            transcript_cmd: format!("/bin/sh {:?} %output% %file%", script),
            ..Config::default()
        };
        let result = run(
            None,
            config,
            child.clone(),
            Arc::new(AtomicBool::new(true)),
            CommandLogger::new(),
            source,
            &Task::default(),
            None,
        )
        .await;
        assert!(child.lock().await.is_none());
        result
    }

    fn make_source(directory: &Path) -> std::path::PathBuf {
        std::fs::create_dir_all(directory).unwrap();
        let source = directory.join("video.mp4");
        std::fs::write(
            &source,
            b"fake media; the test transcriber does not decode it",
        )
        .unwrap();
        source
    }

    #[tokio::test]
    async fn mount_follows_encoded_source_and_cancelled_jobs_do_not_start() {
        let root = tempfile::tempdir().unwrap();
        let source = make_source(&root.path().join("encoding destination"));
        let script = root.path().join("transcribe.sh");
        std::fs::write(
            &script,
            "[ \"$1\" = \"${3%/*}:${3%/*}\" ] || exit 7\nprintf 'WEBVTT\\n\\n00:00:00.000 --> 00:00:01.000\\nTest\\n' > \"$2/video.vtt\"\n",
        ).unwrap();
        let task = Task {
            path: root
                .path()
                .join("original/video.mp4")
                .to_string_lossy()
                .into_owned(),
            ..Task::default()
        };
        let config = Config {
            transcript_cmd: format!("/bin/sh {:?} %mount%:%mount% %output% %file%", script),
            ..Config::default()
        };
        for running in [false, true] {
            run(
                None,
                config.clone(),
                Arc::new(Mutex::new(None)),
                Arc::new(AtomicBool::new(running)),
                CommandLogger::new(),
                &source,
                &task,
                None,
            )
            .await
            .unwrap();
            assert_eq!(source.with_extension("vtt").exists(), running);
        }
    }

    #[tokio::test]
    async fn failed_transcriber_preserves_existing_subtitles_and_cleans_temporary_files() {
        let root = tempfile::tempdir().unwrap();
        let source = make_source(root.path());
        let script = root.path().join("fail.sh");
        std::fs::write(source.with_extension("vtt"), "previous subtitles").unwrap();
        std::fs::write(&script, "printf '%s' \"$1\" > \"$2.scratch\"\nprintf 'partial result' > \"$1/video.vtt\"\nexit 7\n").unwrap();

        let error = transcribe(&source, &script).await.unwrap_err();
        assert!(error.to_string().contains("exit status: 7"));
        assert_eq!(
            std::fs::read_to_string(source.with_extension("vtt")).unwrap(),
            "previous subtitles"
        );
        let scratch = std::fs::read_to_string(source.with_extension("mp4.scratch")).unwrap();
        assert!(!Path::new(&scratch).exists());
    }

    #[tokio::test]
    async fn missing_and_empty_outputs_are_errors_even_after_exit_zero() {
        for body in [
            "exit 0\n",
            "touch \"$1/video.vtt\"\n",
            "printf 'not VTT' > \"$1/video.vtt\"\n",
        ] {
            let root = tempfile::tempdir().unwrap();
            let source = make_source(root.path());
            let script = root.path().join("missing.sh");
            std::fs::write(&script, body).unwrap();
            assert!(transcribe(&source, &script).await.is_err());
            assert!(!source.with_extension("vtt").exists());
        }
    }

    #[tokio::test]
    async fn parallel_transcriptions_with_the_same_basename_keep_their_own_output() {
        let root = tempfile::tempdir().unwrap();
        let first = make_source(&root.path().join("first input"));
        let second = make_source(&root.path().join("second input's folder"));
        let script = root.path().join("success.sh");
        std::fs::write(&script, "printf '%s' \"$1\" > \"$2.scratch\"\nsleep 0.05\nprintf 'WEBVTT\\n\\n00:00:00.000 --> 00:00:01.000\\n%s\\n' \"$2\" > \"$1/video.vtt\"\n").unwrap();
        let (a, b) = tokio::join!(transcribe(&first, &script), transcribe(&second, &script));
        a.unwrap();
        b.unwrap();
        let mut directories = Vec::new();
        for source in [&first, &second] {
            let output = std::fs::read_to_string(source.with_extension("vtt")).unwrap();
            assert!(output.contains(source.to_str().unwrap()));
            let directory = std::fs::read_to_string(source.with_extension("mp4.scratch")).unwrap();
            assert!(!Path::new(&directory).exists());
            directories.push(directory);
        }
        assert_ne!(directories[0], directories[1]);
    }
}
