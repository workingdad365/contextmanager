"""다국어(en, ko, zh, ja) 앱 화면 스크린샷 자동 캡처 스크립트 -> assets/screenshot-<lang>.png
타이틀바(버전 표시)까지 포함하여 Win32 PrintWindow 로 깨끗하게 캡처함.
"""
import ctypes
from ctypes import wintypes
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
EXE = ROOT / "target" / "debug" / "ContextManager.exe"
LANGS = ["en", "ko", "zh", "ja"]

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
dwmapi = ctypes.windll.dwmapi

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    try:
        user32.SetProcessDPIAware()
    except Exception:
        pass


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [
        ("biSize", wintypes.DWORD),
        ("biWidth", wintypes.LONG),
        ("biHeight", wintypes.LONG),
        ("biPlanes", wintypes.WORD),
        ("biBitCount", wintypes.WORD),
        ("biCompression", wintypes.DWORD),
        ("biSizeImage", wintypes.DWORD),
        ("biXPelsPerMeter", wintypes.LONG),
        ("biYPelsPerMeter", wintypes.LONG),
        ("biClrUsed", wintypes.DWORD),
        ("biClrImportant", wintypes.DWORD),
    ]


class BITMAPINFO(ctypes.Structure):
    _fields_ = [("bmiHeader", BITMAPINFOHEADER), ("bmiColors", wintypes.DWORD * 3)]


def find_window_by_pid(pid: int) -> int:
    found = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def enum_cb(hwnd, _lparam):
        if not user32.IsWindowVisible(hwnd):
            return True
        proc_id = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(proc_id))
        if proc_id.value == pid:
            rect = wintypes.RECT()
            user32.GetWindowRect(hwnd, ctypes.byref(rect))
            if (rect.right - rect.left) > 300 and (rect.bottom - rect.top) > 200:
                found.append(hwnd)
                return False
        return True

    user32.EnumWindows(enum_cb, 0)
    return found[0] if found else 0


def capture_hwnd(hwnd: int) -> Image.Image:
    rect = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    w = rect.right - rect.left
    h = rect.bottom - rect.top

    hwnd_dc = user32.GetWindowDC(hwnd)
    mem_dc = gdi32.CreateCompatibleDC(hwnd_dc)
    bmp = gdi32.CreateCompatibleBitmap(hwnd_dc, w, h)
    old = gdi32.SelectObject(mem_dc, bmp)

    # PW_RENDERFULLCONTENT = 2 (DirectX/wgpu 창까지 정상 캡처)
    ok = user32.PrintWindow(hwnd, mem_dc, 2)
    if not ok:
        user32.PrintWindow(hwnd, mem_dc, 0)

    bmi = BITMAPINFO()
    bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    bmi.bmiHeader.biWidth = w
    bmi.bmiHeader.biHeight = -h  # top-down
    bmi.bmiHeader.biPlanes = 1
    bmi.bmiHeader.biBitCount = 32
    bmi.bmiHeader.biCompression = 0  # BI_RGB

    buf = ctypes.create_string_buffer(w * h * 4)
    gdi32.GetDIBits(mem_dc, bmp, 0, h, buf, ctypes.byref(bmi), 0)

    gdi32.SelectObject(mem_dc, old)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem_dc)
    user32.ReleaseDC(hwnd, hwnd_dc)

    img = Image.frombuffer("RGBA", (w, h), buf, "raw", "BGRA", 0, 1).copy()

    # DWM 보이지 않는 그림자 테두리 크롭
    dwm_rect = wintypes.RECT()
    DWMWA_EXTENDED_FRAME_BOUNDS = 9
    if dwmapi.DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, ctypes.byref(dwm_rect), ctypes.sizeof(dwm_rect)) == 0:
        left = max(0, dwm_rect.left - rect.left)
        top = max(0, dwm_rect.top - rect.top)
        right = min(w, dwm_rect.right - rect.left)
        bottom = min(h, dwm_rect.bottom - rect.top)
        if right > left and bottom > top:
            img = img.crop((left, top, right, bottom))

    return img.convert("RGB")


def capture_one(lang: str):
    with tempfile.TemporaryDirectory() as td:
        ready_path = Path(td) / f"ready_{lang}.txt"
        cmd = [str(EXE), "--lang", lang, "--ready-file", str(ready_path)]
        print(f"Capturing [{lang}] ...")
        proc = subprocess.Popen(cmd)
        try:
            deadline = time.time() + 20.0
            while not ready_path.is_file():
                if proc.poll() is not None:
                    raise RuntimeError(f"Process exited prematurely ({proc.returncode})")
                if time.time() > deadline:
                    raise TimeoutError(f"Timed out waiting for {lang} window ready")
                time.sleep(0.1)

            time.sleep(0.35)
            hwnd = find_window_by_pid(proc.pid)
            if not hwnd:
                raise RuntimeError(f"Window handle not found for PID {proc.pid}")
            img = capture_hwnd(hwnd)
            out_path = ASSETS / f"screenshot-{lang}.png"
            img.save(out_path, optimize=True)
            print(f"  saved {out_path} ({img.width}x{img.height})")
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()


def main():
    if not EXE.is_file():
        print(f"Debug binary not found: {EXE}", file=sys.stderr)
        sys.exit(1)
    ASSETS.mkdir(exist_ok=True)
    for lang in LANGS:
        capture_one(lang)


if __name__ == "__main__":
    main()
