"""E5.2 (API side): signing deployment packages with Ed25519."""

from __future__ import annotations

import base64
import zipfile

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from synapse_core.packages import (
    build_manifest,
    canonical_manifest_bytes,
    load_signing_key,
    public_key_b64,
    sign_package,
)


def _seed_b64() -> str:
    key = Ed25519PrivateKey.generate()
    from cryptography.hazmat.primitives import serialization

    raw = key.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption(),
    )
    return base64.b64encode(raw).decode("ascii")


def test_load_signing_key_round_trip() -> None:
    seed = _seed_b64()
    key = load_signing_key(seed)
    assert isinstance(key, Ed25519PrivateKey)
    assert len(base64.b64decode(public_key_b64(key))) == 32


def test_build_manifest_hashes_files() -> None:
    files = {"fw.bin": b"\x01\x02\x03"}
    m = build_manifest(device="mcu1", build_id="b-1", files=files)
    assert m.files[0].size == 3
    import hashlib

    assert m.files[0].sha256 == hashlib.sha256(b"\x01\x02\x03").hexdigest()


def test_sign_package_produces_verifiable_zip() -> None:
    key = load_signing_key(_seed_b64())
    files = {"fw.bin": b"firmware", "cfg.json": b"{}"}
    manifest = build_manifest(device="mcu1", build_id="b-9", files=files)
    data = sign_package(manifest, files, key)

    with zipfile.ZipFile(__import__("io").BytesIO(data)) as zf:
        names = set(zf.namelist())
        assert {"manifest.json", "manifest.sig"} <= names
        assert {"fw.bin", "cfg.json"} <= names
        sig = zf.read("manifest.sig")

    # The signature verifies against the canonical bytes with the matching public key.
    key.public_key().verify(sig, canonical_manifest_bytes(manifest))


def test_canonical_bytes_match_the_daemon_vector() -> None:
    # The same vector the daemon test asserts: a package the API signs verifies on the daemon.
    from datetime import UTC, datetime

    from synapse_core.packages import PackageFile, PackageManifest

    m = PackageManifest(
        device="mcu1",
        build_id="b-1",
        files=[PackageFile(path="a.bin", sha256="ab" * 32, size=3)],
        created_at=datetime(2026, 11, 23, tzinfo=UTC),
        min_daemon="0.1.0",
    )
    assert canonical_manifest_bytes(m) == (
        b'{"build_id":"b-1","created_at":"2026-11-23T00:00:00Z",'
        b'"device":"mcu1","files":[{"path":"a.bin","sha256":"'
        + b"ab" * 32
        + b'","size":3}],"min_daemon":"0.1.0"}'
    )
