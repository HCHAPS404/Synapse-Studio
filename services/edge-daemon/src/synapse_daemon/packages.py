"""Signed deployment packages: the manifest, canonical bytes, verification and quarantine.

A package is a zip with `manifest.json`, the artifact files, and `manifest.sig` (an Ed25519
signature over the canonical manifest bytes). The daemon trusts the public keys in its
config and applies a package only when: the signature matches a trusted key, and every file
in the zip matches its SHA-256 in the manifest. Any failure is `NE1108`; the package is kept
only in a quarantine folder and is never applied.

The API signs with `SYNAPSE_SIGNING_KEY` (E5.2, `synapse_core.packages`); the two sides
agree on the canonical manifest bytes defined by `canonical_manifest_bytes`.
"""

from __future__ import annotations

import base64
import hashlib
import json
import zipfile
from datetime import datetime
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from pydantic import BaseModel, Field

from synapse_daemon.errors import DaemonError, ErrorCode

MANIFEST_NAME = "manifest.json"
SIGNATURE_NAME = "manifest.sig"


class PackageFile(BaseModel):
    """One artifact file inside the package, bound to its content by SHA-256."""

    model_config = {"extra": "forbid"}

    path: str = Field(description="Relative POSIX path inside the package")
    sha256: str = Field(pattern=r"^[0-9a-f]{64}$")
    size: int = Field(ge=0)


class PackageManifest(BaseModel):
    """What a package contains and which device and build it is for."""

    model_config = {"extra": "forbid"}

    device: str
    build_id: str
    files: list[PackageFile]
    created_at: datetime
    min_daemon: str = Field(default="0.1.0", description="Lowest daemon version that may apply it")


def canonical_manifest_bytes(manifest: PackageManifest) -> bytes:
    """The exact bytes that are signed and verified.

    Both the signer (API) and the verifier (daemon) serialise the manifest the same way:
    JSON with sorted keys, no spaces, `created_at` as an ISO-8601 string. This is the single
    definition of "the manifest bytes"; changing it breaks every existing signature.
    """
    payload = manifest.model_dump(mode="json")
    return json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")


def _load_trusted_keys(trusted_b64: list[str]) -> list[Ed25519PublicKey]:
    keys: list[Ed25519PublicKey] = []
    for b64 in trusted_b64:
        try:
            raw = base64.b64decode(b64, validate=True)
            keys.append(Ed25519PublicKey.from_public_bytes(raw))
        except (ValueError, TypeError) as exc:
            raise DaemonError(
                ErrorCode.SIGNATURE_INVALID, f"trusted key is not valid: {exc}"
            ) from exc
    return keys


def _signature_ok(keys: list[Ed25519PublicKey], message: bytes, signature: bytes) -> bool:
    for key in keys:
        try:
            key.verify(signature, message)
            return True
        except InvalidSignature:
            continue
    return False


class VerifiedPackage(BaseModel):
    """A package that passed verification, with its extracted files on disk."""

    model_config = {"extra": "forbid"}

    manifest: PackageManifest
    out_dir: str


def verify_package(
    zip_path: str | Path, *, trusted_keys_b64: list[str], out_dir: str | Path
) -> VerifiedPackage:
    """Verify a package zip and extract it to `out_dir`. Raises `DaemonError(NE1108)` on failure.

    Checks, in order: the zip has a manifest and a signature; the manifest parses; the
    signature over the canonical manifest bytes matches a trusted key; every file listed in
    the manifest is present with a matching SHA-256 and size; the zip holds no file outside
    the manifest and no unsafe path. Only after all checks pass are files written to
    `out_dir`. Nothing is written when any check fails.
    """
    if not trusted_keys_b64:
        raise DaemonError(ErrorCode.SIGNATURE_INVALID, "no trusted keys configured")
    keys = _load_trusted_keys(trusted_keys_b64)

    try:
        with zipfile.ZipFile(zip_path) as zf:
            names = set(zf.namelist())
            if MANIFEST_NAME not in names or SIGNATURE_NAME not in names:
                raise DaemonError(
                    ErrorCode.SIGNATURE_INVALID, "package is missing manifest or signature"
                )
            manifest_bytes = zf.read(MANIFEST_NAME)
            signature = zf.read(SIGNATURE_NAME)
            try:
                manifest = PackageManifest.model_validate_json(manifest_bytes)
            except ValueError as exc:
                raise DaemonError(ErrorCode.SIGNATURE_INVALID, f"manifest invalid: {exc}") from exc

            # The manifest is re-serialised canonically; the file on disk may differ in
            # spacing, so the signature is over the canonical form, not the stored bytes.
            if not _signature_ok(keys, canonical_manifest_bytes(manifest), signature):
                raise DaemonError(ErrorCode.SIGNATURE_INVALID, "signature does not match a key")

            listed = {f.path for f in manifest.files}
            extra = names - listed - {MANIFEST_NAME, SIGNATURE_NAME}
            if extra:
                raise DaemonError(
                    ErrorCode.SIGNATURE_INVALID, f"package has unlisted files: {sorted(extra)}"
                )
            contents: dict[str, bytes] = {}
            for f in manifest.files:
                if _unsafe_path(f.path):
                    raise DaemonError(ErrorCode.SIGNATURE_INVALID, f"unsafe path {f.path!r}")
                if f.path not in names:
                    raise DaemonError(ErrorCode.SIGNATURE_INVALID, f"missing file {f.path!r}")
                data = zf.read(f.path)
                if len(data) != f.size or hashlib.sha256(data).hexdigest() != f.sha256:
                    raise DaemonError(ErrorCode.SIGNATURE_INVALID, f"file {f.path!r} tampered")
                contents[f.path] = data
    except zipfile.BadZipFile as exc:
        raise DaemonError(ErrorCode.SIGNATURE_INVALID, "not a valid zip") from exc

    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    for path, data in contents.items():
        dest = out / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(data)
    return VerifiedPackage(manifest=manifest, out_dir=str(out))


def quarantine_package(zip_path: str | Path, quarantine_dir: str | Path) -> Path:
    """Move a rejected package into the quarantine folder and return its new path."""
    src = Path(zip_path)
    qdir = Path(quarantine_dir)
    qdir.mkdir(parents=True, exist_ok=True)
    dest = qdir / src.name
    data = src.read_bytes()
    dest.write_bytes(data)
    return dest


def _unsafe_path(path: str) -> bool:
    if not path or path.startswith(("/", "\\")) or ":" in path:
        return True
    parts = path.replace("\\", "/").split("/")
    return any(p in ("", ".", "..") for p in parts)
