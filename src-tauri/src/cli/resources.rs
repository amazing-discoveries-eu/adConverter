use std::{fs, io, path::Path};

/// Seed user-editable assets without replacing existing configuration or presets.
pub fn copy_missing(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let destination = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_missing(&entry.path(), &destination)?;
        } else if entry.file_type()?.is_file() {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
            {
                Ok(mut output) => {
                    let mut input = fs::File::open(entry.path())?;
                    io::copy(&mut input, &mut output)?;
                    fs::set_permissions(&destination, entry.metadata()?.permissions())?;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_assets_and_preserves_user_files_on_update() {
        let temporary = std::env::temp_dir().join(format!(
            "adconverter-cli-assets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = temporary.join("assets");
        let target = temporary.join("user");
        fs::create_dir_all(source.join("presets")).unwrap();
        fs::create_dir_all(source.join("scripts")).unwrap();
        fs::write(source.join("config.json"), "default configuration").unwrap();
        fs::write(source.join("presets/mp3.json"), "default preset").unwrap();
        fs::write(source.join("scripts/transcript.py"), "script").unwrap();
        copy_missing(&source, &target).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("config.json")).unwrap(),
            "default configuration"
        );
        assert_eq!(
            fs::read_to_string(target.join("scripts/transcript.py")).unwrap(),
            "script"
        );

        fs::write(target.join("config.json"), "user configuration").unwrap();
        fs::write(target.join("presets/mp3.json"), "user preset").unwrap();
        fs::write(source.join("presets/new.json"), "new preset").unwrap();
        copy_missing(&source, &target).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("config.json")).unwrap(),
            "user configuration"
        );
        assert_eq!(
            fs::read_to_string(target.join("presets/mp3.json")).unwrap(),
            "user preset"
        );
        assert_eq!(
            fs::read_to_string(target.join("presets/new.json")).unwrap(),
            "new preset"
        );
        fs::remove_dir_all(temporary).unwrap();
    }
}

pub fn initialize() -> io::Result<()> {
    let target = dirs::data_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "User data directory unavailable"))?
        .join(super::IDENTIFIER);
    let executable = std::env::current_exe()?;
    let parent = executable
        .parent()
        .ok_or_else(|| io::Error::other("Missing executable directory"))?;
    let mut sources = vec![parent.join("assets")];
    // The desktop bundle stores the same assets in Contents/Resources.
    #[cfg(target_os = "macos")]
    sources.push(parent.join("../Resources/assets"));
    #[cfg(target_os = "linux")]
    sources.push(parent.join("../share/adconverter-cli/assets"));
    if cfg!(debug_assertions) {
        sources.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets"));
    }
    if let Some(source) = sources.iter().find(|source| source.is_dir()) {
        copy_missing(source, &target)?;
    } else if !target.join("config.json").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "CLI assets missing; reinstall adConverter-cli with its assets directory",
        ));
    }
    Ok(())
}
