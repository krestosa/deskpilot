from pathlib import Path

path = Path("tools/harden_spare_confirmation.py")
text = path.read_text(encoding="utf-8")
old = "    unsafe { IsWindowVisible(hwnd) != 0 } && !window_is_cloaked(hwnd)\n"
new = "    (unsafe { IsWindowVisible(hwnd) != 0 }) && !window_is_cloaked(hwnd)\n"
if text.count(old) != 1:
    raise SystemExit("expected one unsafe visibility expression in hardening materializer")
path.write_text(text.replace(old, new, 1), encoding="utf-8", newline="\n")
print("hardening materializer syntax corrected")
