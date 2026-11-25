#!/usr/bin/env python3

from contextlib import contextmanager
import json
import logging
import platform
import re
import subprocess
import sys
import tempfile
import time
from argparse import ArgumentParser
from pathlib import Path

from model_cache import configure_model_cache, get_model_download_root

MODEL_NAME = (
    "mlx-community/whisper-large-v3-mlx" if platform.system() == "Darwin" else "large-v3"
)
ALLOWED_EXTENSIONS = [".mp4", ".mp3", ".mov", ".mkv", ".webm"]
EXCLUDE_DIRS = [
    "_NICHT SENDEN", "00-assets", "00-audio", "00-social-share",
    "Black Error", "03 - Musikalische Lückenfüller",
]
LOCK_EXT = ".lock"
log = logging.getLogger(__name__)


def parse_args(argv=None):
    parser = ArgumentParser(description="Transcribe video file")
    parser.add_argument("-b", help="Use batched transcription", action="store_true")
    parser.add_argument("-c", metavar="compute_type", default="default",
                        help="Whisper compute type, e.g. int8 or float16")
    parser.add_argument("-d", metavar="device", default=None,
                        help="Whisper device (cuda or cpu)")
    parser.add_argument("-l", metavar="language", default="auto")
    parser.add_argument("-f", metavar="file", required=True, nargs="+", type=Path)
    parser.add_argument("-p", help="Print status in percent", action="store_true")
    parser.add_argument("-o", metavar="output", type=Path)
    return parser.parse_args(argv)


def is_locked(video_path):
    return video_path.with_suffix(LOCK_EXT).is_file()


def is_transcribed(video_path):
    return video_path.with_suffix(".vtt").is_file()


def lock_video(video_path):
    video_path.with_suffix(LOCK_EXT).write_text(str(time.time()))


def unlock_video(video_path):
    video_path.with_suffix(LOCK_EXT).unlink(missing_ok=True)


def output_path(video_path, args):
    directory = args.o if args.o is not None else video_path.parent
    directory.mkdir(parents=True, exist_ok=True)
    return directory / video_path.with_suffix(".vtt").name


@contextmanager
def staged_output(final_path):
    # Keep an existing subtitle intact if inference or its child process fails.
    with tempfile.TemporaryDirectory(prefix=".adconverter-", dir=final_path.parent) as temporary:
        pending = Path(temporary) / final_path.name
        yield pending
        pending.replace(final_path)


def progress(value, args):
    if args.p:
        print(f"{value}%", end="\r", flush=True)
    else:
        print(value, flush=True)


def transcribe_video(video_path, args):
    # Load dependencies only when needed; --help works without a model runtime.
    from ctranslate2 import get_cuda_device_count
    from faster_whisper import BatchedInferencePipeline, WhisperModel, format_timestamp

    lang = None if args.l.lower() == "auto" else args.l
    device = args.d or ("cuda" if get_cuda_device_count() > 0 else "cpu")
    log.info("Whisper will run on %s", device)
    model = WhisperModel(
        MODEL_NAME, device=device, compute_type=args.c,
        download_root=get_model_download_root(),
    )
    engine = BatchedInferencePipeline(model=model) if args.b else model
    options = {"vad_filter": True}
    options.update({"multilingual": True} if lang == "ml" else {"language": lang})
    if args.b:
        options["batch_size"] = 16
    segments, info = engine.transcribe(video_path, **options)
    vtt_path = output_path(video_path, args)
    lock_video(video_path)
    log.info("Processing %s", video_path)
    elapsed = 0
    try:
        with staged_output(vtt_path) as pending:
            with pending.open("w", encoding="utf-8") as file:
                file.write("WEBVTT\n\n")
                for segment in segments:
                    file.write(f"\n{format_timestamp(segment.start)} --> {format_timestamp(segment.end)}\n")
                    file.write(f"{segment.text.strip()}\n")
                    elapsed += segment.end - segment.start
                    if info.duration > 0:
                        progress(min(99, int(elapsed / info.duration * 100)), args)
    finally:
        unlock_video(video_path)
    log.info("Transcription completed, saved to %s", vtt_path)
    progress(100, args)


MLX_CODE = """
import json
import sys
from mlx_whisper import transcribe, writers
source, output, model, language = sys.argv[1:]
result = transcribe(audio=source, path_or_hf_repo=model,
                    language=json.loads(language), verbose=False,
                    condition_on_previous_text=False)
writer = writers.get_writer(output_format='vtt', output_dir=output)
writer(result, source)
"""


def transcribe_video_mlx(video_path, args):
    lang = None if args.l.lower() == "auto" else args.l
    vtt_path = output_path(video_path, args)
    lock_video(video_path)
    log.info("Processing %s", video_path)
    try:
        with staged_output(vtt_path) as pending:
            command = [sys.executable, "-c", MLX_CODE, str(video_path),
                       str(pending.parent), MODEL_NAME, json.dumps(lang)]
            # Paths are arguments, not interpolated Python code. Discard stdout so
            # a verbose dependency cannot fill an unread pipe and block the child.
            with subprocess.Popen(command, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.PIPE, text=True) as process:
                try:
                    for line in process.stderr:
                        match = re.search(r"^(\d+)%", line.strip())
                        if match:
                            progress(min(99, int(match.group(1))), args)
                        elif line.strip():
                            log.info("%s", line.strip())
                    status = process.wait()
                except BaseException:
                    process.terminate()
                    process.wait()
                    raise
            if status != 0:
                raise RuntimeError(f"MLX transcription failed with exit code {status}")
            if not pending.is_file() or pending.stat().st_size == 0:
                raise RuntimeError(f"MLX transcription did not produce {vtt_path}")
    finally:
        unlock_video(video_path)
    log.info("Transcription completed, saved to %s", vtt_path)
    progress(100, args)


def input_files(paths):
    for path in paths:
        if path.is_file():
            yield path
        elif path.is_dir():
            for candidate in path.rglob("*"):
                if (candidate.is_file() and candidate.suffix.lower() in ALLOWED_EXTENSIONS
                        and not any(excluded in str(candidate) for excluded in EXCLUDE_DIRS)
                        and not is_locked(candidate) and not is_transcribed(candidate)):
                    yield candidate
        else:
            raise FileNotFoundError(path)


def main(argv=None):
    args = parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="[%(levelname)s] %(message)s")
    configure_model_cache()
    transcribe = transcribe_video_mlx if platform.system() == "Darwin" else transcribe_video
    try:
        failed = False
        for path in input_files(args.f):
            try:
                transcribe(path, args)
            except Exception as error:
                log.error("Failed to transcribe %s: %s", path, error)
                failed = True
        return 1 if failed else 0
    except KeyboardInterrupt:
        log.error("Transcription interrupted")
        return 130
    except Exception as error:
        log.error("Transcription failed: %s", error)
        return 1


if __name__ == "__main__":
    sys.exit(main())
