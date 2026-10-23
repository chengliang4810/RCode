"""Regression coverage for release completeness, versions and architecture."""

import importlib.util
import io
import json
from pathlib import Path
import plistlib
import struct
import tarfile
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("prepare_release", Path(__file__).with_name("prepare-release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
VERSION = "0.0.1-beta"


class ReleasePackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in release.required_assets(VERSION):
            (self.root / name).write_bytes(b"fixture")
        self.linux("x86_64")
        self.linux("aarch64")
        self.mac("x86_64")
        self.mac("arm64")
        binary = bytearray(256)
        binary[:2] = b"MZ"
        struct.pack_into("<I", binary, 0x3C, 0x80)
        binary[0x80:0x86] = b"PE\0\0\x64\x86"
        self.asset("windows-x86_64.exe").write_bytes(binary)
        with zipfile.ZipFile(self.asset("windows-x86_64.zip"), "w") as archive:
            archive.writestr("rcode.exe", binary)
            archive.writestr("rcode-update.json", json.dumps({"releases_url": "https://example.invalid/releases"}))
            for name in release.ROOT_NOTICES:
                archive.writestr(name, "license")
            for name in release.FONT_LICENSES:
                archive.writestr(f"licenses/fonts/{name}", "font license")
            archive.writestr("licenses/icons/LICENSE.symbols", "icon license")

    def asset(self, suffix):
        return self.root / f"rcode-{VERSION}-{suffix}"

    def linux(self, arch, machine=None, version=VERSION, omit=None, mode=0o755):
        prefix = f"rcode-{VERSION}-linux-{arch}"
        header = bytearray(64)
        header[:7] = b"\x7fELF\x02\x01\x01"
        struct.pack_into("<HH", header, 16, 3, machine or {"x86_64": 62, "aarch64": 183}[arch])
        files = [("rcode", bytes(header), mode), ("rcode-version", version.encode(), 0o644),
                 ("install.sh", b"#!/bin/sh\n", 0o755), ("rcode.desktop", b"desktop", 0o644),
                 ("rcode.png", b"icon", 0o644)]
        files += [(name, b"license", 0o644) for name in release.ROOT_NOTICES]
        files += [(f"licenses/fonts/{name}", b"license", 0o644) for name in release.FONT_LICENSES]
        files += [("licenses/icons/LICENSE.symbols", b"license", 0o644)]
        with tarfile.open(self.asset(f"linux-{arch}.tar.gz"), "w:gz") as archive:
            for name, data, permissions in files:
                if name == omit:
                    continue
                info = tarfile.TarInfo(f"{prefix}/{name}")
                info.size, info.mode = len(data), permissions
                archive.addfile(info, io.BytesIO(data))

    def mac(self, arch, cpu=None, version=VERSION):
        cpu = cpu or {"x86_64": 0x01000007, "arm64": 0x0100000C}[arch]
        plist = {"RCodeVersion": version, "CFBundleShortVersionString": "0.0.1", "CFBundleVersion": "0.0.1"}
        with tarfile.open(self.asset(f"macos-{arch}-app.tar.gz"), "w:gz") as archive:
            files = [
                ("MacOS/rcode", struct.pack("<II", 0xFEEDFACF, cpu), 0o755),
                ("Info.plist", plistlib.dumps(plist), 0o644),
            ]
            files += [(f"Resources/licenses/{name}", b"license", 0o644) for name in release.ROOT_NOTICES]
            files += [(f"Resources/licenses/fonts/{name}", b"license", 0o644) for name in release.FONT_LICENSES]
            files += [("Resources/licenses/icons/LICENSE.symbols", b"license", 0o644)]
            for name, data, mode in files:
                info = tarfile.TarInfo(f"RCode.app/Contents/{name}")
                info.size, info.mode = len(data), mode
                archive.addfile(info, io.BytesIO(data))

    def test_mobile_math_license_copies_match_sources(self):
        root = Path(__file__).resolve().parent.parent
        for source in (root / "crates/zatex/fonts").glob("*.txt"):
            copy = root / "apps/mobile/assets/licenses/math" / source.name
            self.assertEqual(copy.read_bytes(), source.read_bytes(), source.name)

    def test_all_platforms_generate_matching_metadata(self):
        manifest = release.prepare_release(self.root, VERSION, f"v{VERSION}")
        self.assertEqual(manifest["version"], VERSION)
        self.assertEqual(set(manifest["files"]), set(release.required_assets(VERSION)))
        for name, meta in manifest["files"].items():
            with (self.root / name).open("rb") as stream:
                self.assertEqual(meta["sha256"], release.sha256(stream))
        self.assertEqual((self.root / "latest.txt").read_text().strip(), VERSION)
        self.assertEqual(json.loads((self.root / "manifest.json").read_text()), manifest)

    def test_incomplete_release_never_writes_manifest(self):
        for suffix in ["macos-x86_64.dmg", "macos-arm64-app.tar.gz", "windows-x86_64-setup.exe"]:
            with self.subTest(suffix=suffix):
                path = self.asset(suffix)
                data = path.read_bytes()
                path.unlink()
                with self.assertRaisesRegex(ValueError, "Missing release artifacts"):
                    release.prepare_release(self.root, VERSION)
                self.assertFalse((self.root / "manifest.json").exists())
                path.write_bytes(data)

    def test_legacy_or_different_tag_is_rejected(self):
        for tag in ["beta-0.0.1", "v0.0.2-beta"]:
            with self.subTest(tag=tag), self.assertRaisesRegex(ValueError, "must match Cargo.toml"):
                release.prepare_release(self.root, VERSION, tag)

    def test_linux_invalid_archive_is_rejected_without_manifest(self):
        self.asset("linux-x86_64.tar.gz").write_bytes(b"not a valid archive")
        with self.assertRaisesRegex(ValueError, "Invalid Linux package"):
            release.prepare_release(self.root, VERSION)
        self.assertFalse((self.root / "manifest.json").exists())

    def test_linux_wrong_architecture_is_rejected(self):
        self.linux("x86_64", machine=183)
        with self.assertRaisesRegex(ValueError, "Wrong ELF architecture"):
            release.prepare_release(self.root, VERSION)

    def test_linux_wrong_version_is_rejected(self):
        self.linux("aarch64", version="0.0.2-beta")
        with self.assertRaisesRegex(ValueError, "Wrong Linux package version"):
            release.prepare_release(self.root, VERSION)

    def test_linux_required_layout_and_permissions(self):
        for missing in ["install.sh", "licenses/fonts/OFL-FiraMath.txt", "rcode.desktop"]:
            with self.subTest(missing=missing):
                self.linux("x86_64", omit=missing)
                with self.assertRaisesRegex(ValueError, "Invalid Linux package"):
                    release.prepare_release(self.root, VERSION)
        self.linux("x86_64", mode=0o644)
        with self.assertRaisesRegex(ValueError, "Invalid Linux package file: rcode"):
            release.prepare_release(self.root, VERSION)

    def test_wrong_mac_architecture_is_rejected(self):
        self.mac("x86_64", cpu=0x0100000C)
        with self.assertRaisesRegex(ValueError, "Wrong Mach-O architecture"):
            release.prepare_release(self.root, VERSION)

    def test_wrong_bundle_version_is_rejected(self):
        self.mac("arm64", version="0.0.2-beta")
        with self.assertRaisesRegex(ValueError, "Wrong bundle version"):
            release.prepare_release(self.root, VERSION)

    def test_windows_architecture_is_rejected(self):
        path = self.asset("windows-x86_64.exe")
        data = bytearray(path.read_bytes())
        data[0x84:0x86] = b"\x64\xaa"
        path.write_bytes(data)
        with self.assertRaisesRegex(ValueError, "not x86_64 PE"):
            release.prepare_release(self.root, VERSION)

    def test_portable_and_updater_must_be_identical(self):
        with self.asset("windows-x86_64.exe").open("ab") as stream:
            stream.write(b"different build")
        with self.assertRaisesRegex(ValueError, "executables differ"):
            release.prepare_release(self.root, VERSION)


if __name__ == "__main__":
    unittest.main()
