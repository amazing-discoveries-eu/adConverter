// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use serde_with::{NoneAsEmptyString, serde_as};
// use tauri::Listener;
// use tauri::Manager;
use log::*;
use serde_json::{Value, json};
use tauri::{
    AppHandle, Emitter, Manager, State, WindowEvent,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_store::StoreExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags, WindowExt};
use tokio::{
    process::Child,
    sync::{
        Mutex,
        mpsc::{Sender, channel},
    },
};
use ts_rs::TS;

pub mod cli;
mod ffmpeg;
mod macros;
mod publisher;
mod transcript;
mod utils;

pub use publisher::Publish;
pub use utils::{
    Sources, copy_assets, delete_files, download,
    errors::ProcessError,
    logging::init_logging,
    presets::{Preset, collect_presets},
    template::Template,
    update,
};

use ffmpeg::{probe::MediaProbe, worker};

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

#[derive(Clone)]
struct AppState {
    run: Arc<AtomicBool>,
    sender: Sender<Task>,
    encoder: Arc<Mutex<Option<Child>>>,
    downloader: Arc<Mutex<Option<Child>>>,
    config: Arc<Mutex<Config>>,
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

const DEFAULT_DOWNLOAD_ARGS: &str = "--output \"%(title)s.%(ext)s\"";

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

impl AppState {
    fn new(tx: Sender<Task>) -> Self {
        Self {
            run: Arc::new(AtomicBool::new(false)),
            sender: tx,
            encoder: Arc::new(Mutex::new(None)),
            downloader: Arc::new(Mutex::new(None)),
            config: Arc::new(Mutex::new(Config {
                lufs: LufsConfig {
                    i: -17.0,
                    lra: 9.0,
                    tp: -1.0,
                },

                transcript_cmd: String::new(),
                download_args: DEFAULT_DOWNLOAD_ARGS.to_string(),
                ..Default::default()
            })),
        }
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        let encoder = self.encoder.clone();
        let downloader = self.downloader.clone();
        self.run.store(false, Ordering::SeqCst);

        tokio::spawn(async move {
            for process in [encoder, downloader] {
                if let Some(mut proc) = process.lock().await.take() {
                    if let Err(e) = proc.kill().await {
                        eprintln!("Failed to kill process: {e:?}");
                    }
                    if let Err(e) = proc.wait().await {
                        eprintln!("Failed to wait for process: {e:?}");
                    }
                }
            }
        });
    }
}

#[tauri::command]
async fn presets_get(app: AppHandle) -> tauri::Result<Vec<Preset>> {
    let presets = collect_presets(&Some(app))
        .await
        .map_err(|e| tauri::Error::AssetNotFound(e.to_string()))?;

    Ok(presets)
}

#[tauri::command]
async fn file_drop(state: State<'_, AppState>, mut task: Task) -> Result<Task, ProcessError> {
    let config = state.config.lock().await;
    let path = task.path.clone();
    task.probe = MediaProbe::new(&config, &path).await?;

    let sources = Sources::new(&path).await?;

    match sources.template {
        Some(src) => task.template = Some(Template::new(&src).await?),
        None => task.template = None,
    };

    Ok(task)
}

#[tauri::command]
async fn yt_dlp_version(state: State<'_, AppState>) -> Result<String, ProcessError> {
    let path = state.config.lock().await.yt_dlp_path.clone();
    download::version(path).await
}

#[tauri::command]
async fn download_url(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    target: Option<String>,
) -> Result<String, ProcessError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(ProcessError::Custom(
            "Please enter a valid http(s) URL".into(),
        ));
    }

    let config = state.config.lock().await;
    let configured_path = config.download_path.clone();
    let yt_dlp_path = config.yt_dlp_path.clone();
    let arguments = if config.download_args.trim().is_empty() {
        DEFAULT_DOWNLOAD_ARGS.to_string()
    } else {
        config.download_args.clone()
    };
    drop(config);
    let directory = resolve_download_directory(target, configured_path, dirs::download_dir())
        .ok_or_else(|| ProcessError::Custom("Could not determine the Downloads folder".into()))?;

    app.emit("download-start", &url)
        .map_err(|error| ProcessError::Custom(error.to_string()))?;
    let file = download::download(
        app.clone(),
        url,
        directory,
        &arguments,
        yt_dlp_path,
        state.downloader.clone(),
    )
    .await?;
    app.emit("download-finish", &file)
        .map_err(|error| ProcessError::Custom(error.to_string()))?;
    Ok(file)
}

#[tauri::command]
async fn task_start(state: State<'_, AppState>) -> Result<(), ProcessError> {
    state.run.store(true, Ordering::SeqCst);

    Ok(())
}

