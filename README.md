# adConverter

adConverter is a batch/preset based encoding application based on ffmpeg.

The program is intended for advanced users who are familiar with the ffmpeg syntax. It only makes sense to use it if you want to convert to the same formats over and over again.

For most users, popular programs like [Handbrake](https://handbrake.fr/) make more sense.

Since this program is intended more for in-house use, there will be little documentation and support.

Linux, MacOS and Windows are supported. ffmpeg and ffprobe must be installed so that the program can find them.

Release builds refresh bundled scripts in the user data directory's `scripts/`
folder on startup. Previous versions are backed up in `scripts-custom/backups/`.
Keep personal scripts in `scripts-custom/`; configuration and presets are preserved.
See [CLI documentation](docs/CLI.md) for paths and transcription configuration.
