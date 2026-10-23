"""Verify the complete desktop release before generating updater metadata."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import struct
import tarfile
import tomllib
import zipfile


FONT_LICENSES = (
    "Geist-OFL.txt", "Sarasa-OFL.txt", "GUST-FONT-LICENSE.txt",
    "OFL-EulerMath.txt", "OFL-FiraMath.txt", "OFL-LibertinusMath.txt",
    "OFL-STIXTwoMath.txt",
)
ROOT_NOTICES = ("LICENSE", "LICENSE-APACHE", "THIRD_PARTY_NOTICES.md")


def required_assets(version):
    platforms = [
        "linux-x86_64.tar.gz", "linux-aarch64.tar.gz",
        "macos-x86_64.dmg", "macos-x86_64-app.tar.gz",
        "macos-arm64.dmg", "macos-arm64-app.tar.gz",
        "windows-x86_64.zip", "windows-x86_64.exe", "windows-x86_64-setup.exe",
    ]
    return [f"rcode-{version}-{platform}" for platform in platforms] + ["install.sh"]


def validate_tag(version, tag):
    if tag is not None and tag != f"v{version}":
        raise ValueError(f"Tag {tag!r} must match Cargo.toml: v{version}")


def sha256(stream):
    return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_mac(path, arch, version):
    with tarfile.open(path, "r:gz") as archive:
        with archive.extractfile("RCode.app/Contents/MacOS/rcode") as binary:
            magic, cpu = struct.unpack("<II", binary.read(8))
        expected_cpu = {"x86_64": 0x01000007, "arm64": 0x0100000C}[arch]
        if magic != 0xFEEDFACF or cpu != expected_cpu:
            raise ValueError(f"Wrong Mach-O architecture in {path.name}: expected {arch}")
        with archive.extractfile("RCode.app/Contents/Info.plist") as source:
            plist = plistlib.load(source)
        numeric = version.split("-")[0]
        if (plist.get("RCodeVersion") != version
                or plist.get("CFBundleShortVersionString") != numeric
                or plist.get("CFBundleVersion") != numeric):
            raise ValueError(f"Wrong bundle version in {path.name}")
        for name in ROOT_NOTICES:
            archive.getmember(f"RCode.app/Contents/Resources/licenses/{name}")
        for name in FONT_LICENSES:
            archive.getmember(f"RCode.app/Contents/Resources/licenses/fonts/{name}")
        archive.getmember("RCode.app/Contents/Resources/licenses/icons/LICENSE.symbols")
        binary_info = archive.getmember("RCode.app/Contents/MacOS/rcode")
        if not binary_info.isfile() or not binary_info.mode & 0o111:
            raise ValueError(f"App binary is not executable in {path.name}")


def verify_linux(path, arch, version):
    prefix = f"rcode-{version}-linux-{arch}"
    try:
        with tarfile.open(path, "r:gz") as archive:
            seen = set()
            for member in archive.getmembers():
                parts = member.name.split("/")
                if (member.name in seen or parts[0] != prefix
                        or any(part in ("", ".", "..") for part in parts)
                        or not (member.isfile() or member.isdir())):
                    raise ValueError(f"Unsafe or duplicate Linux package entry: {member.name}")
                seen.add(member.name)

            def required(name, executable=False):
                member = archive.getmember(f"{prefix}/{name}")
                if not member.isfile() or member.size == 0 or (executable and not member.mode & 0o111):
                    raise ValueError(f"Invalid Linux package file: {name}")
                return archive.extractfile(member)

            with required("rcode", executable=True) as binary:
                header = binary.read(64)
            expected_machine = {"x86_64": 62, "aarch64": 183}[arch]
            if (len(header) < 64 or header[:7] != b"\x7fELF\x02\x01\x01"
                    or struct.unpack_from("<H", header, 16)[0] not in (2, 3)
                    or struct.unpack_from("<H", header, 18)[0] != expected_machine):
                raise ValueError(f"Wrong ELF architecture in {path.name}: expected {arch}")
            with required("rcode-version") as marker:
                if marker.read(256).decode("utf-8").strip() != version:
                    raise ValueError(f"Wrong Linux package version in {path.name}")
            for name in ("rcode.desktop", "rcode.png", *ROOT_NOTICES):
                required(name).close()
            required("install.sh", executable=True).close()
            for name in FONT_LICENSES:
                required(f"licenses/fonts/{name}").close()
            required("licenses/icons/LICENSE.symbols").close()
    except (tarfile.TarError, KeyError, UnicodeError) as error:
        raise ValueError(f"Invalid Linux package {path.name}: {error}") from error


def verify_windows(directory, version):
    exe = directory / f"rcode-{version}-windows-x86_64.exe"
    with exe.open("rb") as binary:
        if binary.read(2) != b"MZ":
            raise ValueError("Windows updater executable has no DOS header")
        binary.seek(0x3C)
        pe_offset = struct.unpack("<I", binary.read(4))[0]
        binary.seek(pe_offset)
        if binary.read(4) != b"PE\0\0" or binary.read(2) != b"\x64\x86":
            raise ValueError("Windows updater executable is not x86_64 PE")
        binary.seek(0)
        exe_hash = sha256(binary)
    with zipfile.ZipFile(directory / f"rcode-{version}-windows-x86_64.zip") as archive:
        with archive.open("rcode.exe") as binary:
            if sha256(binary) != exe_hash:
                raise ValueError("Portable and updater Windows executables differ")
        marker = json.loads(archive.read("rcode-update.json"))
        if not marker.get("releases_url", "").startswith("https://"):
            raise ValueError("Portable package is missing its HTTPS update feed")
        for name in ROOT_NOTICES:
            archive.getinfo(name)
        for name in FONT_LICENSES:
            archive.getinfo(f"licenses/fonts/{name}")
        archive.getinfo("licenses/icons/LICENSE.symbols")


def prepare_release(directory, version, tag=None):
    validate_tag(version, tag)
    names = required_assets(version)
    missing = [name for name in names
               if not (directory / name).is_file() or (directory / name).stat().st_size == 0]
    if missing:
        raise ValueError("Missing release artifacts: " + ", ".join(missing))
    for arch in ["x86_64", "aarch64"]:
        verify_linux(directory / f"rcode-{version}-linux-{arch}.tar.gz", arch, version)
    for arch in ["x86_64", "arm64"]:
        verify_mac(directory / f"rcode-{version}-macos-{arch}-app.tar.gz", arch, version)
    verify_windows(directory, version)
    files = {}
    for name in names:
        with (directory / name).open("rb") as source:
            files[name] = {"sha256": sha256(source)}
    manifest = {"version": version, "files": files}
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    (directory / "latest.txt").write_text(version + "\n", encoding="utf-8")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path)
    parser.add_argument("--check-version", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    with (root / "Cargo.toml").open("rb") as source:
        version = tomllib.load(source)["workspace"]["package"]["version"]
    tag = os.environ.get("GITHUB_REF_NAME") if os.environ.get("GITHUB_REF_TYPE") == "tag" else None
    validate_tag(version, tag)
    if not args.check_version:
        if args.artifacts is None:
            parser.error("--artifacts is required unless --check-version is used")
        manifest = prepare_release(args.artifacts, version, tag)
        print(f"Verified {len(manifest['files'])} assets for {version}")
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as stream:
            stream.write(f"version={version}\n")
    print(f"Version: {version}")


if __name__ == "__main__":
    main()
