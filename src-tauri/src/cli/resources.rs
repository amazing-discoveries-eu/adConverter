use std::{
    fs,
    io::{self, Write},
    path::Path,
};

fn seed_user_assets(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target.join("config.json"))
    {
        // Defaults are loaded by the CLI. A full copy would hide global settings.
        Ok(mut file) => file.write_all(b"{}\n")?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    crate::utils::assets::sync(source, target)
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
        seed_user_assets(source, &target)?;
    } else if !target.join("config.json").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "CLI assets missing; reinstall adConverter-cli with its assets directory",
        ));
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
        seed_user_assets(&source, &target).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("config.json")).unwrap(),
            "{}\n"
        );
        assert_eq!(
            fs::read_to_string(target.join("scripts/transcript.py")).unwrap(),
            "script"
        );

        fs::write(target.join("config.json"), "user configuration").unwrap();
        fs::write(target.join("presets/mp3.json"), "user preset").unwrap();
        fs::write(source.join("presets/new.json"), "new preset").unwrap();
        seed_user_assets(&source, &target).unwrap();
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
