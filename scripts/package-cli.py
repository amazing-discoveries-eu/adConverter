"""Build native Linux CLI packages and portable macOS/Windows ZIPs."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile


def linux_packages(root, binary, version, output):
    packages = []
    with tempfile.TemporaryDirectory(prefix="adconverter-cli-") as temporary:
        work = Path(temporary)
        stage = work / "root"
        executable = stage / "usr/bin/adconverter-cli"
        executable.parent.mkdir(parents=True)
        shutil.copy2(binary, executable)
        executable.chmod(0o755)
        shutil.copytree(root / "src-tauri/assets", stage / "usr/share/adconverter-cli/assets")
        documentation = stage / "usr/share/doc/adconverter-cli"
        documentation.mkdir(parents=True)
        shutil.copy2(root / "docs/CLI.md", documentation / "README.md")
        shutil.copy2(root / "LICENSE", documentation / "copyright")

        # Let Debian determine the actual ELF library dependencies, including
        # those still linked through the shared desktop/CLI Rust library.
        (work / "debian").mkdir()
        (work / "debian/control").write_text(
            "Source: adconverter-cli\nMaintainer: Jonathan Baecker <jonbae77@gmail.com>\n"
            "\nPackage: adconverter-cli\nArchitecture: amd64\nDescription: adConverter CLI\n",
            encoding="utf-8",
        )
        dependencies = subprocess.check_output(
            ["dpkg-shlibdeps", "-O", "-e", str(executable)], cwd=work, text=True,
        ).strip().removeprefix("shlibs:Depends=")
        for kind, arch in [("deb", "amd64"), ("rpm", "x86_64")]:
            package = output / f"adconverter-cli_{version}_{arch}.{kind}"
            command = [
                "fpm", "--force", "-s", "dir", "-t", kind,
                "--name", "adconverter-cli", "--version", version,
                "--architecture", arch, "--license", "GPL-3.0-only",
                "--maintainer", "Jonathan Baecker <jonbae77@gmail.com>",
                "--description", "adConverter command-line encoder with presets and scripts",
                "--url", "https://github.com/amazing-discoveries-eu/adConverter",
                "--depends", "ffmpeg", "--depends", "python3",
                "--package", str(package), "--chdir", str(stage),
            ]
            if kind == "deb":
                for dependency in dependencies.split(","):
                    if dependency.strip():
                        command.extend(["--depends", dependency.strip()])
            else:
                command.append("--rpm-autoreq")
            subprocess.run([*command, "usr"], check=True)
            packages.append(package)
    return packages


def package(root, target, version):
    release = root / "src-tauri/target"
    if target.endswith("-apple-darwin"):
        binary = (
            release / target / "release/bundle/macos"
            / "adConverter.app/Contents/MacOS/adConverter-cli"
        )
        subprocess.run(["codesign", "--verify", "--strict", str(binary)], check=True)
    else:
        name = "adConverter-cli.exe" if "windows" in target else "adConverter-cli"
        binary = release / "release" / name
    if not binary.is_file():
        raise FileNotFoundError(binary)
    output = root / "cli-dist"
    output.mkdir(exist_ok=True)
    if "linux" in target:
        return linux_packages(root, binary, version, output)

    name = f"adConverter-cli_{version}_{target}"
    archive = output / f"{name}.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
        bundle.write(binary, f"{name}/{binary.name}")
        bundle.write(root / "docs/CLI.md", f"{name}/README.md")
        bundle.write(root / "LICENSE", f"{name}/LICENSE")
        for asset in sorted((root / "src-tauri/assets").rglob("*")):
            if asset.is_file():
                bundle.write(asset, f"{name}/assets/{asset.relative_to(root / 'src-tauri/assets').as_posix()}")
    return [archive]


if __name__ == "__main__":
    packages = package(
        Path(__file__).resolve().parent.parent,
        os.environ["CLI_TARGET"], os.environ["APP_VERSION"],
    )
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"directory={packages[0].parent.as_posix()}\n")
        if len(packages) == 1:
            output.write(f"archive={packages[0].as_posix()}\n")
    for artifact in packages:
        print(f"Packaged {artifact.name}")