#[tauri::command]
async fn task_send(task: Task, state: State<'_, AppState>) -> Result<(), ProcessError> {
    state.sender.send(task).await?;

    Ok(())
}

#[tauri::command]
async fn task_cancel(task: Task, state: State<'_, AppState>) -> Result<(), ProcessError> {
    let encoder = state.encoder.clone();
    let downloader = state.downloader.clone();
    state.run.store(false, Ordering::SeqCst);

    for process in [encoder.clone(), downloader] {
        if let Some(mut proc) = process.lock().await.take() {
            proc.kill().await?;
            proc.wait().await?;
        }
    }

    *encoder.lock().await = None;

    for preset in task.presets {
        if let Some(path) = preset.output_path {
            match delete_files(&path).await {
                Ok(_) => info!("Delete unfinished file: {path:?}"),
                Err(e) => error!("{e}"),
            };
        }
    }

    Ok(())
}

#[tauri::command]
async fn template_save(template: Template, path: &str) -> Result<(), ProcessError> {
    template.save(path).await?;

    Ok(())
}

#[tauri::command]
async fn save_preset(app: AppHandle, mut preset: Preset) -> Result<(), ProcessError> {
    let preset_path = app.path().app_data_dir()?.join("presets");

    if !preset_path.is_dir() {
        tokio::fs::create_dir(&preset_path).await?;
    }

    preset.save(&preset_path).await
}

fn prep_ffmpeg_path(config: &mut Config) {
    if let Some(ffmpeg_path) = &config.ffmpeg_path {
        if ffmpeg_path.is_file() {
            config.ffmpeg_path = ffmpeg_path.parent().map(std::path::Path::to_path_buf);
        } else if !ffmpeg_path.is_dir() {
            config.ffmpeg_path = None;
        }
    }
}

fn resolve_download_directory(
    target: Option<String>,
    configured: Option<PathBuf>,
    fallback: Option<PathBuf>,
) -> Option<PathBuf> {
    target
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| configured.filter(|path| !path.as_os_str().is_empty()))
        .or(fallback)
}

fn non_empty_path(path: &str) -> Option<PathBuf> {
    (!path.trim().is_empty()).then(|| PathBuf::from(path))
}

#[tauri::command]
async fn save_config(app: AppHandle, state: State<'_, AppState>) -> Result<(), ProcessError> {
    let store = app.store("config.json")?;
    let mut config = state.config.lock().await;

    if let Some(Value::String(ffmpeg_path)) = store.get("ffmpeg_path") {
        config.ffmpeg_path = Some(PathBuf::from(ffmpeg_path));
    }
    if let Some(Value::String(download_path)) = store.get("download_path") {
        config.download_path = Some(PathBuf::from(download_path));
    }
    if let Some(Value::String(download_args)) = store.get("download_args") {
        config.download_args = download_args;
    }
    if let Some(Value::String(yt_dlp_path)) = store.get("yt_dlp_path") {
        config.yt_dlp_path = non_empty_path(&yt_dlp_path);
    }

    if let Some(Value::String(copyright)) = store.get("copyright") {
        config.copyright = copyright;
    }

    if let Some(l) = store.get("lufs") {
        config.lufs.i = l.get("i").and_then(Value::as_f64).unwrap_or_default();
        config.lufs.lra = l.get("lra").and_then(Value::as_f64).unwrap_or_default();
        config.lufs.tp = l.get("tp").and_then(Value::as_f64).unwrap_or_default();
    }

    if let Some(Value::String(s)) = store.get("transcript_cmd") {
        config.transcript_cmd = s;
    }

    prep_ffmpeg_path(&mut config);
    store.close_resource();

    Ok(())
}

#[tauri::command]
async fn load_config(app: AppHandle, state: State<'_, AppState>) -> Result<(), ProcessError> {
    let app_handle = app.app_handle();

    copy_assets(app_handle).await.expect("Copy assets");

    let store = app_handle.store("config.json").expect("Open config");
    let mut config = state.config.lock().await;

    if let Some(Value::String(ffmpeg_path)) = store.get("ffmpeg_path") {
        config.ffmpeg_path = Some(PathBuf::from(ffmpeg_path));
    }
    if let Some(Value::String(download_path)) = store.get("download_path") {
        config.download_path = Some(PathBuf::from(download_path));
    }
    if let Some(Value::String(download_args)) = store.get("download_args") {
        config.download_args = download_args;
    }
    if let Some(Value::String(yt_dlp_path)) = store.get("yt_dlp_path") {
        config.yt_dlp_path = non_empty_path(&yt_dlp_path);
    }

    if let Some(Value::String(copyright)) = store.get("copyright") {
        config.copyright = copyright;
    }

    if let Some(Value::String(publish_preset)) = store.get("publish_preset") {
        config.publish_preset = Some(publish_preset);
    }

    config.publisher = store.get("publisher");

    match store.get("lufs") {
        Some(l) => {
            config.lufs.i = l.get("i").and_then(Value::as_f64).unwrap_or_default();
            config.lufs.lra = l.get("lra").and_then(Value::as_f64).unwrap_or_default();
            config.lufs.tp = l.get("tp").and_then(Value::as_f64).unwrap_or_default();
        }
        None => {
            store.set("lufs", json!({ "i": -17.0, "lra": 9.0, "tp": -1.0 }));
            store.save().expect("Save config");

            config.lufs.i = -17.0;
            config.lufs.lra = 9.0;
            config.lufs.tp = -1.0;
        }
    }

    match store.get("transcript_cmd") {
        Some(t) => {
            if let Value::String(s) = t {
                config.transcript_cmd = s;
            }
        }
        None => {
            store.set("transcript_cmd", "");
            store.save().expect("Save config");
        }
    }

    prep_ffmpeg_path(&mut config);
    store.close_resource();

    Ok(())
}

