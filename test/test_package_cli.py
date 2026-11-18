"""Check Linux CLI payloads and native package metadata without package tools."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

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
            "docs/cli-config.example.json": "{}",
            "LICENSE": "license",
            "src-tauri/target/cli/release/adConverter-cli": "headless executable",
            # A desktop executable must never be selected for standalone packages.
            "src-tauri/target/release/adConverter-cli": "desktop executable",
        }.items():
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(contents)

    def test_linux_packages_use_headless_binary_and_include_assets(self):
        commands = []

        def build(command, **kwargs):
            commands.append(command)
            stage = Path(command[command.index("--chdir") + 1])
            executable = stage / "usr/bin/adconverter-cli"
            self.assertEqual(executable.read_text(), "headless executable")
            self.assertEqual(executable.stat().st_mode & 0o777, 0o755)
            self.assertEqual((stage / "usr/share/adconverter-cli/assets/presets/mp3.json").read_text(), "preset")
            self.assertTrue((stage / "usr/share/adconverter-cli/assets/scripts/transcript.py").is_file())
            self.assertTrue((stage / "usr/share/adconverter-cli/assets/config.json").is_file())
            self.assertTrue((stage / "usr/share/doc/adconverter-cli/config.example.json").is_file())
            self.assertFalse((stage / "etc/adconverter/config.json").exists())
            Path(command[command.index("--package") + 1]).touch()

        with patch.object(packager.subprocess, "check_output", return_value="shlibs:Depends=libc6 (>= 2.34), libgcc-s1\n"), patch.object(packager.subprocess, "run", side_effect=build):
            packages = packager.package(self.root, "x86_64-unknown-linux-gnu", "1.2.3")
        self.assertEqual([p.suffix for p in packages], [".deb", ".rpm"])
        self.assertTrue(all(p.is_file() for p in packages))
        self.assertIn("libc6 (>= 2.34)", commands[0])
        self.assertIn("libgcc-s1", commands[0])
        self.assertIn("--rpm-autoreq", commands[1])
        for command in commands:
            self.assertNotIn("ffmpeg", command)
            self.assertNotIn("python3", command)
            self.assertIn("adconverter-cli", command)

    def test_rejects_gui_dependencies_instead_of_hiding_them(self):
        for dependency in ["libwebkit2gtk-4.1-0", "libjavascriptcoregtk-4.1.so.0()(64bit)", "libsoup-3.0.so.0", "libgtk-3.so.0"]:
            with self.subTest(dependency=dependency):
                with patch.object(packager.subprocess, "check_output", return_value=f"shlibs:Depends={dependency}\n"), patch.object(packager.subprocess, "run") as build:
                    with self.assertRaisesRegex(RuntimeError, "Desktop library"):
                        packager.package(self.root, "x86_64-unknown-linux-gnu", "1.2.3")
                    build.assert_not_called()

    def test_no_separate_macos_or_windows_packages(self):
        for target in ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc"]:
            with self.subTest(target=target), self.assertRaises(ValueError):
                packager.package(self.root, target, "1.2.3")

    def test_rejects_gui_requirements_added_by_rpm_autodetection(self):
        with patch.object(packager.subprocess, "check_output", side_effect=[
            "shlibs:Depends=libc6 (>= 2.34)\n",
            "libc.so.6()(64bit)\nlibwebkit2gtk-4.1.so.0()(64bit)\n",
        ]), patch.object(packager.subprocess, "run"):
            with self.assertRaisesRegex(RuntimeError, "Desktop library"):
                packager.package(self.root, "x86_64-unknown-linux-gnu", "1.2.3")


if __name__ == "__main__":
    unittest.main()
