from __future__ import annotations

import base64
import hashlib
import io
import pathlib
import string
import tarfile

EXPECTED_SHA256 = "c3798e433b255ce39234a02b229c5bda904ae2fb765f0a316d51c90907a1d214"
BASE64_ALPHABET = string.ascii_uppercase + string.ascii_lowercase + string.digits + "+/"

root = pathlib.Path(".").resolve()
parts = sorted((root / ".deskpilot-bootstrap").glob("part-*"))
if not parts:
    raise SystemExit("no transport parts found")
texts = ["".join(path.read_text(encoding="utf-8").split()) for path in parts]
print("parts:", [(path.name, len(text)) for path, text in zip(parts, texts)])

joined = "".join(texts)
selected: tuple[str, bytes] | None = None

def accept(mode: str, encoded: str) -> tuple[str, bytes] | None:
    try:
        payload = base64.b64decode(encoded, validate=True)
    except Exception:
        return None
    digest = hashlib.sha256(payload).hexdigest()
    if digest == EXPECTED_SHA256:
        return mode, payload
    return None

selected = accept("continuous-base64", joined)
if selected is None and len(joined) % 4 == 3:
    boundaries = []
    offset = 0
    for text in texts[:-1]:
        offset += len(text)
        boundaries.append(offset)
    for boundary in boundaries:
        for character in BASE64_ALPHABET:
            candidate = joined[:boundary] + character + joined[boundary:]
            selected = accept(
                f"repaired-boundary-{boundary}-char-{character}", candidate
            )
            if selected is not None:
                break
        if selected is not None:
            break

if selected is None:
    raise SystemExit(
        "transport could not be reconstructed to the expected SHA-256; refusing to extract"
    )

mode, payload = selected
print(f"selected {mode}: {len(payload)} bytes, sha256={EXPECTED_SHA256}")
with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
    members = archive.getmembers()
    for member in members:
        target = (root / member.name).resolve()
        if root != target and root not in target.parents:
            raise RuntimeError(f"unsafe archive member: {member.name}")
    if not any(member.name.endswith("Cargo.toml") for member in members):
        raise RuntimeError("archive does not contain Cargo.toml")
    archive.extractall(root, filter="data")
