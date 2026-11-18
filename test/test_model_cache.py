import importlib.util
import os
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "model_cache",
    Path(__file__).resolve().parents[1] / "src-tauri/assets/scripts/model_cache.py",
)
cache = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cache)


class ModelCacheTests(unittest.TestCase):
    def test_global_model_directory_is_used_by_both_backends(self):
        with patch.dict(os.environ, {"ADCONVERTER_TRANSCRIPT_MODEL_DIR": "/shared/models"}, clear=True):
            self.assertEqual(cache.get_model_download_root(), Path("/shared/models"))
            cache.configure_model_cache()
            self.assertEqual(os.environ["HF_HUB_CACHE"], "/shared/models")

    def test_unconfigured_cache_keeps_user_defaults_and_existing_hf_settings(self):
        with patch.dict(os.environ, {"HF_HUB_CACHE": "/other/hf-cache"}, clear=True), patch.object(cache.platform, "system", return_value="Linux"):
            self.assertEqual(cache.get_model_download_root(), Path.home() / ".cache/huggingface/hub")
            cache.configure_model_cache()
            self.assertEqual(os.environ["HF_HUB_CACHE"], "/other/hf-cache")

    def test_windows_default_is_beside_script(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(cache.platform, "system", return_value="Windows"):
            self.assertEqual(cache.get_model_download_root(), Path(cache.__file__).parent / ".cache/huggingface/hub")


if __name__ == "__main__":
    unittest.main()
