"""Makes the binary files of examples/heavy-web: two images the page uses (one stored twice),
two it stopped using, a font it uses and one it doesn't. Noise, so they weigh like real assets.

    python scripts/example_assets.py
"""

import random
from pathlib import Path

from PIL import Image

OUT = Path(__file__).resolve().parent.parent / "examples" / "heavy-web"


def noise(path, size, seed):
    rnd = random.Random(seed)
    img = Image.frombytes("RGB", size, rnd.randbytes(size[0] * size[1] * 3))
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)


def blob(path, n, seed):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(random.Random(seed).randbytes(n))


if __name__ == "__main__":
    noise(OUT / "img" / "hero.png", (320, 180), 1)
    noise(OUT / "img" / "header.png", (320, 180), 1)  # the same picture under another name
    noise(OUT / "img" / "promo-summer.png", (320, 180), 2)
    noise(OUT / "img" / "team-2019.png", (400, 220), 3)
    blob(OUT / "fonts" / "Shop.woff2", 60_000, 4)
    blob(OUT / "fonts" / "OldDisplay.woff2", 90_000, 5)
    print("ok")
