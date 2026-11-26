use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

#[cfg(feature = "desktop")]
use log::*;
#[cfg(feature = "desktop")]
use tauri::{AppHandle, Manager, path::BaseDirectory};
#[cfg(feature = "desktop")]
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
#[cfg(feature = "desktop")]
use tauri_plugin_updater::UpdaterExt;
use tokio::fs::{self};

pub(crate) mod assets;
#[cfg(feature = "desktop")]
pub mod download;
pub mod errors;
pub mod logging;
pub mod presets;
pub mod template;

use crate::ProcessError;

pub const AUDIO_EXTENSIONS: [&str; 9] = [
    "wav", "aif", "aiff", "aac", "mp3", "flac", "mp2", "m4a", "opus",
];
pub const IMAGE_EXTENSIONS: [&str; 6] = ["exr", "png", "tga", "tif", "tiff", "gif"];
pub const VIDEO_EXTENSIONS: [&str; 15] = [
    "avi", "mov", "webm", "mp4", "mpv", "m4v", "h264", "mkv", "vob", "wmv", "yuv", "m2v", "mpg",
    "mpeg", "mxf",
];

#[derive(Clone, Default, Debug)]
pub struct Sources {
    pub audio: Option<String>,
    pub video: Option<String>,
    pub template: Option<PathBuf>,
}

impl Sources {
    pub async fn new(src: &str) -> Result<Self, ProcessError> {
        let path = PathBuf::from(src);
        let folder = path
            .parent()
            .ok_or(ProcessError::IO(format!("No parent folder from: {src}")))?;
        let mut entries = fs::read_dir(folder).await?;
        let mut source = Sources::default();

        if VIDEO_EXTENSIONS.contains(
            &path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
                .as_str(),
        ) {
            source.video = Some(src.to_string());
        };

        while let Some(entry) = entries.next_entry().await? {
            if entry.path().file_stem() == path.file_stem() {
                let extension = entry
                    .path()
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();

                if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
                    source.audio = Some(entry.path().to_string_lossy().to_string());
                } else if source.video.is_none() && VIDEO_EXTENSIONS.contains(&extension.as_str()) {
                    source.video = Some(entry.path().to_string_lossy().to_string());
                } else if extension.as_str() == "json" {
                    source.template = Some(entry.path().to_path_buf());
                }
            }
        }

        Ok(source)
    }
}

#[cfg(feature = "desktop")]
pub async fn update(app: tauri::AppHandle) -> tauri_plugin_updater::Result<()> {
    if let Some(update) = app.updater()?.check().await? {
        let mut downloaded = 0;

        let bytes = update
            .download(
                |chunk_length, content_length| {
                    downloaded += chunk_length;
                    debug!("downloaded {downloaded} from {content_length:?}");
                },
                || {
                    debug!("download finished");
                },
            )
            .await?;

        // On Windows the updater exits the app as soon as it launches the installer.
        #[cfg(target_os = "windows")]
        if !confirm_update_restart(
            &app,
            "The update is ready to install. Install it and restart adConverter now?",
        )
        .await
        {
            return Ok(());
        }

        update.install(bytes)?;
        debug!("update installed");

        #[cfg(not(target_os = "windows"))]
        if confirm_update_restart(
            &app,
            "The update is installed. Restart adConverter now to use the new version?",
        )
        .await
        {
            app.request_restart();
        }
    }

    Ok(())
}

#[cfg(feature = "desktop")]
async fn confirm_update_restart(app: &AppHandle, message: &str) -> bool {
    let (send, receive) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .message(message)
        .title("adConverter Update")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Restart now".into(),
            "Later".into(),
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |confirmed| {
        let _ = send.send(confirmed);
    });
    receive.await.unwrap_or(false)
}

pub fn time_to_sec(time_str: &str) -> f64 {
    let mut t = time_str
        .split([':', '.'])
        .filter_map(|n| f64::from_str(n).ok());

    let hours = t.next().unwrap_or(0.0);
    let minutes = t.next().unwrap_or(0.0);
    let seconds = t.next().unwrap_or(0.0);
    let milliseconds = t.next().unwrap_or(0.0);

    hours * 3600.0 + minutes * 60.0 + seconds + milliseconds / 1000.0
}

pub fn is_close<T: num_traits::Signed + std::cmp::PartialOrd>(a: T, b: T, to: T) -> bool {
    (a - b).abs() < to
}

#[cfg(feature = "desktop")]
pub async fn copy_assets(app: &AppHandle) -> Result<(), ProcessError> {
    let path_source = app.path().resolve("assets", BaseDirectory::Resource)?;
    let target_source = app.path().app_data_dir()?;

    tokio::task::spawn_blocking(move || assets::sync(&path_source, &target_source)).await??;
    Ok(())
}

pub async fn find_audio(src: &Path) -> Option<PathBuf> {
    let folder = src.parent()?;
    let mut entries = fs::read_dir(folder).await.ok()?;

    while let Some(entry) = entries.next_entry().await.ok()? {
        if entry.path().file_stem() == src.file_stem() {
            let extension = entry
                .path()
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();

            if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
                return Some(entry.path());
            }
        }
    }

    None
}
