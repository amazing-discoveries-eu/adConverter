use std::{
    env,
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use indicatif::ProgressBar;
use tauri::{AppHandle, Emitter};
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
    app: Option<AppHandle>,
    config: Config,
    child: Arc<Mutex<Option<Child>>>,
    is_running: Arc<AtomicBool>,
    mut cmd_logger: CommandLogger,
    source: &Path,
    task: &Task,
    progress_bar: Option<ProgressBar>,
) -> Result<(), ProcessError> {
    let progress_clone = progress_bar.clone();
    let app_clone = app.clone();
    let running_clone = is_running.clone();
    let mut transcript_cmd = config.transcript_cmd.clone();
    let lang = task.transcript.as_ref().map_or("auto", |v| v);
    let file_name = source.file_name().unwrap();
    let source_str = source.to_string_lossy().to_string();
    let temp_out = env::temp_dir().join(file_name).with_extension("vtt");
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
        && let Some(parent) = Path::new(&task.path).parent()
    {
        transcript_cmd =
            transcript_cmd.replace("%mount%", &format!("\"{}\"", parent.to_string_lossy()));
    };

    transcript_cmd = transcript_cmd.replace("%lang%", lang);

    transcript_cmd = transcript_cmd.replace("%file%", &format!("\"{source_str}\""));

    if transcript_cmd.contains("%output%") {
        transcript_cmd = transcript_cmd.replace("%output%", &format!("{:?}", env::temp_dir()));
    }

    let mut args = shlex::split(&transcript_cmd).ok_or("No transcript command to split")?;

    log_command("Transcript", None, args.clone());

    let program = args.remove(0);

    let mut cmd = Command::new(program);
    cmd.args(args).stderr(Stdio::piped()).stdout(Stdio::piped());

    #[cfg(target_os = "macos")]
    cmd.env("PATH", MACOS_PATH);

    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    if let Some(a) = &app {
        a.emit("transcript-start", 0).expect("Emit progress");
    }

    let mut proc = cmd.spawn()?;

    let stderr = proc.stderr.take().ok_or("Failed to capture stderr")?;
    let stdout = proc.stdout.take().ok_or("Failed to capture stdout")?;

    *child.lock().await = Some(proc);

    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buffer = Vec::new();

        while let Ok(bytes_read) = reader.read_until(b'\n', &mut buffer).await {
            if bytes_read == 0 {
                break; // EOF
            }

            if !is_running.load(Ordering::SeqCst) {
                break;
            }

            let log_line = String::from_utf8_lossy(&buffer);

            if log_line.contains("Transcription completed")
                && let Some(ref current) = progress_clone
            {
                current.set_position(100);
                current.finish_with_message("Transcription done...");
            }

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
                    .emit("transcript-progress", &progress)
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

    if let Some(proc) = child.lock().await.as_mut() {
        proc.wait().await?;
    }

    *child.lock().await = None;

    if temp_out.is_file() {
        let duration = (task.probe.clone().format_duration() * 1000.0) as u64;

        optimize_vtt(&temp_out, &output_path, duration, lang).await?;

        fs::remove_file(temp_out).await?;
    }

    if let Some(a) = &app {
        a.emit("transcript-finish", lang).expect("Emit progress");
    }

    Ok(())
}
