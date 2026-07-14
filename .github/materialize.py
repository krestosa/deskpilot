from __future__ import annotations

import base64
import io
import pathlib
import tarfile

root = pathlib.Path(".").resolve()
parts = sorted((root / ".deskpilot-bootstrap").glob("part-*"))
if not parts:
    raise SystemExit("no transport parts found")
texts = ["".join(path.read_text(encoding="utf-8").split()) for path in parts]
print("parts:", [(path.name, len(text)) for path, text in zip(parts, texts)])

candidates: list[tuple[str, bytes]] = []
try:
    candidates.append(
        ("continuous-base64", base64.b64decode("".join(texts), validate=True))
    )
except Exception as error:
    print(f"continuous-base64 rejected: {error}")
try:
    candidates.append(
        (
            "per-part-base64",
            b"".join(base64.b64decode(text, validate=True) for text in texts),
        )
    )
except Exception as error:
    print(f"per-part-base64 rejected: {error}")

selected: tuple[str, bytes] | None = None
for mode, payload in candidates:
    try:
        with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
            members = archive.getmembers()
            for member in members:
                target = (root / member.name).resolve()
                if root != target and root not in target.parents:
                    raise RuntimeError(f"unsafe archive member: {member.name}")
            if not any(member.name.endswith("Cargo.toml") for member in members):
                raise RuntimeError("archive does not contain Cargo.toml")
        selected = (mode, payload)
        break
    except Exception as error:
        print(f"{mode} archive rejected: {error}")

if selected is None:
    raise SystemExit("no valid source archive candidate")
mode, payload = selected
print(f"selected {mode}: {len(payload)} bytes from {len(parts)} parts")
with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
    archive.extractall(root, filter="data")
