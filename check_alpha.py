#!/usr/bin/env python3
"""Decode an actual alpha plane (never force RGBA first and invent opacity).

Example: python check_alpha.py samples/overlay_prores.mov --time 2.5
All-black can be intentional on a fade frame. All-white is fully opaque at the
selected time. Test several times and composite against dark/light backgrounds.
"""
from __future__ import annotations
import argparse
import io
import json
import math
import shutil
import subprocess
from PIL import Image


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("video")
    ap.add_argument("--time", type=float, default=0)
    ap.add_argument("--ffmpeg", default="ffmpeg")
    args = ap.parse_args()
    if not math.isfinite(args.time) or args.time < 0:
        ap.error("time must be finite and nonnegative")
    ffmpeg = shutil.which(args.ffmpeg)
    if not ffmpeg:
        ap.error("FFmpeg not found")
    cmd = [ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-filter_threads", "1"]
    if args.video.lower().endswith(".webm"):
        cmd += ["-c:v", "libvpx-vp9"]
    cmd += ["-i", args.video, "-ss", str(args.time), "-vf", "alphaextract,format=gray",
            "-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "pipe:1"]
    r = subprocess.run(cmd, capture_output=True)
    if r.returncode or not r.stdout:
        ap.error("Cannot decode an alpha frame. Check timestamp, alpha data and decoder.\n" + r.stderr.decode("utf-8", errors="replace"))
    with Image.open(io.BytesIO(r.stdout)) as im:
        a = im.convert("L")
        lo, hi = a.getextrema()
        hist = a.histogram()
        pixels = a.width * a.height
    print(json.dumps({"time_seconds": args.time, "alpha_min_8bit": lo, "alpha_max_8bit": hi,
                      "transparent_pixel_percent": round(hist[0] / pixels * 100, 4),
                      "semi_transparent_pixel_percent": round(sum(hist[1:255]) / pixels * 100, 4),
                      "opaque_pixel_percent": round(hist[255] / pixels * 100, 4)}, indent=2))


if __name__ == "__main__":
    main()