#[tauri::command]
fn shutdown_system() -> Result<(), ProcessError> {
    system_shutdown::shutdown()?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub async fn run() -> tauri::Result<()> {
    let (tx, rx) = channel(5);

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_cli::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin({
            let mut w_state = tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::POSITION)
                .with_state_flags(StateFlags::VISIBLE);

            if cfg!(not(target_os = "linux")) {
                w_state = w_state.with_state_flags(StateFlags::SIZE);
            }

            w_state.build()
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::new(tx))
        .setup(|app| {
            let app_handle = app.app_handle().clone();
            let app_handle_clone = app_handle.clone();
            let app_handle_clone2 = app_handle.clone();
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;
            let window = app.get_webview_window("main").unwrap();

            let _logger = init_logging(Some(app_handle.clone()));

            tokio::spawn(async move {
                if let Err(error) = update(app_handle_clone2).await {
                    error!("Update check failed: {error}");
                }
            });

            window
                .restore_state(StateFlags::POSITION)
                .expect("Restore window position");

            #[cfg(not(target_os = "linux"))]
            window
                .restore_state(StateFlags::SIZE)
                .expect("Restore window size");

            tokio::spawn(async move {
                if let Err(e) = worker::run(app_handle_clone.clone(), rx).await {
                    error!("{e:?}");
                };
            });

            let _ = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(true)
                .icon(app.default_window_icon().unwrap().clone())
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {
                        println!("menu item {:?} not handled", event.id);
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { .. } => {
                let app_state = window.state::<AppState>();
                let encoder = app_state.encoder.clone();
                let downloader = app_state.downloader.clone();

                tokio::spawn(async move {
                    for process in [encoder, downloader] {
                        if let Some(mut proc) = process.lock().await.take() {
                            if let Err(e) = proc.kill().await {
                                eprintln!("Failed to kill process: {e:?}");
                            }
                            if let Err(e) = proc.wait().await {
                                eprintln!("Failed to wait for process: {e:?}");
                            }
                        }
                    }
                });
            }
            WindowEvent::Moved { .. } => {
                let app = window.app_handle();
                app.save_window_state(StateFlags::all())
                    .expect("Save window position");
            }
            WindowEvent::Resized { .. } => {
                let app = window.app_handle();
                app.save_window_state(StateFlags::all())
                    .expect("Save window position");
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            file_drop,
            yt_dlp_version,
            download_url,
            presets_get,
            task_start,
            task_send,
            task_cancel,
            template_save,
            save_config,
            save_preset,
            shutdown_system,
            load_config,
        ])
        .run(tauri::generate_context!())?;

    Ok(())
}

#[cfg(test)]
mod download_path_tests {
    use super::*;

    #[test]
    fn save_as_overrides_configured_download_path() {
        assert_eq!(
            resolve_download_directory(
                Some("/save-as".into()),
                Some(PathBuf::from("/configured")),
                Some(PathBuf::from("/fallback")),
            ),
            Some(PathBuf::from("/save-as"))
        );
    }

    #[test]
    fn empty_paths_use_download_folder_fallback() {
        assert_eq!(
            resolve_download_directory(
                Some("  ".into()),
                Some(PathBuf::new()),
                Some(PathBuf::from("/fallback")),
            ),
            Some(PathBuf::from("/fallback"))
        );
    }

    #[test]
    fn ignores_an_empty_yt_dlp_path() {
        assert_eq!(non_empty_path("  "), None);
        assert_eq!(
            non_empty_path("/opt/homebrew/bin/yt-dlp"),
            Some(PathBuf::from("/opt/homebrew/bin/yt-dlp"))
        );
    }
}
