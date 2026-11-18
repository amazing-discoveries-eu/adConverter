# adConverter CLI

## Linux

Install the standalone CLI package using your package manager:

```sh
sudo apt install ./adconverter-cli_VERSION_amd64.deb
# Or on an RPM distribution (FFmpeg repositories must be enabled):
sudo dnf install ./adconverter-cli_VERSION_x86_64.rpm
```

Run `adconverter-cli --help` from a terminal. Packages install the command in
`/usr/bin/adconverter-cli` and all supplied presets, scripts, and the default
configuration in `/usr/share/adconverter-cli/assets/`. The lowercase command
name avoids colliding with the CLI included in the desktop application.
FFmpeg (including ffprobe), Python 3, and linked system libraries are package
dependencies. The Linux build targets Ubuntu 22.04; RPM hosts must provide
compatible libraries.

## macOS and Windows

Extract the entire ZIP and keep the `assets` directory beside `adConverter-cli`
(`adConverter-cli.exe` on Windows). Run it from a terminal. The macOS executable
is Developer ID signed and its ZIP is notarized. Install `ffmpeg` and `ffprobe`
separately and make them available on PATH, or configure their location.

## Configuration and presets

On startup, the CLI copies missing bundled files into the user data directory.
Existing configuration, presets, and scripts are preserved. The desktop app does
not need to be installed or started first.

- macOS: `~/Library/Application Support/com.adconverter/`
- Linux: `$XDG_DATA_HOME/com.adconverter/` (default `~/.local/share/com.adconverter/`)
- Windows: `%APPDATA%\com.adconverter\`

Edit `config.json` and the JSON files under `presets/` there. These files remain
after uninstalling the package. New bundled files are added on the next CLI run;
existing files are never replaced automatically.

The supplied transcription scripts require additional Python dependencies or a
container setup; see `scripts/README.md` and the bundled requirements files.
Transcription is disabled in the default configuration (`transcript_cmd` is empty).
