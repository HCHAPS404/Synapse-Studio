"""E5.2 (daemon side): verify signed packages, quarantine on failure, NE1108."""

from __future__ import annotations

import hashlib
import io
import json
import zipfile
from datetime import UTC, datetime
from pathlib import Path

import pytest
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from synapse_daemon.errors import DaemonError, ErrorCode
from synapse_daemon.packages import (
    PackageFile,
    PackageManifest,
    canonical_manifest_bytes,
    quarantine_package,
    verify_package,
)


def _key() -> tuple[Ed25519PrivateKey, str]:
    k = Ed25519PrivateKey.generate()
    pub = k.public_key().public_bytes(
        encoding=serialization.Encoding.Raw, format=serialization.PublicFormat.Raw
    )
    import base64

    return k, base64.b64encode(pub).decode("ascii")


def _manifest(files: dict[str, bytes]) -> PackageManifest:
    entries = [
        PackageFile(path=p, sha256=hashlib.sha256(d).hexdigest(), size=len(d))
        for p, d in sorted(files.items())
    ]
    return PackageManifest(
        device="mcu1",
        build_id="b-1",
        files=entries,
        created_at=datetime(2026, 11, 23, tzinfo=UTC),
    )


def _zip(
    manifest: PackageManifest,
    files: dict[str, bytes],
    key: Ed25519PrivateKey,
    *,
    manifest_override: bytes | None = None,
    sig_override: bytes | None = None,
) -> bytes:
    sig = sig_override or key.sign(canonical_manifest_bytes(manifest))
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        zf.writestr("manifest.json", manifest_override or manifest.model_dump_json(indent=2))
        zf.writestr("manifest.sig", sig)
        for p, d in files.items():
            zf.writestr(p, d)
    return buf.getvalue()


def _write(tmp_path: Path, data: bytes) -> Path:
    p = tmp_path / "pkg.zip"
    p.write_bytes(data)
    return p


def test_verify_good_package_extracts(tmp_path: Path) -> None:
    key, pub = _key()
    files = {"firmware.bin": b"\x00\x01\x02", "notes.txt": b"hello"}
    pkg = _write(tmp_path, _zip(_manifest(files), files, key))
    out = tmp_path / "out"
    result = verify_package(pkg, trusted_keys_b64=[pub], out_dir=out)
    assert result.manifest.device == "mcu1"
    assert (out / "firmware.bin").read_bytes() == b"\x00\x01\x02"


def test_wrong_key_is_ne1108_and_nothing_written(tmp_path: Path) -> None:
    key, _ = _key()
    _, other_pub = _key()
    files = {"a.bin": b"x"}
    pkg = _write(tmp_path, _zip(_manifest(files), files, key))
    out = tmp_path / "out"
    with pytest.raises(DaemonError) as exc:
        verify_package(pkg, trusted_keys_b64=[other_pub], out_dir=out)
    assert exc.value.code is ErrorCode.SIGNATURE_INVALID
    assert not out.exists()


def test_tampered_file_is_ne1108(tmp_path: Path) -> None:
    key, pub = _key()
    files = {"a.bin": b"original"}
    manifest = _manifest(files)
    tampered = {"a.bin": b"TAMPERED"}  # content differs from the manifest hash
    pkg = _write(tmp_path, _zip(manifest, tampered, key))
    with pytest.raises(DaemonError) as exc:
        verify_package(pkg, trusted_keys_b64=[pub], out_dir=tmp_path / "out")
    assert exc.value.code is ErrorCode.SIGNATURE_INVALID


def test_tampered_manifest_breaks_signature(tmp_path: Path) -> None:
    key, pub = _key()
    files = {"a.bin": b"x"}
    manifest = _manifest(files)
    # Change the manifest bytes after signing: device renamed; the signature no longer matches.
    bad = json.loads(manifest.model_dump_json())
    bad["device"] = "other"
    pkg = _write(tmp_path, _zip(manifest, files, key, manifest_override=json.dumps(bad).encode()))
    with pytest.raises(DaemonError) as exc:
        verify_package(pkg, trusted_keys_b64=[pub], out_dir=tmp_path / "out")
    assert exc.value.code is ErrorCode.SIGNATURE_INVALID


def test_unsigned_package_is_ne1108(tmp_path: Path) -> None:
    key, pub = _key()
    files = {"a.bin": b"x"}
    manifest = _manifest(files)
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        zf.writestr("manifest.json", manifest.model_dump_json())
        for p, d in files.items():
            zf.writestr(p, d)  # no manifest.sig
    pkg = _write(tmp_path, buf.getvalue())
    with pytest.raises(DaemonError) as exc:
        verify_package(pkg, trusted_keys_b64=[pub], out_dir=tmp_path / "out")
    assert exc.value.code is ErrorCode.SIGNATURE_INVALID


def test_extra_unlisted_file_is_ne1108(tmp_path: Path) -> None:
    key, pub = _key()
    files = {"a.bin": b"x"}
    manifest = _manifest(files)
    with_extra = {**files, "stowaway.sh": b"rm -rf /"}
    pkg = _write(tmp_path, _zip(manifest, with_extra, key))
    with pytest.raises(DaemonError) as exc:
        verify_package(pkg, trusted_keys_b64=[pub], out_dir=tmp_path / "out")
    assert exc.value.code is ErrorCode.SIGNATURE_INVALID


def test_no_trusted_keys_is_ne1108(tmp_path: Path) -> None:
    key, _ = _key()
    files = {"a.bin": b"x"}
    pkg = _write(tmp_path, _zip(_manifest(files), files, key))
    with pytest.raises(DaemonError) as exc:
        verify_package(pkg, trusted_keys_b64=[], out_dir=tmp_path / "out")
    assert exc.value.code is ErrorCode.SIGNATURE_INVALID


def test_canonical_bytes_are_a_stable_vector() -> None:
    # This exact byte string is also asserted by the API-side test, so a package the API
    # signs verifies on the daemon. Changing either side without the other breaks it.
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


def test_quarantine_keeps_a_copy(tmp_path: Path) -> None:
    pkg = _write(tmp_path, b"not a real zip")
    qdir = tmp_path / "q"
    dest = quarantine_package(pkg, qdir)
    assert dest.exists() and dest.parent == qdir
