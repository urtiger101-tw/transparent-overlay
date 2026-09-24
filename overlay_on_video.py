#!/usr/bin/env python3
"""Loop a straight-alpha overlay on an SDR base video, preserving base audio.

Output is a FINISHED opaque H.264 MP4, not another alpha master.
Requires FFmpeg with libx264; WebM alpha also requires libvpx-vp9.
"""
from __future__ import annotations

import argparse
import math
import json
from fractions import Fraction
import shutil
import subprocess
import sys
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("base", type=Path)
    ap.add_argument("overlay", type=Path)
    ap.add_argument("output", type=Path)
    ap.add_argument("--x", type=int, default=0, help="Left position in base-video pixels")
    ap.add_argument("--y", type=int, default=0, help="Top position in base-video pixels")
    ap.add_argument("--width", type=int, help="Optional overlay width; maintain aspect ratio")
    ap.add_argument("--start", type=float, default=0, help="Start time on base timeline, seconds")
    ap.add_argument("--end", type=float, help="Optional end time, seconds (exclusive)")
    ap.add_argument("--fps", help="Output CFR, e.g. 30 or 30000/1001; default reads base video's average frame rate")
    ap.add_argument("--ffprobe", help="Optional ffprobe executable path")
    ap.add_argument("--ffmpeg", default="ffmpeg")
    args = ap.parse_args()
    ffmpeg = shutil.which(args.ffmpeg)
    if not ffmpeg:
        ap.error("FFmpeg not found; install it or specify --ffmpeg")
    if not args.base.is_file() or not args.overlay.is_file():
        ap.error("Both input files must exist")
    if args.output.exists():
        ap.error("Output already exists; choose another filename")
    if args.output.suffix.lower() != ".mp4":
        ap.error("This helper exports an opaque .mp4 final video")
    if not math.isfinite(args.start) or args.start < 0:
        ap.error("start must be finite and nonnegative")
    if args.end is not None and (not math.isfinite(args.end) or args.end <= args.start):
        ap.error("end must be finite and greater than start")
    if args.width is not None and (args.width < 2 or args.width % 2):
        ap.error("width must be a positive even integer >= 2")
    fps = args.fps
    if fps is None:
        sibling = Path(ffmpeg).with_name("ffprobe.exe" if Path(ffmpeg).suffix.lower() == ".exe" else "ffprobe")
        ffprobe = shutil.which(args.ffprobe) if args.ffprobe else (str(sibling) if sibling.is_file() else shutil.which("ffprobe"))
        if not ffprobe:
            ap.error("ffprobe not found. Pass --ffprobe or explicitly set --fps 30")
        probe = subprocess.run([ffprobe, "-v", "error", "-select_streams", "v:0",
                                "-show_entries", "stream=avg_frame_rate,r_frame_rate", "-of", "json", str(args.base)],
                               capture_output=True, text=True, encoding="utf-8", errors="replace")
        if probe.returncode:
            ap.error("Unable to read base-video frame rate: " + probe.stderr)
        streams = json.loads(probe.stdout).get("streams", [])
        if not streams:
            ap.error("The base input has no video stream")
        fps = streams[0].get("avg_frame_rate", "0/0")
        if fps in ("0/0", "0", "N/A"):
            fps = streams[0].get("r_frame_rate", "0/0")
    try:
        rate = Fraction(fps)
        if not 0 < rate <= 240:
            raise ValueError("invalid frame rate")
    except (ValueError, ZeroDivisionError):
        ap.error("Invalid frame rate; set --fps to a positive rate <= 240, e.g. 30")
    fps = str(rate)
    print(f"Output CFR: {fps}", flush=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    cmd = [ffmpeg, "-hide_banner", "-nostdin", "-n", "-filter_complex_threads", "1",
           "-i", str(args.base), "-stream_loop", "-1"]
    if args.overlay.suffix.lower() == ".webm":
        cmd += ["-c:v", "libvpx-vp9"]  # INPUT decoder, before this input's -i.
    cmd += ["-i", str(args.overlay)]
    asset_filter = "format=rgba"
    if args.width:
        asset_filter += f",scale={args.width}:-2"
    # Shift the asset timestamps so its frame zero occurs AT --start, not earlier.
    asset_filter += f",setpts=PTS-STARTPTS+{args.start:.9f}/TB"
    enable = f"gte(t,{args.start:.9f})"
    if args.end is not None:
        enable += f"*lt(t,{args.end:.9f})"
    graph = (f"[0:v]setpts=PTS-STARTPTS[b];[1:v]{asset_filter}[o];"
             f"[b][o]overlay=x={args.x}:y={args.y}:alpha=straight:format=auto:"
             f"shortest=1:enable='{enable}',"
             "pad=ceil(iw/2)*2:ceil(ih/2)*2,format=yuv420p[v]")
    # Do not use OUTPUT -shortest: short audio should NOT truncate the base video.
    cmd += ["-filter_complex", graph, "-map", "[v]", "-map", "0:a?",
            "-af", "asetpts=PTS-STARTPTS", "-r", fps, "-fps_mode", "cfr", "-c:v", "libx264", "-preset", "medium", "-crf", "18",
            "-threads", "2", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", str(args.output)]
    result = subprocess.run(cmd)
    if result.returncode:
        raise RuntimeError("FFmpeg failed. Review its diagnostic output above.")
    print("DONE: " + str(args.output.resolve()))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        raise SystemExit(1)
