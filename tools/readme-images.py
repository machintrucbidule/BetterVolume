"""Composes the documentation images from the layers the application drew.

    set BV_README=<layers folder>
    cargo test -p bv-app readme_images -- --ignored
    python tools/readme-images.py <layers folder> docs/images

The application draws every surface with its own code (crates/bv-app/src/app/readme.rs);
this script only adds what Windows adds on a real screen: the desktop background, the
translucent material behind the panel and the taskbar, rounded corners, shadows and
outlines, and the pointer. It writes:

- panel.png: the panel in the dark and the light theme, side by side;
- settings.png: the settings window;
- tray.png: the tray icon in both looks, the speaker then each output, dark and light;
- bubble.png: the bubble above the taskbar, dark and light;
- demo.webp: the animated demo.

Needs Pillow (with WebP support).
"""

import json
import os
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

SRC, DST = sys.argv[1], sys.argv[2]
with open(os.path.join(SRC, "scenes.json"), encoding="utf-8") as _f:
    META = json.load(_f)
S = META["scale"]
W, H = round(META["scene"][0] * S), round(META["scene"][1] * S)
TASKBAR = round(META["taskbar"] * S)

# What Windows draws around a surface (Windows 11 flyouts and taskbar).
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


def background(mode, size=(W, H)):
    """A soft abstract desktop background: a diagonal gradient and a glow."""
    key = (mode, size)
    if key in _backgrounds:
        return _backgrounds[key]
    w, h = size
    stops = STYLE[mode]["stops"]
    small = Image.new("RGB", (64, 64))
    px = small.load()
    for y in range(64):
        for x in range(64):
            t = (x / 63 * 0.45 + y / 63 * 0.55)
            for (t0, c0), (t1, c1) in zip(stops, stops[1:]):
                if t <= t1:
                    px[x, y] = mix_colour(c0, c1, (t - t0) / (t1 - t0))
                    break
    image = small.resize((w, h), Image.BICUBIC).convert("RGBA")
    glow = Image.new("L", (w, h), 0)
    ImageDraw.Draw(glow).ellipse((w * 0.05, h * 0.05, w * 0.75, h * 0.6), fill=120)
    glow = glow.filter(ImageFilter.GaussianBlur(w * 0.18))
    image = Image.composite(Image.new("RGBA", (w, h), STYLE[mode]["glow"] + (255,)), image, glow)
    _backgrounds[key] = image
    return image


_blurred = {}


def blurred(mode, size=(W, H)):
    """The background as the translucent material sees it."""
    key = (mode, size)
    if key not in _blurred:
        _blurred[key] = background(mode, size).filter(ImageFilter.GaussianBlur(30 * S))
    return _blurred[key]


def rounded_mask(size, radius=RADIUS):
    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size[0] - 1, size[1] - 1), radius, fill=255)
    return mask


def surface(scene, mode, image, x, y, translucent=True, limit=None):
    """Puts a popup surface on the scene at (x, y) pixels: shadow, material, content,
    rounded corners, outline. `limit`: nothing is drawn below this line (behind the
    taskbar)."""
    w, h = image.size
    if w <= 0 or h <= 0:
        return
    shadow = Image.new("L", (w + 120, h + 120), 0)
    ImageDraw.Draw(shadow).rounded_rectangle((60, 60 + 8 * S, 60 + w, 60 + h + 8 * S), RADIUS, fill=255)
    shadow = shadow.filter(ImageFilter.GaussianBlur(16 * S))
    shadow = shadow.point(lambda v: round(v * STYLE[mode]["shadow"]))
    black = Image.new("RGBA", shadow.size, (0, 0, 0, 255))
    piece = Image.new("RGBA", scene.size, (0, 0, 0, 0))
    piece.paste(black, (x - 60, y - 60), shadow)
    if translucent:
        material = blurred(mode, scene.size).crop((x, y, x + w, y + h))
        body = Image.alpha_composite(material, image)
    else:
        body = image
    mask = rounded_mask((w, h))
    piece.paste(body, (x, y), mask)
    outline = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    ImageDraw.Draw(outline).rounded_rectangle((0, 0, w - 1, h - 1), RADIUS, outline=STYLE[mode]["outline"], width=1)
    piece.alpha_composite(outline, (x, y))
    if limit is not None:
        cut = Image.new("L", scene.size, 255)
        ImageDraw.Draw(cut).rectangle((0, limit, scene.size[0], scene.size[1]), fill=0)
        piece.putalpha(ImageChops.multiply(piece.getchannel("A"), cut))
    scene.alpha_composite(piece)


def taskbar(scene, mode, name):
    top = scene.size[1] - TASKBAR
    strip = blurred(mode, scene.size).crop((0, top, scene.size[0], scene.size[1]))
    strip = Image.alpha_composite(strip, Image.new("RGBA", strip.size, STYLE[mode]["taskbar"]))
    ImageDraw.Draw(strip).line((0, 0, strip.size[0], 0), fill=STYLE[mode]["taskbar_line"], width=1)
    strip.alpha_composite(layer(name))
    scene.alpha_composite(strip, (0, top))


# The Windows arrow pointer, in DIPs.
ARROW = [(0, 0), (0, 16.5), (4, 12.6), (6.8, 18.8), (9.2, 17.8), (6.5, 11.8), (11.8, 11.8)]


