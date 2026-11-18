"""Check CLI payloads and native package metadata without Linux package tools."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location(
    "package_cli", Path(__file__).resolve().parents[1] / "scripts/package-cli.py"
)
packager = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packager)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for name, contents in {
            "src-tauri/assets/config.json": "{}",
            "src-tauri/assets/presets/mp3.json": "preset",
            "src-tauri/assets/scripts/transcript.py": "script",
            "docs/CLI.md": "instructions",
            "LICENSE": "license",
        }.items():
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(contents)

    def binary(self, name):
        binary = self.root / "src-tauri/target/release" / name
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"test executable")
        binary.chmod(0o755)
        return binary

    def test_windows_zip_contains_all_assets(self):
        self.binary("adConverter-cli.exe")
        [archive] = packager.package(self.root, "x86_64-pc-windows-msvc", "1.2.3")
        with zipfile.ZipFile(archive) as bundle:
            prefix = "adConverter-cli_1.2.3_x86_64-pc-windows-msvc/"
            self.assertEqual(bundle.read(prefix + "assets/presets/mp3.json"), b"preset")
            self.assertIn(prefix + "assets/config.json", bundle.namelist())
            self.assertIn(prefix + "assets/scripts/transcript.py", bundle.namelist())
            self.assertIn(prefix + "adConverter-cli.exe", bundle.namelist())

    def test_linux_packages_install_assets_and_declare_dependencies(self):
        self.binary("adConverter-cli")
        commands = []

        def build(command, **kwargs):
            commands.append(command)
            stage = Path(command[command.index("--chdir") + 1])
            self.assertEqual((stage / "usr/bin/adconverter-cli").read_bytes(), b"test executable")
            self.assertEqual((stage / "usr/share/adconverter-cli/assets/presets/mp3.json").read_text(), "preset")
            self.assertTrue((stage / "usr/share/adconverter-cli/assets/scripts/transcript.py").is_file())
            self.assertTrue((stage / "usr/share/adconverter-cli/assets/config.json").is_file())
            Path(command[command.index("--package") + 1]).touch()

        with patch.object(packager.subprocess, "check_output", return_value="shlibs:Depends=libc6 (>= 2.34), libwebkit2gtk-4.1-0\n"), patch.object(packager.subprocess, "run", side_effect=build):
            packages = packager.package(self.root, "x86_64-unknown-linux-gnu", "1.2.3")
        self.assertEqual([p.suffix for p in packages], [".deb", ".rpm"])
        self.assertTrue(all(p.is_file() for p in packages))
        self.assertIn("libc6 (>= 2.34)", commands[0])
        self.assertIn("libwebkit2gtk-4.1-0", commands[0])
        self.assertIn("--rpm-autoreq", commands[1])
        for command in commands:
            self.assertIn("ffmpeg", command)
            self.assertIn("python3", command)
            self.assertIn("adconverter-cli", command)

    def test_macos_uses_signed_bundle_binary(self):
        binary = self.root / "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/adConverter.app/Contents/MacOS/adConverter-cli"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"signed binary")
        binary.chmod(0o755)
        with patch.object(packager.subprocess, "run") as codesign:
            [archive] = packager.package(self.root, "aarch64-apple-darwin", "1.2.3")
        codesign.assert_called_once_with(["codesign", "--verify", "--strict", str(binary)], check=True)
        with zipfile.ZipFile(archive) as bundle:
            entry = bundle.getinfo("adConverter-cli_1.2.3_aarch64-apple-darwin/adConverter-cli")
            self.assertEqual((entry.external_attr >> 16) & 0o777, 0o755)


if __name__ == "__main__":
    unittest.main()
