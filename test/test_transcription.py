"""Exercise transcription failures and the actual MLX child protocol without models."""

from contextlib import redirect_stdout
import importlib.util
import io
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / "src-tauri/assets/scripts"
with patch.object(sys, "path", [str(SCRIPTS), *sys.path]):
    spec = importlib.util.spec_from_file_location("transcript_cli", SCRIPTS / "transcript_cli.py")
    script = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(script)


class TranscriptionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="adconverter-transcription-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "source's folder" / "video's name.mp4"
        self.source.parent.mkdir()
        self.source.write_bytes(b"fake media")
        self.output = self.root / "requested output"
        self.args = ["-f", str(self.source), "-o", str(self.output), "-l", "de"]

    def cpu_modules(self, segments):
        model = SimpleNamespace(transcribe=lambda *a, **k: (segments, SimpleNamespace(duration=1)))
        return {
            "torch": SimpleNamespace(cuda=SimpleNamespace(is_available=lambda: False)),
            "faster_whisper": SimpleNamespace(
                WhisperModel=lambda *a, **k: model,
                BatchedInferencePipeline=lambda model: model,
                format_timestamp=lambda t: f"00:00:{t:06.3f}",
            ),
        }

    def test_cpu_inference_failure_returns_nonzero_and_removes_partial_output(self):
        def segments():
            yield SimpleNamespace(start=0, end=0.5, text="Partial")
            raise RuntimeError("inference failed")

        self.output.mkdir()
        previous = self.output / self.source.with_suffix(".vtt").name
        previous.write_text("previous subtitles")
        stdout = io.StringIO()
        with patch.dict(sys.modules, self.cpu_modules(segments())), patch.object(script.platform, "system", return_value="Linux"), redirect_stdout(stdout), self.assertLogs(script.log, level="ERROR"):
            status = script.main(self.args)
        self.assertEqual(status, 1)
        self.assertEqual(previous.read_text(), "previous subtitles")
        self.assertFalse(list(self.output.glob(".adconverter-*")))
        self.assertFalse(self.source.with_suffix(".lock").exists())
        self.assertNotIn("100", stdout.getvalue().splitlines())

    def test_cpu_success_uses_requested_output_directory(self):
        segments = [SimpleNamespace(start=0, end=1, text="Complete")]
        with patch.dict(sys.modules, self.cpu_modules(segments)), patch.object(script.platform, "system", return_value="Linux"), redirect_stdout(io.StringIO()):
            self.assertEqual(script.main(self.args), 0)
        result = self.output / self.source.with_suffix(".vtt").name
        self.assertIn("Complete", result.read_text())
        self.assertFalse(self.source.with_suffix(".vtt").exists())

    def fake_mlx(self):
        module = self.root / "mlx_whisper.py"
        module.write_text('''
import os
from pathlib import Path

def transcribe(**kwargs):
    if os.environ.get("FAKE_MLX_FAIL"):
        raise RuntimeError("simulated MLX failure")
    return {"text": kwargs["audio"]}

class writers:
    @staticmethod
    def get_writer(output_format, output_dir):
        def write(result, source):
            if not os.environ.get("FAKE_MLX_EMPTY"):
                output = Path(output_dir) / Path(source).with_suffix(".vtt").name
                output.write_text("WEBVTT\\n\\n00:00:00.000 --> 00:00:01.000\\n" + result["text"] + "\\n")
        return write
''')
        return {"PYTHONPATH": str(self.root), "FAKE_MLX_FAIL": "", "FAKE_MLX_EMPTY": ""}

    def test_mlx_child_handles_quotes_spaces_and_output_directory(self):
        with patch.dict(os.environ, self.fake_mlx()), patch.object(script.platform, "system", return_value="Darwin"), redirect_stdout(io.StringIO()):
            self.assertEqual(script.main(self.args), 0)
        result = self.output / self.source.with_suffix(".vtt").name
        self.assertIn(str(self.source), result.read_text())
        self.assertFalse(self.source.with_suffix(".vtt").exists())
        self.assertFalse(self.source.with_suffix(".lock").exists())

    def test_mlx_failure_and_missing_output_never_report_success(self):
        self.output.mkdir()
        previous = self.output / self.source.with_suffix(".vtt").name
        previous.write_text("previous subtitles")
        for failure in ["FAKE_MLX_FAIL", "FAKE_MLX_EMPTY"]:
            environment = self.fake_mlx()
            environment[failure] = "1"
            stdout = io.StringIO()
            with self.subTest(failure=failure), patch.dict(os.environ, environment), patch.object(script.platform, "system", return_value="Darwin"), redirect_stdout(stdout), self.assertLogs(script.log, level="INFO") as logs:
                self.assertEqual(script.main(self.args), 1)
            self.assertFalse(any("Transcription completed" in line for line in logs.output))
            self.assertNotIn("100", stdout.getvalue().splitlines())
            self.assertFalse(self.source.with_suffix(".lock").exists())
            self.assertEqual(previous.read_text(), "previous subtitles")
            self.assertFalse(list(self.output.glob(".adconverter-*")))

    def test_container_payload_can_run_help_without_optional_libraries(self):
        image = self.root / "image"
        image.mkdir()
        copied = []
        for line in (SCRIPTS / "Dockerfile").read_text().splitlines():
            if line.startswith("COPY "):
                for source in line.split()[1:-1]:
                    if source.endswith(".py"):
                        shutil.copy2(SCRIPTS / source, image)
                        copied.append(Path(source).name)
        self.assertIn("model_cache.py", copied)
        self.assertIn("transcript_cli.py", copied)
        result = subprocess.run([sys.executable, "-S", str(image / "transcript_cli.py"), "--help"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Transcribe video file", result.stdout)


if __name__ == "__main__":
    unittest.main()
