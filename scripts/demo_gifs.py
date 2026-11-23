"""Renders the README demo GIFs from transcripts of real shrink-anything runs.

    python scripts/demo_gifs.py      (needs Pillow; Windows fonts by default, set SA_FONT/SA_SYMBOL_FONT elsewhere)
"""

import os
import re
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

OUT = Path(__file__).resolve().parent.parent / "docs" / "assets"
FONT = os.environ.get("SA_FONT", "C:/Windows/Fonts/consola.ttf")
BOLD = os.environ.get("SA_FONT_BOLD", "C:/Windows/Fonts/consolab.ttf")
SYMBOL = os.environ.get("SA_SYMBOL_FONT", "C:/Windows/Fonts/seguisym.ttf")
SIZE, LINE, PAD, BAR = 17, 24, 22, 34
COLORS = {
    "": (201, 209, 217), "m": (94, 234, 212), "g": (63, 185, 80), "r": (248, 81, 73),
    "d": (125, 133, 144), "b": (240, 246, 252), "y": (210, 153, 34), "p": (63, 185, 80),
}
BG, CHROME = (13, 17, 23), (22, 27, 34)
SPIN = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
SYMBOLS = set("▸✓✗→·↩✂≡" + SPIN)

font = ImageFont.truetype(FONT, SIZE)
bold = ImageFont.truetype(BOLD, SIZE)
symbol = ImageFont.truetype(SYMBOL, SIZE + 1)
CELL = font.getlength("M")


def spans(markup):
    """'[m]mint[/] plain' -> [(color, text), ...]"""
    out, color = [], ""
    for part in re.split(r"(\[[a-z]?\]|\[/\])", markup):
        if re.fullmatch(r"\[[a-z]?\]", part):
            color = part[1:-1]
        elif part == "[/]":
            color = ""
        elif part:
            out.append((color, part))
    return out


def render(lines, width, height, cursor=False):
    img = Image.new("RGB", (width, height), BG)
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, width, BAR], fill=CHROME)
    for i, c in enumerate([(255, 95, 86), (255, 189, 46), (39, 201, 63)]):
        d.ellipse([16 + i * 22, 11, 28 + i * 22, 23], fill=c)
    d.text((width / 2, BAR / 2), "shrink-anything", font=font, fill=COLORS["d"], anchor="mm")
    y = BAR + PAD - 4
    last_x = PAD
    for line in lines:
        x = PAD
        for color, text in spans(line):
            for ch in text:
                fill = COLORS[color]
                if ch in SYMBOLS:
                    d.text((x + CELL / 2, y + LINE / 2 - 2), ch, font=symbol, fill=fill, anchor="mm")
                else:
                    d.text((x, y + 2), ch, font=bold if color in ("b", "m") else font, fill=fill)
                x += CELL
        last_x = x
        y += LINE
    if cursor:
        d.rectangle([last_x + 2, y - LINE + 4, last_x + CELL - 1, y - 4], fill=COLORS[""])
    return img


def make(name, script, width, rows):
    """script: list of ("type", text) | ("out", line, ms) | ("spin", label, seconds) | ("wait", ms)"""
    height = BAR + PAD * 2 + rows * LINE
    frames, durations, lines = [], [], []

    def snap(ms, cursor=False):
        frames.append(render(lines[-rows:], width, height, cursor))
        durations.append(ms)

    for step in script:
        kind = step[0]
        if kind == "type":
            lines.append("[p]$[/] ")
            text = step[1]
            for i in range(0, len(text), 3):
                lines[-1] = "[p]$[/] [b]" + text[: i + 3] + "[/]"
                snap(45, cursor=True)
            snap(450, cursor=True)
        elif kind == "out":
            lines.append(step[1])
            snap(step[2] if len(step) > 2 else 90)
        elif kind == "spin":
            lines.append("")
            for i in range(int(step[2] * 8)):
                lines[-1] = "[m]" + SPIN[i % 10] + "[/] " + step[1]
                snap(110)
            lines[-1] = "[m]▸[/] " + step[1]
            snap(60)
        elif kind == "wait":
            snap(step[1])

    palette = frames[-1].convert("P", palette=Image.ADAPTIVE, colors=64)
    frames = [f.quantize(palette=palette, dither=Image.Dither.NONE) for f in frames]
    frames[0].save(OUT / name, save_all=True, append_images=frames[1:], duration=durations, loop=0, optimize=True, disposal=1)
    print(name, len(frames), "frames,", (OUT / name).stat().st_size // 1024, "KB")


# examples/bloated-ts
GO = [
    ("type", "shrink-anything go -- npm test"),
    ("out", "[m]›‹ shrink-anything[/] [d]v0.1.0[/]"),
    ("spin", "checking the project as it is: [b]npm test[/]", 0.8),
    ("out", "[g]✓[/] passes in 220ms (3 tests)", 250),
    ("spin", "round 1: cutting 8 things (36 lines) and running the check", 1.2),
    ("out", "  [g]8[/] kept", 250),
    ("spin", "round 2: cutting 3 things (7 lines) and running the check", 1.0),
    ("out", "  [g]3[/] kept", 250),
    ("out", "[g]✓[/] 106 → [b]58[/] lines ([g]-45%[/]) · 5 → [b]3[/] files ([g]-40%[/]) · 3 → [b]1[/] dependencies ([g]-67%[/])", 500),
    ("out", "  3 tests still run and pass · the check ran 4 times", 200),
    ("out", "[d]  take it back with: shrink-anything undo[/]", 600),
    ("out", ""),
    ("out", "  Removed: 7 pieces of dead code, 2 dead files, 2 unused dependencies."),
    ("out", "  Checked with [b]npm test[/]: 3 tests ran before, 3 after.", 5000),
]

# a function only a test reaches, through a name read from a data file
PUT_BACK = [
    ("type", "shrink-anything cut -- python -m unittest"),
    ("out", "[m]›‹ shrink-anything[/] [d]v0.1.0[/]"),
    ("spin", "checking the project as it is: [b]python -m unittest[/]", 0.7),
    ("out", "[g]✓[/] passes in 186ms (2 tests)", 250),
    ("spin", "round 1: cutting 2 things (4 lines) and running the check", 1.4),
    ("out", "  [g]1[/] kept, [y]1[/] put back", 700),
    ("spin", "round 2: cutting 1 thing (2 lines) and running the check", 0.9),
    ("out", "  [g]1[/] kept", 250),
    ("spin", "checking the final state", 0.6),
    ("out", "  [y]↩[/] secretly_used                  [d]the check failed: FAILED (errors=1)[/]", 1200),
    ("out", "[g]✓[/] 29 → [b]18[/] lines ([g]-38%[/])", 300),
    ("out", "  2 tests still run and pass · the check ran 5 times", 200),
    ("out", "[d]  take it back with: shrink-anything undo[/]", 5000),
]

if __name__ == "__main__":
    make("demo.gif", GO, 940, 15)
    make("put-back.gif", PUT_BACK, 940, 14)
