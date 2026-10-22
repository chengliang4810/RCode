#!/usr/bin/env python3
"""Resize the preserved RCode app artwork for Flutter's Android/iOS runners."""
import json
import shutil
from pathlib import Path

from PIL import Image


def main():
    root = Path(__file__).resolve().parents[2]
    mobile = root / "apps/mobile"
    source = mobile / "assets/app-icon.png"
    image = Image.open(source).convert("RGB")
    catalog = mobile / "ios/Runner/Assets.xcassets/AppIcon.appiconset"
    contents = json.loads((catalog / "Contents.json").read_text(encoding="utf-8"))
    outputs = {}
    for icon in contents["images"]:
        points = float(icon["size"].split("x")[0])
        scale = float(icon["scale"].removesuffix("x"))
        outputs[catalog / icon["filename"]] = round(points * scale)
    for density, pixels in {"mdpi": 48, "hdpi": 72, "xhdpi": 96,
                            "xxhdpi": 144, "xxxhdpi": 192}.items():
        outputs[mobile / f"android/app/src/main/res/mipmap-{density}/ic_launcher.png"] = pixels
    for destination, pixels in outputs.items():
        destination.parent.mkdir(parents=True, exist_ok=True)
        if image.size == (pixels, pixels):
            shutil.copyfile(source, destination)
        else:
            image.resize((pixels, pixels), Image.Resampling.LANCZOS).save(destination)
    print(f"Generated {len(outputs)} Android/iOS icons from the preserved RCode artwork.")


if __name__ == "__main__":
    main()
