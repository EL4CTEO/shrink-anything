"""Renders the shrink-anything logo, wordmark and social card (Pillow, 4x supersampled).

The mark is a block of code being squeezed: the faint red lines are what the code was, the
bright core is what's left, and the two chevrons pushing in are code brackets turned around.

    python scripts/logo.py
"""

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

OUT = Path(__file__).resolve().parent.parent / "docs" / "assets"
SS = 4  # supersampling

MINT_A, MINT_B = (47, 191, 135), (94, 234, 212)
INK = (240, 246, 252)

# (width before, width after) of each line of code, top to bottom, on a 1024 grid
ROWS = [(520, 150), (640, 210), (430, 120), (700, 230), (560, 170), (660, 200), (380, 110)]
BAR_H, GAP = 46, 26
CORE_LEFT = 512 - 115  # the core is left-aligned, like code


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))


def gradient(size, c0, c1, horizontal=False):
    w, h = size
    strip = Image.new("RGBA", (w, 1) if horizontal else (1, h))
    n = w if horizontal else h
    for i in range(n):
        c = lerp(c0, c1, i / max(1, n - 1))
        strip.putpixel((i, 0) if horizontal else (0, i), c)
    return strip.resize(size)


def grow(mask, px):
    """The mask, wider by about px on every side (a blur and a threshold: much faster than MaxFilter)."""
    return mask.filter(ImageFilter.GaussianBlur(px)).point(lambda v: 255 if v > 24 else 0).filter(ImageFilter.GaussianBlur(px * 0.25))


def chevron(draw, cx, cy, half_h, depth, width, direction):
    """A thick > (direction 1) or < (direction -1) with round ends, tip at cx."""
    pts = [(cx - direction * depth, cy - half_h), (cx, cy), (cx - direction * depth, cy + half_h)]
    draw.line(pts, fill=255, width=int(width), joint="curve")
    r = width / 2
    for x, y in pts:
        draw.ellipse([x - r, y - r, x + r, y + r], fill=255)


def mark(size=1024, background=True):
    S = size * SS
    k = S / 1024
    canvas = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    if background:
        bg = gradient((S, S), (22, 28, 36, 255), (8, 10, 14, 255))
        glow = Image.new("L", (S, S), 0)
        ImageDraw.Draw(glow).ellipse([S * 0.22, S * 0.22, S * 0.78, S * 0.78], fill=140)
        glow = glow.filter(ImageFilter.GaussianBlur(S * 0.11))
        tint = Image.new("RGBA", (S, S), (40, 200, 150, 255))
        bg = Image.composite(Image.blend(bg, tint, 0.20), bg, glow)
        mask = Image.new("L", (S, S), 0)
        ImageDraw.Draw(mask).rounded_rectangle([0, 0, S - 1, S - 1], radius=int(S * 0.22), fill=255)
        canvas.paste(bg, (0, 0), mask)
        rim = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        ImageDraw.Draw(rim).rounded_rectangle([2 * k, 2 * k, S - 2 * k, S - 2 * k], radius=int(S * 0.22), outline=(255, 255, 255, 28), width=int(3 * k))
        canvas = Image.alpha_composite(canvas, rim)

    total = len(ROWS) * BAR_H + (len(ROWS) - 1) * GAP
    top = (1024 - total) / 2

    # what the code was: faint lines, redder the further they stick out
    ghost = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    dg = ImageDraw.Draw(ghost)
    for r, (before, _) in enumerate(ROWS):
        y0 = top + r * (BAR_H + GAP)
        x0 = 512 - before / 2
        dg.rounded_rectangle([x0 * k, y0 * k, (x0 + before) * k, (y0 + BAR_H) * k], radius=int(BAR_H * k / 2), fill=(248, 81, 73, 74))
    canvas = Image.alpha_composite(canvas, ghost)

    # the chevrons, with a dark edge so they sit on top of the ghost lines
    cy, half_h, depth, width = 512, 190, 150, 78
    chev = Image.new("L", (S, S), 0)
    dc = ImageDraw.Draw(chev)
    chevron(dc, 330 * k, cy * k, half_h * k, depth * k, width * k, 1)
    chevron(dc, 694 * k, cy * k, half_h * k, depth * k, width * k, -1)
    edge = grow(chev, 7 * k)
    dark = Image.new("RGBA", (S, S), (10, 13, 18, 255))
    dark.putalpha(edge)
    canvas = Image.alpha_composite(canvas, dark)
    halo = Image.new("RGBA", (S, S), (60, 230, 180, 0))
    halo.putalpha(chev.filter(ImageFilter.GaussianBlur(30 * k)).point(lambda v: int(v * 0.55)))
    canvas = Image.alpha_composite(canvas, halo)
    fill = gradient((S, S), (*MINT_B, 255), (*MINT_A, 255))
    layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    layer.paste(fill, (0, 0), chev)
    canvas = Image.alpha_composite(canvas, layer)
    shine = Image.new("L", (S, S), 0)
    ds = ImageDraw.Draw(shine)
    chevron(ds, (330 - 6) * k, (cy - 10) * k, (half_h - 4) * k, (depth - 2) * k, 14 * k, 1)
    chevron(ds, (694 + 6) * k, (cy - 10) * k, (half_h - 4) * k, (depth - 2) * k, 14 * k, -1)
    shine = ImageChops.multiply(shine.filter(ImageFilter.GaussianBlur(3 * k)), chev).point(lambda v: int(v * 0.30))
    white = Image.new("RGBA", (S, S), (255, 255, 255, 0))
    white.putalpha(shine)
    canvas = Image.alpha_composite(canvas, white)

    # what's left: the bright core between them
    core = Image.new("L", (S, S), 0)
    dk = ImageDraw.Draw(core)
    for r, (_, after) in enumerate(ROWS):
        y0 = top + r * (BAR_H + GAP)
        dk.rounded_rectangle([CORE_LEFT * k, y0 * k, (CORE_LEFT + after) * k, (y0 + BAR_H) * k], radius=int(BAR_H * k / 2), fill=255)
    backing = grow(core, 9 * k)
    dark = Image.new("RGBA", (S, S), (10, 13, 18, 255))
    dark.putalpha(backing.point(lambda v: int(v * 0.92)))
    canvas = Image.alpha_composite(canvas, dark)
    layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    layer.paste(gradient((S, S), (255, 255, 255, 255), (214, 244, 236, 255)), (0, 0), core)
    canvas = Image.alpha_composite(canvas, layer)
    return canvas.resize((size, size), Image.LANCZOS)


