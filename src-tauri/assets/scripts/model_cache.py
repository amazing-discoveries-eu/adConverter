"""Model cache selection shared by the transcription entry point and its tests."""

import os
from pathlib import Path
import platform


def get_model_download_root() -> Path:
    configured = os.environ.get("ADCONVERTER_TRANSCRIPT_MODEL_DIR")
    if configured:
        return Path(configured).expanduser()
    if platform.system() == "Windows":
        return Path(__file__).resolve().parent / ".cache" / "huggingface" / "hub"
    return Path.home() / ".cache" / "huggingface" / "hub"


def configure_model_cache():
    # Hugging Face reads this during import; set it before importing MLX Whisper.
    if os.environ.get("ADCONVERTER_TRANSCRIPT_MODEL_DIR"):
        os.environ["HF_HUB_CACHE"] = str(get_model_download_root())
