# adConverter CLI

## Standalone Linux packages

Install the CLI using your package manager:

```sh
sudo apt install ./adconverter-cli_VERSION_amd64.deb
# Or on an RPM distribution:
sudo dnf install ./adconverter-cli_VERSION_x86_64.rpm
```

Run `adconverter-cli --help` from a terminal. Packages install the command in
`/usr/bin/adconverter-cli` and all supplied presets, scripts, and the default
configuration in `/usr/share/adconverter-cli/assets/`. The lowercase command
name avoids colliding with the CLI included in the desktop application.

The standalone CLI is built without the `desktop` Cargo feature. It does not
link Tauri, GTK, WebKitGTK, JavaScriptCore, or libsoup. Only actual binary library
requirements are declared as hard dependencies. The build targets Ubuntu 22.04;
RPM hosts must provide compatible system libraries.

FFmpeg and ffprobe are required for conversion but are installed separately.
The package deliberately does not require a package named `ffmpeg`: depending on
the distribution, `ffmpeg-free`, third-party packages, or a custom FFmpeg build
can supply the executables. Make them available on PATH or set `ffmpeg_path`
in `config.json`. Python is only needed when using Python transcription scripts.

To build the CLI directly from source:

```sh
cargo build --release --no-default-features --manifest-path src-tauri/Cargo.toml --bin adConverter-cli
```

For a manual installation, place `src-tauri/assets` beside the executable as an
`assets` directory, or install the Linux package which includes those files.

## macOS and Windows

The CLI remains included in the desktop bundle; there is no separate CLI
release package for these platforms. Run the bundled `adConverter-cli`
(`adConverter-cli.exe` on Windows) from a terminal. The macOS CLI is signed
as part of the app bundle. FFmpeg and ffprobe must be installed separately.

## Configuration and presets

On startup, the CLI copies missing bundled files into the user data directory.
Existing configuration, presets, and scripts are preserved. The desktop app does
not need to be installed or started first when using the standalone Linux package.

- macOS: `~/Library/Application Support/com.adconverter/`
- Linux: `$XDG_DATA_HOME/com.adconverter/` (default `~/.local/share/com.adconverter/`)
- Windows: `%APPDATA%\com.adconverter\`

Edit `config.json` and the JSON files under `presets/` there. These files remain
after uninstalling the package. New bundled files are added on the next CLI run;
existing files are never replaced automatically.

The supplied transcription scripts require additional Python dependencies or a
container setup; see `scripts/README.md` and the bundled requirements files.
Transcription is disabled in the default configuration (`transcript_cmd` is empty).