def pointer(scene, at):
    x, y = at[0] * S, at[1] * S
    points = [(x + px * S, y + py * S) for px, py in ARROW]
    shadow = Image.new("RGBA", scene.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).polygon([(px + 1.5 * S, py + 1.5 * S) for px, py in points], fill=(0, 0, 0, 70))
    scene.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(1.2 * S)))
    draw = ImageDraw.Draw(scene)
    draw.polygon(points, fill=(255, 255, 255, 255), outline=(0, 0, 0, 255))


def px(v):
    return round(v * S)


def panel_scene(entry):
    mode = entry["mode"]
    scene = background(mode).copy()
    image = layer(entry["flyout"])
    surface(scene, mode, image, px(entry["at"][0]), px(entry["at"][1]))
    taskbar(scene, mode, entry["taskbar"])
    return scene


def save_png(image, name):
    image.convert("RGB").save(os.path.join(DST, name), optimize=True)
    print(name, image.size, os.path.getsize(os.path.join(DST, name)), "bytes")


def panels():
    dark, light = (panel_scene(e) for e in META["panels"])
    both = Image.new("RGBA", (W * 2, H))
    both.paste(dark, (0, 0))
    both.paste(light, (W, 0))
    save_png(both, "panel.png")


def bubbles():
    images = []
    for entry in META["bubbles"]:
        mode = entry["mode"]
        scene = background(mode).copy()
        image = layer(entry["bubble"])
        x, y = px(entry["at"][0]), px(entry["at"][1])
        surface(scene, mode, image, x, y, translucent=False)
        taskbar(scene, mode, entry["taskbar"])
        top = max(0, y - px(28))
        images.append(scene.crop((0, top, W, H)))
    w, h = images[0].size
    both = Image.new("RGBA", (w, h * 2))
    both.paste(images[0], (0, 0))
    both.paste(images[1], (0, h))
    save_png(both, "bubble.png")


def settings():
    entry = next(e for e in META["settings"] if e["mode"] == "light")
    mode = entry["mode"]
    content = layer(entry["window"])
    caption = layer(entry["caption"])
    # The title bar takes the window's own background colour.
    fill = content.getpixel((content.size[0] - 2, 2))
    image = Image.new("RGBA", (content.size[0], content.size[1] + caption.size[1]), fill)
    image.alpha_composite(caption, (0, 0))
    image.alpha_composite(content, (0, caption.size[1]))
    margin = px(40)
    size = (image.size[0] + 2 * margin, image.size[1] + 2 * margin)
    scene = background(mode, size).copy()
    surface(scene, mode, image, margin, margin, translucent=False)
    save_png(scene, "settings.png")


def trays():
    """The tray icon at the large size, one row per look (Windows, then BetterVolume):
    the speaker, then each output's device, on a strip of dark taskbar then of light
    taskbar (each icon's slot of the taskbar, side by side)."""
    gap, mode_gap = px(6), px(16)
    cells = {}
    for entry in META["trays"]:
        mode = entry["mode"]
        scene = background(mode).copy()
        taskbar(scene, mode, entry["taskbar"])
        centre = px(entry["icon"][0])
        cells[(entry["look"], entry["picture"], mode)] = scene.crop(
            (centre - px(20), H - TASKBAR, centre + px(20), H))
    looks = list(dict.fromkeys(e["look"] for e in META["trays"]))
    pictures = list(dict.fromkeys(e["picture"] for e in META["trays"]))
    w, h = next(iter(cells.values())).size
    strip = w * len(pictures)
    sheet = Image.new("RGBA", (strip * 2 + mode_gap, h * len(looks) + gap * (len(looks) - 1)), (0, 0, 0, 0))
    for row, look in enumerate(looks):
        for col, mode in enumerate(("dark", "light")):
            for i, picture in enumerate(pictures):
                sheet.paste(cells[(look, picture, mode)], (col * (strip + mode_gap) + i * w, row * (h + gap)))
    path = os.path.join(DST, "tray.png")
    sheet.save(path, optimize=True)
    print("tray.png", sheet.size, os.path.getsize(path), "bytes")


def frame_scene(frame, mode):
    index = 0 if mode == "dark" else 1
    scene = background(mode).copy()
    if frame["flyout"]:
        image = layer(frame["flyout"][index])
        surface(scene, mode, image, px(frame["at"][0]), px(frame["at"][1]), limit=H - TASKBAR)
    taskbar(scene, mode, frame["taskbar"][index])
    return scene


def demo():
    frames, durations = [], []
    for frame in META["frames"]:
        mix = frame["mix"]
        if mix <= 0:
            scene = frame_scene(frame, "dark")
        elif mix >= 1:
            scene = frame_scene(frame, "light")
        else:
            scene = Image.blend(frame_scene(frame, "dark"), frame_scene(frame, "light"), mix)
        pointer(scene, frame["cursor"])
        scene = scene.convert("RGB")
        if frames and scene.tobytes() == frames[-1].tobytes():
            durations[-1] += frame["ms"]
            continue
        frames.append(scene)
        durations.append(frame["ms"])
    path = os.path.join(DST, "demo.webp")
    frames[0].save(
        path,
        save_all=True,
        append_images=frames[1:],
        duration=durations,
        loop=0,
        quality=88,
        method=6,
    )
    print("demo.webp", frames[0].size, len(frames), "frames", sum(durations), "ms", os.path.getsize(path), "bytes")


os.makedirs(DST, exist_ok=True)
panels()
bubbles()
settings()
trays()
demo()
