#!/usr/bin/env python3
"""Create reusable straight-alpha animations with Pillow + FFmpeg.

Examples:
  python make_overlay.py --out output
  python make_overlay.py --images a.png b.png c.png --out my_carousel
  python make_overlay.py --images logo.png --mode float --seconds 6 --out my_logo

No background removal is performed: use already-transparent source images.
"""
from __future__ import annotations

import argparse
import json
import math
import shutil
import subprocess
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter, ImageOps
except ImportError:
    raise SystemExit("Pillow is required. Run: python -m pip install -r requirements.txt")

VERSION = "1.0.0"
ENCODERS = {"prores": "prores_ks", "qtrle": "qtrle", "webm": "libvpx-vp9"}


def run(command: list[str]) -> None:
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if result.returncode:
        raise RuntimeError("FFmpeg failed:\n" + result.stderr[-6000:])


def parse_size(value: str) -> tuple[int, int]:
    try:
        w, h = (int(v) for v in value.lower().split("x"))
    except (ValueError, TypeError):
        raise argparse.ArgumentTypeError("Use WIDTHxHEIGHT, e.g. 1920x1080")
    if min(w, h) < 64 or w % 2 or h % 2:
        raise argparse.ArgumentTypeError("Width and height must be even integers >= 64")
    return w, h


def transparent(size: tuple[int, int]) -> Image.Image:
    return Image.new("RGBA", size, (0, 0, 0, 0))


def demo_images(folder: Path) -> list[Path]:
    """Original geometric test assets; no external fonts or pictures."""
    folder.mkdir(parents=True, exist_ok=True)
    paths: list[Path] = []
    size, cx, cy = 480, 240, 240
    colors = [(238, 187, 92), (88, 203, 190), (238, 137, 130)]
    for i, color in enumerate(colors):
        ink = transparent((size, size))
        d = ImageDraw.Draw(ink)
        d.ellipse((58, 58, 422, 422), outline=(*color, 255), width=8)
        d.ellipse((78, 78, 402, 402), outline=(*color, 115), width=3)
        if i == 0:
            d.ellipse((146, 146, 334, 334), outline=(*color, 255), width=16)
            d.ellipse((209, 209, 271, 271), fill=(*color, 255))
        elif i == 1:
            d.polygon([(240, 119), (353, 240), (240, 361), (127, 240)], fill=(*color, 230))
            d.polygon([(240, 169), (305, 240), (240, 311), (175, 240)], fill=(0, 0, 0, 0))
        else:
            pts = []
            for j in range(16):
                radius = 121 if j % 2 == 0 else 50
                angle = -math.pi / 2 + j * math.pi / 8
                pts.append((cx + radius * math.cos(angle), cy + radius * math.sin(angle)))
            d.polygon(pts, fill=(*color, 245))
        # Blur alpha only, retain straight RGB. This gives a soft, translucent glow.
        glow = Image.new("RGBA", ink.size, (*color, 0))
        glow.putalpha(ink.getchannel("A").filter(ImageFilter.GaussianBlur(10)).point(lambda a: round(a * 0.32)))
        glow.alpha_composite(ink)
        path = folder / f"{i + 1:02d}_demo.png"
        glow.save(path)
        paths.append(path)
    return paths


def load_asset(path: Path, size: tuple[int, int]) -> Image.Image:
    if not path.is_file():
        raise ValueError(f"Image not found: {path}")
    with Image.open(path) as src:
        im = ImageOps.exif_transpose(src).convert("RGBA")
    lo, hi = im.getchannel("A").getextrema()
    if hi == 0:
        raise ValueError(f"Image is completely transparent: {path}")
    if lo == 255:
        print(f"WARNING: {path.name} is fully opaque; this tool will NOT remove its background.", file=sys.stderr)
    # Fixed fit, no distortion. Keep source padding for intentional composition.
    max_w, max_h = int(size[0] * 0.64), int(size[1] * 0.64)
    factor = min(max_w / im.width, max_h / im.height)
    return im.resize((max(1, round(im.width * factor)), max(1, round(im.height * factor))), Image.Resampling.LANCZOS)


def place(im: Image.Image, size: tuple[int, int], dx: float = 0, dy: float = 0) -> Image.Image:
    canvas = transparent(size)
    canvas.alpha_composite(im, (round((size[0] - im.width) / 2 + dx), round((size[1] - im.height) / 2 + dy)))
    return canvas


def crossfade(a: Image.Image, b: Image.Image, amount: float) -> Image.Image:
    """Blend in premultiplied space, then export straight RGBA.

    Avoids dark RGB fringes and the alpha dip caused by overlaying two
    independently faded images. 8-bit operations are adequate for this demo;
    this is not a linear-light, floating-point/VFX compositing engine.
    """
    if amount <= 0:
        return a
    if amount >= 1:
        return b
    return Image.blend(a.convert("RGBa"), b.convert("RGBa"), amount).convert("RGBA")


