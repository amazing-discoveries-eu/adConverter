use serde::{Deserialize, Serialize};
#[cfg(feature = "desktop")]
use tauri::Error as TauriError;
use thiserror::Error;

use crate::ffmpeg::probe::FfProbeError;

#[derive(Error, Debug, Serialize, Deserialize)]
pub enum ProcessError {
    #[error("{0}")]
    Custom(String),

    #[error("{0}")]
    Module(String),

    #[error("{0}")]
    Tauri(String),

    #[error("IO error: {0}")]
    IO(String),

    #[error("{0}")]
    Ffprobe(String),

    #[error("Regex compile error {0}")]
    Regex(String),

    #[error("Thread error {0}")]
    Thread(String),
}

impl From<std::io::Error> for ProcessError {
    fn from(err: std::io::Error) -> ProcessError {
        ProcessError::IO(err.to_string())
    }
}

impl From<std::path::StripPrefixError> for ProcessError {
    fn from(err: std::path::StripPrefixError) -> ProcessError {
        ProcessError::IO(err.to_string())
    }
}

impl From<FfProbeError> for ProcessError {
    fn from(err: FfProbeError) -> Self {
        Self::Ffprobe(err.to_string())
    }
}

impl<T> From<std::sync::PoisonError<T>> for ProcessError {
    fn from(err: std::sync::PoisonError<T>) -> ProcessError {
        ProcessError::Custom(err.to_string())
    }
}

impl From<&str> for ProcessError {
    fn from(err: &str) -> Self {
        Self::Regex(err.to_string())
    }
}

impl From<regex::Error> for ProcessError {
    fn from(err: regex::Error) -> Self {
        Self::Regex(err.to_string())
    }
}

impl From<serde_json::Error> for ProcessError {
    fn from(err: serde_json::Error) -> Self {
        Self::Custom(err.to_string())
    }
}

#[cfg(feature = "desktop")]
impl From<tauri::Error> for ProcessError {
    fn from(err: tauri::Error) -> Self {
        Self::Tauri(err.to_string())
    }
}

#[cfg(feature = "desktop")]
impl From<tauri_plugin_store::Error> for ProcessError {
    fn from(err: tauri_plugin_store::Error) -> Self {
        Self::Tauri(err.to_string())
    }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for ProcessError {
    fn from(err: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Self::Custom(err.to_string())
    }
}

impl From<tokio::task::JoinError> for ProcessError {
    fn from(err: tokio::task::JoinError) -> Self {
        Self::Custom(err.to_string())
    }
}

#[cfg(feature = "desktop")]
impl From<tauri_plugin_http::reqwest::Error> for ProcessError {
    fn from(err: tauri_plugin_http::reqwest::Error) -> Self {
        Self::Tauri(err.to_string())
    }
}

impl From<Box<dyn std::any::Any + std::marker::Send>> for ProcessError {
    fn from(err: Box<dyn std::any::Any + std::marker::Send>) -> Self {
        Self::Thread(format!("{err:?}"))
    }
}

impl From<Box<dyn std::error::Error>> for ProcessError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        Self::Thread(format!("{err:?}"))
    }
}

impl From<inquire::InquireError> for ProcessError {
    fn from(err: inquire::InquireError) -> ProcessError {
        Self::Module(err.to_string())
    }
}

#[cfg(feature = "desktop")]
impl From<ProcessError> for TauriError {
    fn from(err: ProcessError) -> Self {
        TauriError::Anyhow(err.into())
    }
}

#[cfg(feature = "desktop")]
impl From<ProcessError> for tauri::Result<()> {
    fn from(err: ProcessError) -> Self {
        Err(TauriError::Anyhow(err.into()))
    }
}
