#!/usr/bin/env python3
"""Build a reproducible native source bundle, validate it, and optionally sign it.

Release: python3 tools/package.py --sign-key /private/key --public-key /public/key.pub
Signing uses minisign; private keys are never copied into the source bundle.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import tomllib
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
SKIP_DIRS = {"target", ".git", ".local", "node_modules", "__pycache__", "tests", "examples"}
MAX_FILES = 512
MAX_BYTES = 64 * 1024 * 1024


def bundle_entries(root: pathlib.Path) -> tuple[pathlib.Path, list[tuple[str, bytes]]]:
    native = json.loads((root / "horizon-native.json").read_text(encoding="utf-8"))
    mod_id, version = native["id"], native["version"]
    if not mod_id or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789_" for c in mod_id):
        raise ValueError("invalid mod ID")
    if not version or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.+-" for c in version):
        raise ValueError("invalid mod version")
    mod_dir = root / "mixins" / mod_id
    mixin = tomllib.loads((mod_dir / "horizon-mixin.toml").read_text(encoding="utf-8"))
    if mixin["mod"]["id"] != mod_id or mixin["mod"]["version"] != version:
        raise ValueError("mod ID/version mismatch")
    if mixin["compatibility"]["horizon_revision"] != native["horizon"]:
        raise ValueError("Horizon revision mismatch")
    compatibility = json.loads((root / "compatibility.json").read_text(encoding="utf-8"))
    if compatibility.get("hooks") != native["hooks"]:
        raise ValueError("hooks version mismatch")
    for crate in mixin.get("crate", []):
        path = pathlib.PurePosixPath(crate["path"])
        if path.is_absolute() or ".." in path.parts:
            raise ValueError("crate path escapes bundle")
        manifest = tomllib.loads((mod_dir / path / "Cargo.toml").read_text(encoding="utf-8"))
        if manifest["package"]["name"] != crate["id"] or manifest["package"]["version"] != version:
            raise ValueError("crate name/version mismatch")
    entries = [("horizon-native.json", (root / "horizon-native.json").read_bytes())]
    for path in sorted(mod_dir.rglob("*")):
        relative = path.relative_to(root)
        if any(p in SKIP_DIRS for p in relative.parts):
            continue
        if path.is_symlink():
            raise ValueError(f"symlinks are not allowed: {relative}")
        if not path.is_file():
            continue
        # The registry accepts Rust sources and TOML manifests, not binary fixtures.
        if path.suffix not in {".rs", ".toml"}:
            raise ValueError(f"unexpected native source: {relative}")
        entries.append((relative.as_posix(), path.read_bytes()))
    if len(entries) > MAX_FILES or sum(len(data) for _, data in entries) > MAX_BYTES:
        raise ValueError("bundle exceeds registry limits (512 files / 64 MiB)")
    return root / f"{mod_id}-{version}.hnmod", entries


def verify_and_package(root: pathlib.Path = ROOT) -> pathlib.Path:
    out, entries = bundle_entries(root)
    fd, name = tempfile.mkstemp(prefix=".hnmod-", suffix=".hnmod", dir=root)
    os.close(fd)
    temporary = pathlib.Path(name)
    try:
        with zipfile.ZipFile(temporary, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for arcname, data in entries:
                info = zipfile.ZipInfo(arcname, date_time=(2026, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.create_system = 3
                info.external_attr = 0o100644 << 16
                archive.writestr(info, data)
        with zipfile.ZipFile(temporary) as archive:
            if archive.testzip() is not None:
                raise ValueError("archive CRC verification failed")
        # Use the official validator before replacing a previous release artifact.
        subprocess.run(["cargo", "horizon-mod", "check", str(temporary), "--json"], check=True)
        digest = hashlib.sha256(temporary.read_bytes()).hexdigest()
        if out.exists() and hashlib.sha256(out.read_bytes()).hexdigest() != digest:
            out.with_suffix(out.suffix + ".minisig").unlink(missing_ok=True)
        os.replace(temporary, out)
        out.with_suffix(out.suffix + ".sha256").write_text(f"{digest}  {out.name}\n", encoding="ascii")
    finally:
        temporary.unlink(missing_ok=True)
    print(f"Packaged and verified {out.name} ({len(entries)} files, {out.stat().st_size} bytes)")
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sign-key", type=pathlib.Path, help="existing minisign secret key")
    parser.add_argument("--public-key", type=pathlib.Path, help="matching minisign public key")
    args = parser.parse_args()
    if bool(args.sign_key) != bool(args.public_key):
        parser.error("--sign-key and --public-key are required together")
    out = verify_and_package()
    if args.sign_key:
        subprocess.run(["minisign", "-Sm", str(out), "-s", str(args.sign_key)], check=True)
        subprocess.run(["minisign", "-Vm", str(out), "-p", str(args.public_key)], check=True)
        # Registry's independent signature verification, using the base64 public key.
        key = args.public_key.read_text(encoding="ascii").splitlines()[1]
        subprocess.run(["cargo", "horizon-mod", "check", str(out), "--key", key,
                        "--signature", str(out) + ".minisig", "--json"], check=True)
    else:
        print("Unsigned bundle. Use --sign-key and --public-key to sign a release.")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)
