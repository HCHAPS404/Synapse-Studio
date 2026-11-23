"""Signing of deployment packages on the API side (E5.2).

A deployment package is a zip with `manifest.json` (device, build id, files with SHA-256,
`created_at`, `min_daemon`), the artifact files, and `manifest.sig` (an Ed25519 signature
over the canonical manifest bytes). The daemon (`synapse_daemon.packages`) verifies it and
rejects a tampered or wrongly-signed package with `NE1108`. The two sides agree on the
canonical manifest bytes defined here.

The private key comes from `SYNAPSE_SIGNING_KEY` (base64 Ed25519 seed, 32 bytes); it is
never committed and never logged.
"""

from __future__ import annotations

import base64
import hashlib
import io
import json
import zipfile
from collections.abc import Mapping
from datetime import UTC, datetime

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from pydantic import BaseModel, Field

MANIFEST_NAME = "manifest.json"
SIGNATURE_NAME = "manifest.sig"


class PackageFile(BaseModel):
    """One artifact file in the package, bound to its content by SHA-256."""

    model_config = {"extra": "forbid"}

    path: str
    sha256: str = Field(pattern=r"^[0-9a-f]{64}$")
    size: int = Field(ge=0)


class PackageManifest(BaseModel):
    """What a package contains and which device and build it is for."""

    model_config = {"extra": "forbid"}

    device: str
    build_id: str
    files: list[PackageFile]
    created_at: datetime
    min_daemon: str = "0.1.0"


def canonical_manifest_bytes(manifest: PackageManifest) -> bytes:
    """The exact bytes that are signed. Must match `synapse_daemon.packages`."""
    payload = manifest.model_dump(mode="json")
    return json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")


def load_signing_key(seed_b64: str) -> Ed25519PrivateKey:
    """Load the Ed25519 private key from a base64 32-byte seed (from the environment)."""
    seed = base64.b64decode(seed_b64, validate=True)
    if len(seed) != 32:
        raise ValueError("SYNAPSE_SIGNING_KEY must be a base64 32-byte Ed25519 seed")
    return Ed25519PrivateKey.from_private_bytes(seed)


def public_key_b64(key: Ed25519PrivateKey) -> str:
    """The base64 public key a daemon adds to its trusted keys."""
    from cryptography.hazmat.primitives import serialization

    raw = key.public_key().public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )
    return base64.b64encode(raw).decode("ascii")


def build_manifest(
    *,
    device: str,
    build_id: str,
    files: Mapping[str, bytes],
    min_daemon: str = "0.1.0",
    created_at: datetime | None = None,
) -> PackageManifest:
    """Build a manifest from the artifact bytes, hashing each file."""
    entries = [
        PackageFile(path=path, sha256=hashlib.sha256(data).hexdigest(), size=len(data))
        for path, data in sorted(files.items())
    ]
    return PackageManifest(
        device=device,
        build_id=build_id,
        files=entries,
        created_at=created_at or datetime.now(UTC),
        min_daemon=min_daemon,
    )


def sign_package(
    manifest: PackageManifest, files: Mapping[str, bytes], key: Ed25519PrivateKey
) -> bytes:
    """Produce the signed package zip bytes.

    The manifest file in the zip is the pretty JSON for readability; the signature is over
    the canonical bytes, so the daemon re-serialises before verifying and spacing cannot
    break a signature.
    """
    signature = key.sign(canonical_manifest_bytes(manifest))
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr(MANIFEST_NAME, manifest.model_dump_json(indent=2))
        zf.writestr(SIGNATURE_NAME, signature)
        for f in manifest.files:
            zf.writestr(f.path, files[f.path])
    return buf.getvalue()
