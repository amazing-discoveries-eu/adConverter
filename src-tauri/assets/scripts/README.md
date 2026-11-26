# Transcription container

When this directory is the copy in adConverter's user data directory, bundled
files are automatically refreshed when a release build starts. Keep personal scripts
in the neighboring `scripts-custom/` directory. Replaced versions are preserved
under `scripts-custom/backups/`.

Run these commands from this directory. The image includes both
`transcript_cli.py` and its `model_cache.py` helper; no script mounts are needed.

```bash
docker build -t adconverter-whisper .
```

For NVIDIA execution, configure the NVIDIA Container Toolkit on the host:

```bash
docker run --rm --gpus all \
  -v /path/to/media:/media \
  -v /path/to/output:/output \
  -v /path/to/model-cache:/models \
  -e ADCONVERTER_TRANSCRIPT_MODEL_DIR=/models \
  adconverter-whisper -d cuda -c float16 -l de -f /media/video.mp4 -o /output
```

For CPU execution, omit `--gpus all` and use `-d cpu -c int8`. If `-d` is omitted,
CTranslate2 selects CUDA when it sees a GPU and otherwise selects the CPU.
The image uses CUDA 12.9.2 and cuDNN 9. The separate `requirements-torch.txt`
is only for the PyTorch dependency of the standalone `whisper.py` script.

For adConverter's `transcript_cmd`, use this command as a single line:

```text
docker run --rm --gpus all -v %mount%:%mount% -v %output%:%output% -v /var/cache/adconverter/models:/models -e ADCONVERTER_TRANSCRIPT_MODEL_DIR=/models adconverter-whisper -d cuda -c float16 -l %lang% -f %file% -o %output%
```

Both the directory containing the actual transcription input (`%mount%`) and the per-job output directory (`%output%`)
must be mounted at the same paths inside the container. adConverter chooses a
fresh output directory for each job and removes it afterwards. `%output%` is an
output directory, not a file. When encoding supplies the transcription input,
`%mount%` follows that file to the encoding destination. The media directory must permit the script's
short-lived `.lock` file; container users also need access to the model cache.
Use appropriate volume labels on hosts enforcing SELinux.

Container environment variables are separate from the host process environment.
The example mounts a shared cache explicitly and selects its container path with
`ADCONVERTER_TRANSCRIPT_MODEL_DIR`; configure these paths to match your setup.
If you override the packaged script with a bind mount, mount `model_cache.py`
next to it too, or mount the whole scripts directory.

The script returns a nonzero exit code if transcription fails. macOS/MLX and
Linux/Windows/Faster Whisper both honor `-o` and report completion only after
successfully producing the VTT file. `--help` works without the model libraries.
