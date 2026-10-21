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
            for name in ["LICENSE", "LICENSE-APACHE", "THIRD_PARTY_NOTICES.md"]:
                archive.writestr(name, "license")

    def asset(self, suffix):
        return self.root / f"rcode-{VERSION}-{suffix}"

    def mac(self, arch, cpu=None, version=VERSION):
        cpu = cpu or {"x86_64": 0x01000007, "arm64": 0x0100000C}[arch]
        plist = {"RCodeVersion": version, "CFBundleShortVersionString": "0.0.1", "CFBundleVersion": "0.0.1"}
        with tarfile.open(self.asset(f"macos-{arch}-app.tar.gz"), "w:gz") as archive:
            for name, data, mode in [
                ("MacOS/rcode", struct.pack("<II", 0xFEEDFACF, cpu), 0o755),
                ("Info.plist", plistlib.dumps(plist), 0o644),
            ]:
                info = tarfile.TarInfo(f"RCode.app/Contents/{name}")
                info.size, info.mode = len(data), mode
                archive.addfile(info, io.BytesIO(data))

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
