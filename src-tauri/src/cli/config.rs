use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{Config, ProcessError};

pub fn global_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("PROGRAMDATA")
            .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
            .join("adconverter/config.json")
    }
    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from("/etc/adconverter/config.json")
    }
}

fn merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                merge(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn read_layer(path: &Path, optional: bool) -> Result<Option<Value>, ProcessError> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if optional && error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ProcessError::Custom(format!(
                "Cannot read CLI configuration {}: {error}",
                path.display()
            )));
        }
    };
    let value: Value = serde_json::from_str(&contents).map_err(|error| {
        ProcessError::Custom(format!(
            "Invalid CLI configuration {}: {error}",
            path.display()
        ))
    })?;
    if !value.is_object() {
        return Err(ProcessError::Custom(format!(
            "CLI configuration {} must contain a JSON object",
            path.display()
        )));
    }
    Ok(Some(value))
}

/// Explicit configuration replaces both normal file layers; defaults always apply.
pub fn load(
    global: &Path,
    user: &Path,
    explicit: Option<&Path>,
    no_global: bool,
) -> Result<Config, ProcessError> {
    let mut value: Value = serde_json::from_str(include_str!("../../assets/config.json"))?;
    if let Some(path) = explicit {
        if let Some(layer) = read_layer(path, false)? {
            merge(&mut value, layer);
        }
    } else {
        if !no_global && let Some(layer) = read_layer(global, true)? {
            merge(&mut value, layer);
        }
        if let Some(layer) = read_layer(user, true)? {
            merge(&mut value, layer);
        }
    }
    serde_json::from_value(value)
        .map_err(|error| ProcessError::Custom(format!("Invalid merged CLI configuration: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct Files(PathBuf);

    impl Files {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "adconverter-config-{}-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path(name);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn missing_optional_files_use_embedded_defaults() {
        let files = Files::new();
        let config = load(&files.path("global"), &files.path("user"), None, false).unwrap();
        assert_eq!(config.transcript_cmd, "");
        assert_eq!(config.lufs.i, -17.0);
        assert!(config.transcript_model_dir.is_none());
    }

    #[test]
    fn merges_global_and_user_settings_recursively() {
        let files = Files::new();
        let global = files.write("global", r#"{"transcript_cmd":"global-python", "lufs":{"i":-20.0}, "transcript_model_dir":"/shared/models"}"#);
        let user = files.write("user", r#"{"copyright":"User", "lufs":{"tp":-2.0}, "transcript_lang":[{"name":"French", "code":"fr"}]}"#);
        let config = load(&global, &user, None, false).unwrap();
        assert_eq!(config.transcript_cmd, "global-python");
        assert_eq!(config.copyright, "User");
        assert_eq!(config.lufs.i, -20.0);
        assert_eq!(config.lufs.lra, 9.0);
        assert_eq!(config.lufs.tp, -2.0);
        assert_eq!(config.transcript_lang.len(), 1);
        assert_eq!(config.transcript_lang[0].code, "fr");
        assert_eq!(
            config.transcript_model_dir,
            Some(PathBuf::from("/shared/models"))
        );
    }

    #[test]
    fn explicit_user_values_can_disable_global_settings() {
        let files = Files::new();
        let global = files.write(
            "global",
            r#"{"transcript_cmd":"global", "transcript_model_dir":"/shared/models"}"#,
        );
        let user = files.write(
            "user",
            r#"{"transcript_cmd":"", "transcript_model_dir":null}"#,
        );
        let config = load(&global, &user, None, false).unwrap();
        assert!(config.transcript_cmd.is_empty());
        assert!(config.transcript_model_dir.is_none());
    }

    #[test]
    fn no_global_config_skips_even_an_invalid_global_file() {
        let files = Files::new();
        let global = files.write("global", "invalid json");
        let user = files.write("user", r#"{"transcript_cmd":"personal-python"}"#);
        let config = load(&global, &user, None, true).unwrap();
        assert_eq!(config.transcript_cmd, "personal-python");
    }

    #[test]
    fn explicit_file_replaces_global_and_user_layers() {
        let files = Files::new();
        let global = files.write("global", "invalid json");
        let user = files.write("user", "invalid json");
        let explicit = files.write("explicit", r#"{"transcript_cmd":"custom-python"}"#);
        let config = load(&global, &user, Some(&explicit), false).unwrap();
        assert_eq!(config.transcript_cmd, "custom-python");
        assert_eq!(config.lufs.i, -17.0);
        assert!(load(&global, &user, Some(&files.path("missing")), false).is_err());
    }

    #[test]
    fn reports_invalid_or_unreadable_present_files() {
        let files = Files::new();
        for contents in ["invalid json", "[]"] {
            let global = files.write("global", contents);
            let error = load(&global, &files.path("user"), None, false).unwrap_err();
            assert!(error.to_string().contains(&global.display().to_string()));
        }
        assert!(load(&files.0, &files.path("user"), None, false).is_err());
    }
}
