from __future__ import annotations

import base64
import hashlib
import io
import pathlib
import tarfile

EXPECTED_SHA256 = "c3798e433b255ce39234a02b229c5bda904ae2fb765f0a316d51c90907a1d214"
MISSING_OFFSET = 3651
MISSING_CHARACTER = "n"

root = pathlib.Path(".").resolve()
parts = sorted((root / ".deskpilot-bootstrap").glob("part-*"))
if not parts:
    raise SystemExit("no transport parts found")
encoded = "".join(
    "".join(path.read_text(encoding="utf-8").split()) for path in parts
)

if len(encoded) % 4 == 3:
    encoded = encoded[:MISSING_OFFSET] + MISSING_CHARACTER + encoded[MISSING_OFFSET:]

payload = base64.b64decode(encoded, validate=True)
digest = hashlib.sha256(payload).hexdigest()
if digest != EXPECTED_SHA256:
    raise SystemExit(
        f"source archive checksum mismatch: expected {EXPECTED_SHA256}, got {digest}"
    )

with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
    members = archive.getmembers()
    for member in members:
        target = (root / member.name).resolve()
        if root != target and root not in target.parents:
            raise RuntimeError(f"unsafe archive member: {member.name}")
    if not any(member.name == "Cargo.toml" for member in members):
        raise RuntimeError("archive does not contain root Cargo.toml")
    archive.extractall(root, filter="data")

print(
    f"materialized {len(members)} archive entries from {len(parts)} parts; "
    f"sha256={digest}"
)
