"""Validate native resource parity, format arguments, plurals and Swift key use.

Run with Python 3; no Xcode or device is required. UIKit rendering and system
permission prompts still need an iOS build and runtime check.
"""

from collections import Counter
import json
from pathlib import Path
import plistlib
import re


ROOT = Path(__file__).resolve().parents[1] / "RCode"
LOCALES = ("en", "zh-Hans")
ENTRY = re.compile(r'("(?:\\.|[^"\\])*")\s*=\s*("(?:\\.|[^"\\])*")\s*;')
FORMAT = re.compile(r"%(?:\d+\$)?(?:ld|llu|d|@)")


def strings(path):
    source = path.read_text("utf-8")
    result = {}
    for match in ENTRY.finditer(source):
        key, value = (json.loads(part) for part in match.groups())
        assert key not in result, f"{path}: duplicate {key}"
        result[key] = value
    rest = ENTRY.sub("", source)
    rest = re.sub(r"/\*.*?\*/", "", rest, flags=re.S).strip()
    assert not rest, f"{path}: unparsed resource text"
    return result


def formats(value):
    # %% is a literal percent. Positional arguments may reorder by locale.
    return Counter(re.sub(r"%\d+\$", "%", value) for value in FORMAT.findall(value.replace("%%", "")))


def validate():
    tables = {locale: strings(ROOT / f"{locale}.lproj/Localizable.strings") for locale in LOCALES}
    plurals = {locale: plistlib.loads((ROOT / f"{locale}.lproj/Localizable.stringsdict").read_bytes()) for locale in LOCALES}
    reference = tables["en"]
    for locale in LOCALES:
        assert reference.keys() == tables[locale].keys(), f"{locale}: translation keys differ"
        for key, english in reference.items():
            assert formats(english) == formats(tables[locale][key]), f"{locale}: format mismatch for {key}"
        assert plurals["en"].keys() == plurals[locale].keys(), f"{locale}: plural keys differ"
        for key, entry in plurals[locale].items():
            assert entry["NSStringLocalizedFormatKey"] == "%#@count@", key
            count = entry["count"]
            assert count["NSStringFormatSpecTypeKey"] == "NSStringPluralRuleType", key
            assert count["NSStringFormatValueTypeKey"] == "ld", key
            assert "other" in count, f"{locale}: missing plural fallback for {key}"
            for category in ("one", "other"):
                if category in count:
                    assert formats(count[category]) == Counter({"%ld": 1}), f"{locale}: plural argument mismatch for {key}"
        privacy = strings(ROOT / f"{locale}.lproj/InfoPlist.strings")
        assert privacy.keys() == {"NSCameraUsageDescription", "NSPhotoLibraryUsageDescription"}
    assert all("one" in entry["count"] for entry in plurals["en"].values()), "English plural missing singular"
    used = set()
    for path in ROOT.rglob("*.swift"):
        if "Generated" in path.parts or "Debug" in path.parts:
            continue
        source = path.read_text("utf-8")
        for kind, key in re.findall(r'L10n\.(text|format|count)\("([^"\\]+)"', source):
            assert key in (plurals["en"] if kind == "count" else reference), f"{path}: missing {key}"
            used.add(key)
    project = (ROOT.parent / "RCode.xcodeproj/project.pbxproj").read_text("utf-8")
    assert '"zh-Hans"' in project, "Chinese localization absent from Xcode knownRegions"
    print(f"iOS localization OK: {len(reference)} keys, {len(plurals['en'])} plural messages, {len(used)} referenced keys; permission resources match")


if __name__ == "__main__":
    validate()
