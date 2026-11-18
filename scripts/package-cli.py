"""Build native Linux CLI packages without desktop or external-tool requirements."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def linux_packages(root, binary, version, output):
    packages = []
    with tempfile.TemporaryDirectory(prefix="adconverter-cli-") as temporary:
        work = Path(temporary)
        stage = work / "root"
        executable = stage / "usr/bin/adconverter-cli"
        executable.parent.mkdir(parents=True)
        shutil.copy2(binary, executable)
        executable.chmod(0o755)
        shutil.copytree(
            root / "src-tauri/assets", stage / "usr/share/adconverter-cli/assets",
            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
        )
        documentation = stage / "usr/share/doc/adconverter-cli"
        documentation.mkdir(parents=True)
        shutil.copy2(root / "docs/CLI.md", documentation / "README.md")
        shutil.copy2(root / "docs/cli-config.example.json", documentation / "config.example.json")
        shutil.copy2(root / "LICENSE", documentation / "copyright")

        # Keep real ELF requirements; GUI libraries must never enter this build.
        (work / "debian").mkdir()
        (work / "debian/control").write_text(
            "Source: adconverter-cli\nMaintainer: Jonathan Baecker <jonbae77@gmail.com>\n"
            "\nPackage: adconverter-cli\nArchitecture: amd64\nDescription: adConverter CLI\n",
            encoding="utf-8",
        )
        dependencies = subprocess.check_output(
            ["dpkg-shlibdeps", "-O", "-e", str(executable)], cwd=work, text=True,
        ).strip().removeprefix("shlibs:Depends=")
        reject_desktop_dependencies(dependencies)
        for kind, arch in [("deb", "amd64"), ("rpm", "x86_64")]:
            package = output / f"adconverter-cli_{version}_{arch}.{kind}"
            command = [
                "fpm", "--force", "-s", "dir", "-t", kind,
                "--name", "adconverter-cli", "--version", version,
                "--architecture", arch, "--license", "GPL-3.0-only",
                "--maintainer", "Jonathan Baecker <jonbae77@gmail.com>",
                "--description", "adConverter command-line encoder with presets and scripts",
                "--url", "https://github.com/amazing-discoveries-eu/adConverter",
                "--package", str(package), "--chdir", str(stage),
            ]
            if kind == "deb":
                for dependency in dependencies.split(","):
                    if dependency.strip():
                        command.extend(["--depends", dependency.strip()])
            else:
                command.append("--rpm-autoreq")
            subprocess.run([*command, "usr"], check=True)
            if kind == "rpm":
                requirements = subprocess.check_output(
                    ["rpm", "-qp", "--requires", str(package)], text=True,
                )
                reject_desktop_dependencies(requirements)
            packages.append(package)
    return packages


def reject_desktop_dependencies(dependencies):
    forbidden = ("webkit", "javascriptcore", "libsoup", "libgtk", "libgdk")
    if any(name in dependencies.lower() for name in forbidden):
        raise RuntimeError(f"Desktop library in standalone CLI dependencies: {dependencies}")


def package(root, target, version):
    if target != "x86_64-unknown-linux-gnu":
        raise ValueError(f"Standalone CLI packages are only built for Linux: {target}")
    binary = root / "src-tauri/target/cli/release/adConverter-cli"
    if not binary.is_file():
        raise FileNotFoundError(binary)
    output = root / "cli-dist"
    output.mkdir(exist_ok=True)
    return linux_packages(root, binary, version, output)


if __name__ == "__main__":
    packages = package(
        Path(__file__).resolve().parent.parent,
        os.environ["CLI_TARGET"], os.environ["APP_VERSION"],
    )
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"directory={packages[0].parent.as_posix()}\n")
    for artifact in packages:
        print(f"Packaged {artifact.name}")