def render_frame(index: int, assets: list[Image.Image], size: tuple[int, int],
                 mode: str, slot_frames: int, transition_frames: int) -> Image.Image:
    total = slot_frames * (len(assets) if mode == "carousel" else 1)
    index %= total  # Periodic model; frame(total) == frame(0).
    if mode == "carousel":
        slot, local = divmod(index, slot_frames)
        start = slot_frames - transition_frames
        if local < start:
            return place(assets[slot], size)
        u = (local - start) / transition_frames
        u = u * u * (3 - 2 * u)  # smoothstep, zero endpoint velocity
        shift = size[0] * 0.075
        a = place(assets[slot], size, dx=-shift * u)
        b = place(assets[(slot + 1) % len(assets)], size, dx=shift * (1 - u))
        return crossfade(a, b, u)
    phase = 2 * math.pi * index / total
    if mode == "float":
        return place(assets[0], size, dx=size[0] * 0.045 * math.sin(phase),
                     dy=-size[1] * 0.025 * math.sin(2 * phase))
    # Rotate on an ample transparent canvas so no corners are clipped.
    return place(assets[0], size).rotate(-360 * index / total, resample=Image.Resampling.BICUBIC,
                                        expand=False, fillcolor=(0, 0, 0, 0))


def checkerboard(size: tuple[int, int]) -> Image.Image:
    im = Image.new("RGB", size, (58, 64, 76))
    d = ImageDraw.Draw(im)
    tile = max(12, size[1] // 12)
    for y in range(0, size[1], tile):
        for x in range(0, size[0], tile):
            if (x // tile + y // tile) % 2:
                d.rectangle((x, y, x + tile - 1, y + tile - 1), fill=(91, 99, 112))
    return im


def encode(ffmpeg: str, frames: Path, out: Path, fmt: str, fps: int, total: int) -> Path:
    suffix = ".webm" if fmt == "webm" else ".mov"
    path = out / f"overlay_{fmt}{suffix}"
    command = [ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-n",
               "-filter_threads", "1", "-framerate", str(fps), "-start_number", "0",
               "-i", str(frames / "frame_%05d.png"), "-frames:v", str(total), "-an"]
    if fmt == "prores":
        command += ["-c:v", "prores_ks", "-profile:v", "4", "-pix_fmt", "yuva444p10le",
                    "-alpha_bits", "16", "-qscale:v", "4", "-threads", "2"]
    elif fmt == "qtrle":
        command += ["-c:v", "qtrle", "-pix_fmt", "argb"]
    else:
        command += ["-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0",
                    "-crf", "25", "-auto-alt-ref", "0", "-deadline", "good", "-cpu-used", "4",
                    "-threads", "2"]
    command += [str(path)]
    run(command)
    return path


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--version", action="version", version=VERSION)
    ap.add_argument("--images", nargs="+", type=Path, help="Images in playback order; omit to use demo assets")
    ap.add_argument("--mode", choices=["carousel", "float", "spin"], default="carousel")
    ap.add_argument("--size", type=parse_size, default=(960, 540))
    ap.add_argument("--fps", type=int, default=30)
    ap.add_argument("--seconds", type=float, default=3, help="Seconds per carousel slot (INCLUDING transition), or per single-image loop")
    ap.add_argument("--transition", type=float, default=1, help="Transition seconds within each carousel slot")
    ap.add_argument("--formats", nargs="+", choices=list(ENCODERS), default=["prores"])
    ap.add_argument("--keep-frames", action="store_true", help="Keep the complete RGBA PNG sequence")
    ap.add_argument("--preview", action="store_true", help="Create an opaque checkerboard MP4 preview, showing TWO cycles")
    ap.add_argument("--out", type=Path, default=Path("output"), help="New or empty folder; never overwrites existing material")
    ap.add_argument("--ffmpeg", default="ffmpeg", help="FFmpeg executable path or command")
    args = ap.parse_args()
    if args.fps < 1 or args.fps > 120:
        ap.error("fps must be an integer in 1..120")
    if not math.isfinite(args.seconds) or args.seconds <= 0:
        ap.error("seconds must be finite and positive")
    if not math.isfinite(args.transition) or args.transition < 0:
        ap.error("transition must be finite and nonnegative")
    slot_frames = round(args.seconds * args.fps)
    if slot_frames < 2 or abs(slot_frames - args.seconds * args.fps) > 1e-6:
        ap.error("seconds * fps must be a whole number of at least 2 frames")
    transition_frames = round(args.transition * args.fps)
    if args.mode == "carousel" and not (1 <= transition_frames < slot_frames):
        ap.error("For carousel, transition must be at least 1 frame and shorter than seconds")
    if args.out.exists() and (not args.out.is_dir() or any(args.out.iterdir())):
        ap.error("Output folder must be NEW or EMPTY; choose another --out to protect existing files")
    ffmpeg = shutil.which(args.ffmpeg)
    if not ffmpeg:
        ap.error("FFmpeg not found. Add it to PATH or pass --ffmpeg C:/path/ffmpeg.exe")
    enc_info = subprocess.run([ffmpeg, "-hide_banner", "-encoders"], capture_output=True,
                              text=True, encoding="utf-8", errors="replace")
    if enc_info.returncode:
        raise RuntimeError("Unable to inspect FFmpeg encoders: " + enc_info.stderr)
    enc_names = {line.split()[1] for line in enc_info.stdout.splitlines() if len(line.split()) >= 2}
    needed = {ENCODERS[f] for f in args.formats} | ({"libx264"} if args.preview else set())
    if needed - enc_names:
        ap.error("This FFmpeg build lacks: " + ", ".join(sorted(needed - enc_names)))
    args.out.mkdir(parents=True, exist_ok=True)
    paths = args.images or demo_images(args.out / "source_images")
    if args.mode != "carousel" and len(paths) != 1:
        if args.images:
            ap.error("float and spin need exactly ONE --images file")
        paths = paths[:1]
    assets = [load_asset(path, args.size) for path in paths]
    if args.mode == "spin":
        # A rectangular source must fit at EVERY angle, including 45/90 degrees.
        im = assets[0]
        factor = min(1.0, min(args.size) * 0.88 / math.hypot(im.width, im.height))
        if factor < 1:
            assets[0] = im.resize((max(1, round(im.width * factor)), max(1, round(im.height * factor))), Image.Resampling.LANCZOS)
    total = slot_frames * (len(assets) if args.mode == "carousel" else 1)
    duration = total / args.fps
    frames = args.out / "frames"
    frames.mkdir()
    print(f"Rendering {total} frames, {duration:.3f}s, {args.size[0]}x{args.size[1]}, straight RGBA")
    # Render 0 .. total-1. Do NOT append frame(total), which repeats the endpoint.
    for n in range(total):
        im = render_frame(n, assets, args.size, args.mode, slot_frames, transition_frames)
        im.save(frames / f"frame_{n:05d}.png", compress_level=3)
        if n % (args.fps * 2) == 0:
            print(f"  frame {n}/{total}", flush=True)
    if render_frame(0, assets, args.size, args.mode, slot_frames, transition_frames).tobytes() != render_frame(total, assets, args.size, args.mode, slot_frames, transition_frames).tobytes():
        raise RuntimeError("Periodic model check failed")
    outputs = [encode(ffmpeg, frames, args.out, fmt, args.fps, total) for fmt in dict.fromkeys(args.formats)]
    mid_path = frames / f"frame_{min(total - 1, slot_frames - max(1, transition_frames // 2)):05d}.png"
    with Image.open(mid_path) as src:
        src.save(args.out / "transparent_sample.png")
    if args.preview:
        checker = args.out / "preview_background_CHECKERBOARD.png"
        checkerboard(args.size).save(checker)
        overlay = outputs[0]
        command = [ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-n",
                   "-filter_complex_threads", "1", "-loop", "1", "-framerate", str(args.fps),
                   "-i", str(checker), "-stream_loop", "-1"]
        if overlay.suffix == ".webm":
            command += ["-c:v", "libvpx-vp9"]
        command += ["-i", str(overlay), "-filter_complex",
                    "[0:v]setpts=PTS-STARTPTS[b];[1:v]setpts=PTS-STARTPTS[o];"
                    "[b][o]overlay=alpha=straight:format=auto,format=yuv420p[v]",
                    "-map", "[v]", "-t", str(2 * duration), "-an", "-r", str(args.fps), "-fps_mode", "cfr", "-c:v", "libx264",
                    "-preset", "fast", "-crf", "20", "-threads", "2", "-movflags", "+faststart",
                    str(args.out / "preview_checkerboard_NOT_TRANSPARENT.mp4")]
        run(command)
    manifest = {
        "kit_version": VERSION, "width": args.size[0], "height": args.size[1], "fps": args.fps,
        "frame_count": total, "duration_seconds": duration, "mode": args.mode,
        "seconds_per_slot_including_transition": slot_frames / args.fps,
        "transition_seconds": transition_frames / args.fps if args.mode == "carousel" else 0,
        "loopable": True, "alpha_mode": "straight", "source_precision": "8-bit RGBA",
        "outputs": [p.name for p in outputs], "png_frames_retained": args.keep_frames,
        "source_images": [p.name for p in paths],
        "note": "MP4/checkerboard files are PREVIEWS ONLY, not transparent assets. No endpoint frame is duplicated."
    }
    (args.out / "manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")
    if not args.keep_frames:
        shutil.rmtree(frames)  # Only remove the folder newly created by THIS run.
    print("DONE: " + str(args.out.resolve()))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        raise SystemExit(1)
