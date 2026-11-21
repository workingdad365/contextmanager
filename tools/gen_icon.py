"""ContextManager 아이콘 생성 스크립트 (assets/contextmanager.ico, assets/contextmanager.png)"""
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024
ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets"


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))


def render() -> Image.Image:
    # 청록 -> 딥블루 대각선 그라데이션 배경
    top, bottom = (14, 165, 233), (30, 58, 138)
    grad = Image.new("RGBA", (S, S))
    px = grad.load()
    for y in range(S):
        for x in range(S):
            px[x, y] = lerp(top, bottom, (x + y) / (2 * S)) + (255,)

    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((40, 40, S - 40, S - 40), radius=210, fill=255)
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    img.paste(grad, (0, 0), mask)

    d = ImageDraw.Draw(img)

    # 컨텍스트 메뉴 카드
    d.rounded_rectangle((185, 195, 795, 835), radius=72, fill=(15, 23, 42, 70))
    d.rounded_rectangle((165, 175, 775, 815), radius=72, fill=(255, 255, 255, 250))

    # 메뉴 항목 3줄
    rows = [
        (285, (14, 165, 233, 255), 420),
        (455, (59, 130, 246, 255), 360),
        (625, (239, 68, 68, 255), 310),
    ]
    for y, color, width in rows:
        d.rounded_rectangle((240, y, 300, y + 60), radius=16, fill=color)
        d.rounded_rectangle((335, y + 12, 335 + width, y + 48), radius=18, fill=(51, 65, 85, 220))

    # 우하단 정리(체크 배지)
    cx, cy, r = 770, 740, 145
    d.ellipse((cx - r + 10, cy - r + 12, cx + r + 10, cy + r + 12), fill=(15, 23, 42, 90))
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=(16, 185, 129, 255), outline=(255, 255, 255, 255), width=22)
    d.line([(cx - 58, cy + 4), (cx - 14, cy + 48), (cx + 66, cy - 42)], fill=(255, 255, 255, 255), width=32, joint="curve")
    return img


def main():
    OUT.mkdir(exist_ok=True)
    big = render()
    big.resize((256, 256), Image.LANCZOS).save(OUT / "contextmanager.png")
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    big.resize((256, 256), Image.LANCZOS).save(
        OUT / "contextmanager.ico", sizes=[(s, s) for s in sizes]
    )
    print("generated", OUT)


if __name__ == "__main__":
    main()