def font(names, size):
    for n in names:
        p = Path("C:/Windows/Fonts") / n
        if p.exists():
            return ImageFont.truetype(str(p), size)
    return ImageFont.load_default(size)


def wordmark(width, height, logo_size, title_size, tagline=None, bg=True):
    img = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    if bg:
        img = gradient((width, height), (17, 21, 28, 255), (8, 10, 14, 255))
    logo = mark(logo_size)
    title = font(["segoeuib.ttf", "seguisb.ttf", "arialbd.ttf"], title_size)
    small = font(["segoeui.ttf", "arial.ttf"], int(title_size * 0.36))
    d = ImageDraw.Draw(img)
    w_a = d.textlength("shrink", font=title)
    w_b = d.textlength("-anything", font=title)
    total = logo_size + title_size * 0.45 + w_a + w_b
    x = (width - total) / 2
    y = (height - logo_size) / 2 - (title_size * 0.32 if tagline else 0)
    img.alpha_composite(logo, (int(x), int(y)))
    tx = x + logo_size + title_size * 0.45
    ty = y + logo_size / 2
    d.text((tx, ty), "shrink", font=title, fill=INK if bg else (139, 148, 158), anchor="lm")
    d.text((tx + w_a, ty), "-anything", font=title, fill=(63, 207, 160), anchor="lm")
    if tagline:
        d.text((width / 2, y + logo_size + title_size * 0.62), tagline, font=small, fill=(139, 148, 158), anchor="mm")
    return img


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    mark(1024).save(OUT / "logo.png")
    mark(256).save(OUT / "logo-256.png")
    mark(256).save(OUT.parent.parent / ".claude-plugin" / "icon.png")
    mark(1024, background=False).resize((512, 512), Image.LANCZOS).save(OUT / "mark.png")
    wordmark(1600, 400, 260, 150, bg=False).save(OUT / "wordmark.png")
    wordmark(1280, 640, 250, 124, tagline="Less code. Same tests passing.").convert("RGB").save(OUT / "social.png")
    print("ok")
