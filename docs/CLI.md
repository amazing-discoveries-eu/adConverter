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
Configuration and existing presets are preserved. Bundled files under `scripts/`
are refreshed when their contents change. The desktop app uses the same update
policy. The standalone Linux CLI can update scripts on its own.

The app records the highest release version that has started updating the scripts.
Earlier releases then skip the scripts directory. Debug builds only seed missing
scripts before a release has managed the directory. Rebuilt releases with the
same version can refresh the scripts. Symbolic links to scripts or script
directories are preserved.

- macOS: `~/Library/Application Support/com.adconverter/`
- Linux: `$XDG_DATA_HOME/com.adconverter/` (default `~/.local/share/com.adconverter/`)
- Windows: `%APPDATA%\com.adconverter\`

Edit `config.json` and the JSON files under `presets/` there. These files remain
after uninstalling the package. New bundled files are added on the next CLI run.

Put your own scripts in `scripts-custom/` in the same user data directory and
point `transcript_cmd` at them. This directory is created automatically and its
contents are not replaced by application updates. Files in `scripts/` whose
names do not occur in the bundle are also left untouched for compatibility.

Before replacing a changed bundled script, the app saves its previous contents
under `scripts-custom/backups/`, with a unique `.bak` filename. This preserves
edits made to older script copies, too. Existing backups are never overwritten,
and unchanged scripts do not create another backup on every start. To keep using
your modified version, copy it from the backup into `scripts-custom/` and update
`transcript_cmd`. Include any local helper modules it imports alongside it.

Python virtual environments and installed Python packages are not automatically
modified. If a new script requires updated dependencies, update the environment
using the bundled requirements files.

The supplied transcription scripts require additional Python dependencies or a
container setup; see `scripts/README.md` and the bundled requirements files.
Transcription is disabled in the default configuration (`transcript_cmd` is empty).

The transcription command must pass `%output%` to the script as its output
directory (for the supplied script: `-o %output%`). Each job receives a private
temporary directory, preventing collisions between media with the same basename.
Container commands must mount this directory too; see `scripts/README.md`.
A nonzero exit code, missing output, or invalid VTT output fails the job instead
of reporting success. Final subtitle files are replaced only after transcription
has completed successfully.

## Optional system-wide CLI configuration

The CLI loads configuration in this order, with later settings taking priority:

1. Built-in defaults.
2. `/etc/adconverter/config.json`, if present (Linux and macOS).
3. The user's `config.json` at the data-directory path above, if present.
4. Command-line options, including `--transcript-model-dir`.

On Windows, the optional global file is `%PROGRAMDATA%\adconverter\config.json`.
This layering applies to the CLI; the desktop application's settings are unchanged.
Objects such as `lufs` are merged field by field. Arrays are replaced. Empty
strings and `null` are explicit overrides, not requests to inherit a global value.
Malformed or unreadable existing files produce an error that identifies the file.

Global configuration is disabled simply by leaving the global file absent.
Packages ship only an example at
`/usr/share/doc/adconverter-cli/config.example.json`; they never install or modify
the active `/etc/adconverter/config.json`. To enable the example manually:

```sh
sudo install -d -m 755 /etc/adconverter
sudo install -m 644 /usr/share/doc/adconverter-cli/config.example.json /etc/adconverter/config.json
```

The example assumes a Python environment at `/opt/adconverter-transcript` with
the transcription dependencies installed, and uses the packaged scripts:

```json
{
    "transcript_cmd": "/opt/adconverter-transcript/bin/python /usr/share/adconverter-cli/assets/scripts/transcript_cli.py -d cpu -c int8 -l %lang% -f %file% -o %output%",
    "transcript_model_dir": null
}
```

New user configurations contain `{}` and inherit these defaults. Existing user
configurations are preserved. Remove `transcript_cmd` or `transcript_model_dir`
from an existing user file to inherit those global settings; an existing empty
`transcript_cmd` deliberately disables the global transcription command.

For one invocation, ignore the global file:

```sh
adconverter-cli --no-global-config --files video.mp4
```

Or use an explicitly selected file. `--config` replaces both global and user file
layers; built-in defaults still fill omitted fields. A missing explicit file is
an error. This also lets existing users select the global file directly:

```sh
adconverter-cli --config /etc/adconverter/config.json --files video.mp4 \
  --lang de --fade false --lufs false --presets
```

## Optional shared model cache

Set `transcript_model_dir` to an absolute directory such as
`/var/cache/adconverter/models` to share downloaded models. Leave it absent or
`null` to keep the normal per-user model cache. Users downloading models need
write access to the shared directory, which can be managed through a shared group.
The packaged `transcript_cli.py` and `model_cache.py` support this setting for
Faster Whisper and MLX. Bundled copies under the user data directory's `scripts/`
are refreshed automatically. Private versions in `scripts-custom/` must be updated
explicitly, or use the current packaged script directly.

To override the configured model directory for a single invocation:

```sh
adconverter-cli --transcript-model-dir /srv/whisper-models --files video.mp4
```

The directory is passed only to the transcription subprocess, through
`ADCONVERTER_TRANSCRIPT_MODEL_DIR` and `HF_HUB_CACHE`. Custom scripts can read
these variables too. The CLI does not change the parent shell environment.
