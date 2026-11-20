"""Composes the Microsoft Store images from the layers the application drew.

    set BV_STORE=<layers folder>
    cargo test -p bv-app store_images -- --ignored
    python tools/store-images.py <layers folder> <output folder>

The application draws every surface with its own code (crates/bv-app/src/app/store.rs);
this script only adds what Windows adds on a real screen, the same way as
tools/readme-images.py: the desktop background, the translucent material behind the
panel and the taskbar, rounded corners, shadows and outlines, and the pointer. It
writes, in the output folder:

- screenshots/<language>-<n>.png: six 1920 x 1080 screenshots per language;
- video/BetterVolume.mp4: the English demonstration video (1920 x 1080, H.264, with a
  silent stereo AAC track, as the Store asks), and video/thumbnail.png, one of its frames;
- hero.png: the 16:9 super hero image, packaging/store/hero.svg rendered by Microsoft Edge.

Needs Pillow, ffmpeg (on the PATH) and Microsoft Edge.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageChops, ImageDraw, ImageFilter

SRC, DST = sys.argv[1], sys.argv[2]
with open(os.path.join(SRC, "scenes.json"), encoding="utf-8") as _f:
    META = json.load(_f)
S = META["scale"]
W, H = round(META["desktop"][0] * S), round(META["desktop"][1] * S)
TASKBAR = round(META["taskbar"] * S)
FPS = 25

# What Windows draws around a surface (Windows 11 flyouts, windows and taskbar).
RADIUS = round(8 * S)
STYLE = {
    "dark": {
        "stops": [(0.0, (16, 28, 52)), (0.55, (34, 58, 96)), (1.0, (68, 52, 92))],
        "glow": (92, 120, 190),
        "taskbar": (28, 28, 28, 218),
        "taskbar_line": (255, 255, 255, 20),
        "outline": (117, 117, 117, 102),
        "shadow": 0.42,
    },
    "light": {
        "stops": [(0.0, (196, 214, 240)), (0.55, (222, 232, 246)), (1.0, (236, 222, 240))],
        "glow": (255, 255, 255),
        "taskbar": (238, 238, 238, 210),
        "taskbar_line": (0, 0, 0, 16),
        "outline": (117, 117, 117, 66),
        "shadow": 0.20,
    },
}

_layers = {}


def layer(name):
    """A layer drawn by the application (premultiplied BGRA), as straight RGBA."""
    if name not in _layers:
        w, h = META["layers"][name]
        with open(os.path.join(SRC, name + ".raw"), "rb") as f:
            data = f.read()
        _layers[name] = Image.frombuffer("RGBA", (w, h), data, "raw", "BGRa", 0, 1).copy()
    return _layers[name]


def mix_colour(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


_backgrounds = {}


def background(mode):
    """A soft abstract desktop background: a diagonal gradient and a glow."""
    if mode in _backgrounds:
        return _backgrounds[mode]
    stops = STYLE[mode]["stops"]
    small = Image.new("RGB", (64, 64))
    pixels = small.load()
    for y in range(64):
        for x in range(64):
            t = x / 63 * 0.45 + y / 63 * 0.55
            for (t0, c0), (t1, c1) in zip(stops, stops[1:]):
                if t <= t1:
                    pixels[x, y] = mix_colour(c0, c1, (t - t0) / (t1 - t0))
                    break
    image = small.resize((W, H), Image.BICUBIC).convert("RGBA")
    glow = Image.new("L", (W, H), 0)
    ImageDraw.Draw(glow).ellipse((W * 0.05, H * 0.05, W * 0.75, H * 0.6), fill=120)
    glow = glow.filter(ImageFilter.GaussianBlur(W * 0.18))
    image = Image.composite(Image.new("RGBA", (W, H), STYLE[mode]["glow"] + (255,)), image, glow)
    _backgrounds[mode] = image
    return image


_blurred = {}


def blurred(mode):
    """The background as the translucent material sees it."""
    if mode not in _blurred:
        _blurred[mode] = background(mode).filter(ImageFilter.GaussianBlur(30 * S))
    return _blurred[mode]


def rounded_mask(size, radius=RADIUS):
    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size[0] - 1, size[1] - 1), radius, fill=255)
    return mask


_shadows = {}


def shadow_of(size, mode):
    """The soft shadow under a surface of `size` (with a 60-pixel margin all round)."""
    key = (size, mode)
    if key not in _shadows:
        w, h = size
        shadow = Image.new("L", (w + 120, h + 120), 0)
        ImageDraw.Draw(shadow).rounded_rectangle((60, 60 + 8 * S, 60 + w, 60 + h + 8 * S), RADIUS, fill=255)
        shadow = shadow.filter(ImageFilter.GaussianBlur(16 * S))
        _shadows[key] = shadow.point(lambda v: round(v * STYLE[mode]["shadow"]))
    return _shadows[key]


def surface(scene, mode, image, x, y, translucent=True, limit=None, opacity=1.0):
    """Puts a surface on the scene at (x, y) pixels: shadow, material, content, rounded
    corners, outline. `limit`: nothing is drawn below this line (behind the taskbar)."""
    w, h = image.size
    if w <= 0 or h <= 0 or opacity <= 0:
        return
    piece = Image.new("RGBA", scene.size, (0, 0, 0, 0))
    black = Image.new("RGBA", (w + 120, h + 120), (0, 0, 0, 255))
    piece.paste(black, (x - 60, y - 60), shadow_of((w, h), mode))
    if translucent:
        material = blurred(mode).crop((x, y, x + w, y + h))
        body = Image.alpha_composite(material, image)
    else:
        body = image
    piece.paste(body, (x, y), rounded_mask((w, h)))
    outline = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    ImageDraw.Draw(outline).rounded_rectangle((0, 0, w - 1, h - 1), RADIUS, outline=STYLE[mode]["outline"], width=1)
    piece.alpha_composite(outline, (x, y))
    if limit is not None:
        cut = Image.new("L", scene.size, 255)
        ImageDraw.Draw(cut).rectangle((0, limit, scene.size[0], scene.size[1]), fill=0)
        piece.putalpha(ImageChops.multiply(piece.getchannel("A"), cut))
    if opacity < 1.0:
        piece.putalpha(piece.getchannel("A").point(lambda v: round(v * opacity)))
    scene.alpha_composite(piece)


_windows = {}


def window_image(content_name, caption_name):
    """A window: the title bar Windows draws, in the window's own colour, above the
    client area."""
    key = (content_name, caption_name)
    if key not in _windows:
        content = layer(content_name)
        caption = layer(caption_name)
        fill = content.getpixel((content.size[0] - 2, 2))
        image = Image.new("RGBA", (content.size[0], content.size[1] + caption.size[1]), fill)
        image.alpha_composite(caption, (0, 0))
        image.alpha_composite(content, (0, caption.size[1]))
        _windows[key] = image
    return _windows[key]


def px(v):
    return round(v * S)


def taskbar(scene, mode, name):
    top = H - TASKBAR
    strip = blurred(mode).crop((0, top, W, H))
    strip = Image.alpha_composite(strip, Image.new("RGBA", strip.size, STYLE[mode]["taskbar"]))
    ImageDraw.Draw(strip).line((0, 0, W, 0), fill=STYLE[mode]["taskbar_line"], width=1)
    strip.alpha_composite(layer(name))
    scene.alpha_composite(strip, (0, top))


def compose(mode, description):
    """The desktop with the surfaces of `description`, bottom to top, and the taskbar."""
    scene = background(mode).copy()
    for item in description["items"]:
        x, y = px(item["at"][0]), px(item["at"][1])
        kind = item["kind"]
        if kind == "popup":
            surface(scene, mode, layer(item["layer"]), x, y, limit=H - TASKBAR)
        elif kind == "solid":
            surface(scene, mode, layer(item["layer"]), x, y, translucent=False)
        elif kind == "window":
            image = window_image(item["layer"], item["caption"])
            surface(scene, mode, image, x, y, translucent=False, opacity=item["opacity"])
        else:
            raise ValueError(kind)
    taskbar(scene, mode, description["taskbar"])
    return scene


# The Windows arrow pointer, in DIPs.
ARROW = [(0, 0), (0, 16.5), (4, 12.6), (6.8, 18.8), (9.2, 17.8), (6.5, 11.8), (11.8, 11.8)]


def pointer(scene, at):
    x, y = at[0] * S, at[1] * S
    points = [(x + ax * S, y + ay * S) for ax, ay in ARROW]
    shadow = Image.new("RGBA", scene.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).polygon([(ax + 1.5 * S, ay + 1.5 * S) for ax, ay in points], fill=(0, 0, 0, 70))
    scene.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(1.2 * S)))
    ImageDraw.Draw(scene).polygon(points, fill=(255, 255, 255, 255), outline=(0, 0, 0, 255))


def screenshots():
    folder = os.path.join(DST, "screenshots")
    os.makedirs(folder, exist_ok=True)
    for shot in META["shots"]:
        image = compose(shot["mode"], shot).convert("RGB")
        path = os.path.join(folder, shot["name"] + ".png")
        image.save(path, optimize=True)
    print(len(META["shots"]), "screenshots", (W, H))


def frame_image(frame):
    mix = frame["mix"]
    if mix <= 0:
        scene = compose("dark", frame["dark"])
    elif mix >= 1:
        scene = compose("light", frame["light"])
    else:
        scene = Image.blend(compose("dark", frame["dark"]), compose("light", frame["light"]), mix)
    pointer(scene, frame["cursor"])
    return scene.convert("RGB")


def video():
    """Every recorded step is held for its duration, at 25 frames per second."""
    folder = os.path.join(DST, "video")
    os.makedirs(folder, exist_ok=True)
    work = tempfile.mkdtemp(prefix="bv-store-video-")
    try:
        index, total, previous, thumbnail = 0, 0, None, None
        for frame in META["frames"]:
            key = json.dumps([frame["mix"], frame["dark"], frame["light"], frame["cursor"]], sort_keys=True)
            if key != previous:
                image = frame_image(frame)
                previous = key
            # Frames up to the end of this step of the recording (no drift from rounding).
            count = round((total + frame["ms"]) * FPS / 1000) - index
            for _ in range(count):
                image.save(os.path.join(work, f"{index:05d}.png"), compress_level=1)
                index += 1
            total += frame["ms"]
            # The thumbnail: the panel open in the dark theme, before the first change.
            if thumbnail is None and frame["dark"] and frame["mix"] <= 0 and total >= 2700:
                thumbnail = image
        target = os.path.join(folder, "BetterVolume.mp4")
        subprocess.run(
            [
                "ffmpeg", "-y", "-loglevel", "error",
                "-framerate", str(FPS), "-i", os.path.join(work, "%05d.png"),
                "-f", "lavfi", "-i", "anullsrc=channel_layout=stereo:sample_rate=48000",
                "-shortest",
                "-c:v", "libx264", "-profile:v", "high", "-pix_fmt", "yuv420p",
                "-bf", "2", "-g", str(FPS // 2), "-b:v", "12M", "-maxrate", "50M", "-bufsize", "50M",
                "-c:a", "aac", "-b:a", "384k",
                "-movflags", "+faststart",
                target,
            ],
            check=True,
        )
        (thumbnail or image).save(os.path.join(folder, "thumbnail.png"), optimize=True)
        print("video", index, "frames", total, "ms", os.path.getsize(target), "bytes")
    finally:
        shutil.rmtree(work, ignore_errors=True)


def hero():
    """The 16:9 super hero image: packaging/store/hero.svg rendered by Microsoft Edge
    (present on every Windows 11) in headless mode, at 1920 x 1080."""
    source = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "packaging", "store", "hero.svg")
    edge = next(
        (p for p in (
            os.path.join(os.environ.get("ProgramFiles(x86)", ""), "Microsoft", "Edge", "Application", "msedge.exe"),
            os.path.join(os.environ.get("ProgramFiles", ""), "Microsoft", "Edge", "Application", "msedge.exe"),
        ) if os.path.exists(p)),
        None,
    )
    if edge is None:
        sys.exit("Microsoft Edge not found")
    work = tempfile.mkdtemp(prefix="bv-store-hero-")
    try:
        shot = os.path.join(work, "hero.png")
        subprocess.run(
            [
                edge, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
                f"--user-data-dir={os.path.join(work, 'profile')}", "--window-size=1920,1080",
                f"--screenshot={shot}", "file:///" + source.replace("\\", "/"),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        image = Image.open(shot).convert("RGB")
        if image.size != (1920, 1080):
            sys.exit(f"hero image is {image.size}, not 1920 x 1080")
        image.save(os.path.join(DST, "hero.png"), optimize=True)
        print("hero", image.size)
    finally:
        shutil.rmtree(work, ignore_errors=True)


os.makedirs(DST, exist_ok=True)
screenshots()
video()
hero()
