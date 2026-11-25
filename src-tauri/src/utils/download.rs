use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use log::debug;
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};

use crate::utils::errors::ProcessError;

const OUTPUT_MARKER: &str = "__ADCONVERTER_OUTPUT__";

pub async fn version(path: Option<PathBuf>) -> Result<String, ProcessError> {
    version_from_program(program(path)).await
}

async fn version_from_program(program: PathBuf) -> Result<String, ProcessError> {
    let mut command = Command::new(program);
    command.arg("--version");

    #[cfg(target_os = "macos")]
    command.env("PATH", crate::MACOS_PATH);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);

    let output = command.output().await.map_err(|error| {
        ProcessError::Custom(format!(
            "yt-dlp was not found. Install it and make sure it is in PATH: {error}"
        ))
    })?;

    if !output.status.success() {
        return Err(ProcessError::Custom(format!(
            "yt-dlp could not be executed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Download one media URL and report yt-dlp's percentage to the webview.
pub async fn download(
    app: AppHandle,
    url: String,
    directory: PathBuf,
    arguments: &str,
    yt_dlp_path: Option<PathBuf>,
    process: Arc<Mutex<Option<Child>>>,
    is_running: &AtomicBool,
) -> Result<String, ProcessError> {
    tokio::fs::create_dir_all(&directory).await?;

    let arguments = parse_arguments(arguments)?;
    eprintln!("[yt-dlp] starting download for {url} with arguments: {arguments:?}");
    let mut command = Command::new(program(yt_dlp_path));
    command
        .kill_on_drop(true)
        .args(arguments)
        .args(["--no-playlist", "--newline", "--progress"])
        .arg("--print")
        .arg(format!("after_move:{OUTPUT_MARKER}%(filepath)s"))
        .arg(&url)
        .current_dir(&directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "macos")]
    command.env("PATH", crate::MACOS_PATH);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);

    let mut slot = process.lock().await;
    if !is_running.load(Ordering::SeqCst) {
        return Err(ProcessError::Custom("yt-dlp download was cancelled".into()));
    }
    let mut child = command.spawn().map_err(|error| {
        ProcessError::Custom(format!(
            "Could not start yt-dlp. Install it and make sure it is in PATH: {error}"
        ))
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ProcessError::Custom("Could not read yt-dlp output".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ProcessError::Custom("Could not read yt-dlp error output".into()))?;
    let mut stdout = BufReader::new(stdout).lines();
    let mut stderr = BufReader::new(stderr).lines();
    *slot = Some(child);
    drop(slot);
    let mut downloaded_file = None;
    let mut error_output = Vec::new();
    let mut stdout_open = true;
    let mut stderr_open = true;

    while stdout_open || stderr_open {
        tokio::select! {
            line = stdout.next_line(), if stdout_open => match line? {
                Some(line) => process_line(&app, &line, &mut downloaded_file),
                None => stdout_open = false,
            },
            line = stderr.next_line(), if stderr_open => match line? {
                Some(line) => {
                    debug!("yt-dlp: {line}");
                    eprintln!("[yt-dlp] {line}");
                    process_line(&app, &line, &mut downloaded_file);
                    if !line.trim().is_empty() {
                        error_output.push(line);
                    }
                }
                None => stderr_open = false,
            },
        }
    }

    let status = match process.lock().await.take() {
        Some(mut child) => child.wait().await?,
        None => return Err(ProcessError::Custom("yt-dlp download was cancelled".into())),
    };
    if !status.success() {
        let details = error_output
            .into_iter()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(ProcessError::Custom(format!(
            "yt-dlp exited with status {status}: {details}"
        )));
    }

    downloaded_file.ok_or_else(|| {
        ProcessError::Custom("yt-dlp did not return the downloaded file path".into())
    })
}

fn program(path: Option<PathBuf>) -> PathBuf {
    match path {
        Some(path) if path.is_dir() => path.join("yt-dlp"),
        Some(path) => path,
        None => PathBuf::from("yt-dlp"),
    }
}

fn process_line(app: &AppHandle, line: &str, downloaded_file: &mut Option<String>) {
    if let Some(path) = output_path(line) {
        *downloaded_file = Some(path.to_string());
    }

    // yt-dlp's --newline output contains values such as "[download]  42.1%".
    if let Some(percent) = progress_percent(line) {
        let _ = app.emit("download-progress", percent);
    }
}

fn parse_arguments(arguments: &str) -> Result<Vec<String>, ProcessError> {
    shlex::split(arguments)
        .ok_or_else(|| ProcessError::Custom("Invalid quotes in yt-dlp parameters".into()))
}

fn output_path(line: &str) -> Option<&str> {
    line.strip_prefix(OUTPUT_MARKER)
}

fn progress_percent(line: &str) -> Option<f64> {
    line.split_whitespace()
        .find_map(|part| part.strip_suffix('%'))
        .and_then(|percent| percent.parse::<f64>().ok())
        .map(|percent| percent.clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_arguments() {
        assert_eq!(
            parse_arguments("--output \"%(title)s.%(ext)s\" --format best").unwrap(),
            ["--output", "%(title)s.%(ext)s", "--format", "best"]
        );
    }

    #[test]
    fn rejects_unclosed_argument_quotes() {
        assert!(parse_arguments("--output \"unfinished").is_err());
    }

    #[test]
    fn parses_and_clamps_progress() {
        assert_eq!(progress_percent("[download]  42.1% of 10MiB"), Some(42.1));
        assert_eq!(progress_percent("[download] 120.0%"), Some(100.0));
        assert_eq!(progress_percent("no progress"), None);
    }

    #[test]
    fn extracts_only_marked_output_paths() {
        assert_eq!(
            output_path("__ADCONVERTER_OUTPUT__/tmp/video.mp4"),
            Some("/tmp/video.mp4")
        );
        assert_eq!(output_path("/tmp/video.mp4"), None);
    }

    #[tokio::test]
    async fn reports_a_missing_program() {
        let error = version_from_program(PathBuf::from("adconverter-definitely-missing-program"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("yt-dlp was not found"));
    }

    #[test]
    fn resolves_a_directory_to_the_yt_dlp_binary() {
        let directory = std::env::temp_dir();
        assert_eq!(program(Some(directory.clone())), directory.join("yt-dlp"));
    }
}
