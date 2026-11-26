use std::{
    fs,
    io::{self, Write},
    path::Path,
};

/// Refresh bundled scripts, while preserving configuration, presets and custom files.
pub(crate) fn sync(source: &Path, target: &Path) -> io::Result<()> {
    let version = (!cfg!(debug_assertions)).then(|| {
        semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo package version is valid")
    });
    sync_version(source, target, version.as_ref())
}

fn sync_version(source: &Path, target: &Path, version: Option<&semver::Version>) -> io::Result<()> {
    fs::metadata(source)?;
    fs::create_dir_all(target.join("scripts-custom"))?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(target.join(".bundled-scripts.lock"))?;
    lock.lock()?;

    let marker = target.join(".bundled-scripts-version");
    let previous = match fs::read_to_string(&marker) {
        Ok(value) => Some(semver::Version::parse(value.trim()).map_err(io::Error::other)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let refresh = version.is_some_and(|version| {
        previous
            .as_ref()
            .is_none_or(|previous| version.cmp_precedence(previous).is_ge())
    });
    if let Some(version) = version.filter(|_| refresh)
        && previous.as_ref() != Some(version)
    {
        // Record the version before replacing any scripts so an interrupted update
        // can be retried without allowing an older installation to overwrite it.
        let mut pending = tempfile::NamedTempFile::new_in(target)?;
        writeln!(pending, "{version}")?;
        pending.persist(marker).map_err(|error| error.error)?;
    }
    let sync_scripts = refresh || previous.is_none();
    sync_directory(source, source, target, target, sync_scripts, refresh)
}

fn sync_directory(
    root: &Path,
    source: &Path,
    target: &Path,
    user_root: &Path,
    sync_scripts: bool,
    refresh: bool,
) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name() == "__pycache__" || path.extension().is_some_and(|ext| ext == "pyc") {
            continue;
        }
        let destination = target.join(entry.file_name());
        let relative = path.strip_prefix(root).map_err(io::Error::other)?;
        if !sync_scripts && relative.starts_with("scripts") {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if relative.starts_with("scripts")
                && fs::symlink_metadata(&destination).is_ok_and(|metadata| metadata.is_symlink())
            {
                continue;
            }
            sync_directory(root, &path, &destination, user_root, sync_scripts, refresh)?;
        } else if kind.is_file() {
            if refresh && let Ok(script) = relative.strip_prefix("scripts") {
                update_script(
                    &path,
                    &destination,
                    &user_root.join("scripts-custom/backups").join(script),
                )?;
            } else {
                copy_new(&path, &destination)?;
            }
        }
    }
    Ok(())
}

fn copy_new(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut pending = tempfile::NamedTempFile::new_in(destination.parent().unwrap())?;
    io::copy(&mut fs::File::open(source)?, &mut pending)?;
    pending
        .as_file()
        .set_permissions(fs::metadata(source)?.permissions())?;
    match pending.persist_noclobber(destination) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}

fn update_script(source: &Path, destination: &Path, backup: &Path) -> io::Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.is_symlink() => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return copy_new(source, destination);
        }
        Err(error) => return Err(error),
    }
    let contents = fs::read(source)?;
    let previous = match fs::read(destination) {
        Ok(previous) if previous == contents => return Ok(()),
        Ok(previous) => previous,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return copy_new(source, destination);
        }
        Err(error) => return Err(error),
    };

    // Keep every displaced version, including edits made before managed scripts
    // and custom scripts were separated. Failure to back up leaves it untouched.
    let directory = backup.parent().unwrap();
    fs::create_dir_all(directory)?;
    let prefix = format!("{}.", backup.file_name().unwrap().to_string_lossy());
    let mut saved = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".bak")
        .tempfile_in(directory)?;
    saved.write_all(&previous)?;
    saved.flush()?;
    let (_, saved_path) = saved.keep().map_err(|error| error.error)?;

    let mut pending = tempfile::NamedTempFile::new_in(destination.parent().unwrap())?;
    pending.write_all(&contents)?;
    pending.flush()?;
    pending
        .as_file()
        .set_permissions(fs::metadata(source)?.permissions())?;
    pending.persist(destination).map_err(|error| error.error)?;
    log::info!("Updated bundled script: {destination:?}; previous copy: {saved_path:?}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, name: &str, contents: &str) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn sync_release(source: &Path, user: &Path, version: &str) -> io::Result<()> {
        sync_version(
            source,
            user,
            Some(&semver::Version::parse(version).unwrap()),
        )
    }

    #[test]
    fn refreshes_scripts_and_preserves_user_assets_and_every_displaced_version() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("bundled");
        let user = temporary.path().join("user");
        write(&source, "config.json", "default configuration");
        write(&source, "presets/mp3.json", "default preset");
        write(&source, "scripts/transcript.py", "script v1");
        write(&source, "scripts/lib/helper.py", "helper v1");
        write(&source, "scripts/__pycache__/helper.pyc", "cache");
        sync_release(&source, &user, "0.18.3").unwrap();
        assert!(user.join("scripts-custom").is_dir());
        assert!(!user.join("scripts/__pycache__").exists());

        write(&user, "config.json", "personal configuration");
        write(&user, "presets/mp3.json", "personal preset");
        write(&user, "scripts/transcript.py", "personal edits to v1");
        write(&user, "scripts/my_legacy_script.py", "legacy custom script");
        write(&user, "scripts-custom/transcript.py", "custom script");
        write(&source, "config.json", "new default configuration");
        write(&source, "presets/mp3.json", "new default preset");
        write(&source, "scripts/transcript.py", "script v2");
        write(&source, "scripts/lib/helper.py", "helper v2");
        write(&source, "presets/new.json", "new preset");
        sync_release(&source, &user, "0.18.4").unwrap();

        for (name, expected) in [
            ("config.json", "personal configuration"),
            ("presets/mp3.json", "personal preset"),
            ("presets/new.json", "new preset"),
            ("scripts/transcript.py", "script v2"),
            ("scripts/lib/helper.py", "helper v2"),
            ("scripts/my_legacy_script.py", "legacy custom script"),
            ("scripts-custom/transcript.py", "custom script"),
        ] {
            assert_eq!(fs::read_to_string(user.join(name)).unwrap(), expected);
        }
        let backup_dir = user.join("scripts-custom/backups");
        let backups = || {
            fs::read_dir(&backup_dir)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_file())
                .map(|entry| fs::read_to_string(entry.path()).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(backups(), vec!["personal edits to v1"]);
        let helper_backup = fs::read_dir(backup_dir.join("lib"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::read_to_string(helper_backup).unwrap(), "helper v1");
        sync_release(&source, &user, "0.18.4").unwrap();
        assert_eq!(backups().len(), 1); // No backup or rewrite on an unchanged start.

        write(&source, "scripts/transcript.py", "script v3");
        sync_release(&source, &user, "0.18.4").unwrap();
        let saved = backups();
        assert_eq!(saved.len(), 2);
        assert!(saved.contains(&"personal edits to v1".to_string()));
        assert!(saved.contains(&"script v2".to_string()));
    }

    #[test]
    fn backup_failure_does_not_replace_the_existing_script() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("bundled");
        let user = temporary.path().join("user");
        write(&source, "scripts/transcript.py", "new script");
        write(&user, "scripts/transcript.py", "personal script");
        write(
            &user,
            "scripts-custom/backups",
            "a file blocks the backup directory",
        );
        assert!(sync_release(&source, &user, "0.18.4").is_err());
        assert_eq!(
            fs::read_to_string(user.join("scripts/transcript.py")).unwrap(),
            "personal script"
        );
    }

    #[test]
    fn older_releases_and_debug_builds_cannot_change_newer_scripts() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        let user = temp.path().join("user");
        write(&old, "scripts/transcript.py", "old");
        write(&old, "scripts/legacy.py", "old helper");
        write(&old, "presets/default.json", "preset");
        write(&new, "scripts/transcript.py", "new");
        sync_release(&new, &user, "0.18.10").unwrap();
        sync_release(&old, &user, "0.18.9").unwrap();
        sync_version(&old, &user, None).unwrap();
        assert_eq!(
            fs::read_to_string(user.join("scripts/transcript.py")).unwrap(),
            "new"
        );
        assert!(!user.join("scripts/legacy.py").exists());
        assert!(user.join("presets/default.json").is_file());
        assert!(!user.join("scripts-custom/backups").exists());
    }

    #[cfg(unix)]
    #[test]
    fn leaves_linked_scripts_and_script_directories_in_place() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("bundled");
        let user = temp.path().join("user");
        let custom = temp.path().join("custom");
        write(&source, "scripts/transcript.py", "bundled");
        write(&source, "scripts/missing.py", "bundled");
        write(&source, "scripts/lib/helper.py", "bundled helper");
        write(&custom, "transcript.py", "custom");
        fs::create_dir_all(user.join("scripts")).unwrap();
        for (name, target) in [
            ("transcript.py", custom.join("transcript.py")),
            ("missing.py", custom.join("missing.py")),
            ("lib", custom.clone()),
        ] {
            std::os::unix::fs::symlink(&target, user.join("scripts").join(name)).unwrap();
        }
        sync_release(&source, &user, "0.18.4").unwrap();
        for name in ["transcript.py", "missing.py", "lib"] {
            assert!(
                fs::symlink_metadata(user.join("scripts").join(name))
                    .unwrap()
                    .is_symlink()
            );
        }
        assert_eq!(
            fs::read_to_string(user.join("scripts/transcript.py")).unwrap(),
            "custom"
        );
        assert!(!custom.join("helper.py").exists());
        assert!(!user.join("scripts-custom/backups").exists());
    }
}
