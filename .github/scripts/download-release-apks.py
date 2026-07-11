#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
import re
import shutil
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath


SHA256_RE = re.compile(r"[0-9a-f]{64}")


def parse_args():
    parser = argparse.ArgumentParser(
        description="Download and verify APKs declared in a release manifest."
    )
    parser.add_argument("manifest", type=Path)
    parser.add_argument("feed_dir", type=Path)
    parser.add_argument(
        "--only",
        action="append",
        default=[],
        metavar="PACKAGE_ID",
        help="download only the selected package group; may be repeated",
    )
    return parser.parse_args()


def load_manifest(path):
    with path.open(encoding="utf-8") as file_obj:
        manifest = json.load(file_obj)

    if manifest.get("schema_version") != 1:
        raise SystemExit(f"Unsupported manifest schema in {path}")
    if not isinstance(manifest.get("packages"), list):
        raise SystemExit(f"Manifest packages must be a list in {path}")
    return manifest


def validate_name(name, label):
    if not isinstance(name, str) or not name.endswith(".apk"):
        raise SystemExit(f"Invalid {label}: {name!r}")
    if Path(name).name != name or PurePosixPath(name).name != name:
        raise SystemExit(f"{label} must be a plain APK filename: {name!r}")


def validate_sha256(digest, label):
    if not isinstance(digest, str) or not SHA256_RE.fullmatch(digest):
        raise SystemExit(f"Invalid SHA256 for {label}: {digest!r}")


def validate_url(url, label):
    if not isinstance(url, str) or not url.startswith("https://"):
        raise SystemExit(f"Invalid HTTPS URL for {label}: {url!r}")


def validate_manifest(packages):
    package_ids = set()
    outputs = set()

    for package in packages:
        package_id = package.get("id")
        if not isinstance(package_id, str) or not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", package_id):
            raise SystemExit(f"Invalid package id: {package_id!r}")
        if package_id in package_ids:
            raise SystemExit(f"Duplicate package id: {package_id}")
        package_ids.add(package_id)

        artifacts = package.get("artifacts")
        if not isinstance(artifacts, list) or not artifacts:
            raise SystemExit(f"Package {package_id} has no artifacts")

        for artifact in artifacts:
            artifact_type = artifact.get("type")
            validate_url(artifact.get("url"), package_id)
            validate_sha256(artifact.get("sha256"), package_id)

            if artifact_type == "file":
                files = [artifact]
            elif artifact_type == "archive":
                if artifact.get("format") not in ("zip", "tar.gz"):
                    raise SystemExit(f"Unsupported archive format for {package_id}")
                files = artifact.get("files")
                if not isinstance(files, list) or not files:
                    raise SystemExit(f"Archive for {package_id} has no APK files")
            else:
                raise SystemExit(f"Unsupported artifact type for {package_id}: {artifact_type!r}")

            sources = set()
            for file_entry in files:
                output = file_entry.get("output")
                validate_name(output, f"output for {package_id}")
                validate_sha256(file_entry.get("sha256"), output)
                if output in outputs:
                    raise SystemExit(f"Duplicate output filename: {output}")
                outputs.add(output)

                if artifact_type == "archive":
                    source = file_entry.get("source")
                    validate_name(source, f"archive source for {package_id}")
                    if source in sources:
                        raise SystemExit(f"Duplicate archive source filename: {source}")
                    sources.add(source)


def file_sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as file_obj:
        for chunk in iter(lambda: file_obj.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download(url, destination, expected_sha256):
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "openwrt-packages-manifest-downloader"},
    )
    last_error = None

    for attempt in range(1, 4):
        try:
            with urllib.request.urlopen(request, timeout=180) as response:
                with destination.open("wb") as file_obj:
                    shutil.copyfileobj(response, file_obj, length=1024 * 1024)
            actual_sha256 = file_sha256(destination)
            if actual_sha256 != expected_sha256:
                raise SystemExit(
                    f"SHA256 mismatch for {url}: expected {expected_sha256}, got {actual_sha256}"
                )
            return
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            last_error = exc
            destination.unlink(missing_ok=True)
            if attempt < 3:
                print(f"Download attempt {attempt}/3 failed: {exc}; retrying...")
                time.sleep(2)

    raise SystemExit(f"Failed to download {url}: {last_error}")


def copy_verified(source_obj, destination, expected_sha256):
    digest = hashlib.sha256()
    temporary = destination.with_name(f".{destination.name}.download")
    try:
        with temporary.open("wb") as output:
            while True:
                chunk = source_obj.read(1024 * 1024)
                if not chunk:
                    break
                digest.update(chunk)
                output.write(chunk)
        actual_sha256 = digest.hexdigest()
        if actual_sha256 != expected_sha256:
            raise SystemExit(
                f"SHA256 mismatch for {destination.name}: expected {expected_sha256}, got {actual_sha256}"
            )
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def install_file(artifact, feed_dir, work_dir):
    output = artifact["output"]
    downloaded = work_dir / output
    print(f"Downloading {output}")
    download(artifact["url"], downloaded, artifact["sha256"])
    os.replace(downloaded, feed_dir / output)


def archive_members(archive, archive_format):
    if archive_format == "zip":
        with zipfile.ZipFile(archive) as file_obj:
            members = {}
            for info in file_obj.infolist():
                if info.is_dir() or not info.filename.endswith(".apk"):
                    continue
                name = PurePosixPath(info.filename).name
                if name in members:
                    raise SystemExit(f"Duplicate APK basename in archive: {name}")
                members[name] = info
            yield members, lambda member: file_obj.open(member)
        return

    with tarfile.open(archive, mode="r:gz") as file_obj:
        members = {}
        for member in file_obj.getmembers():
            if not member.isfile() or not member.name.endswith(".apk"):
                continue
            name = PurePosixPath(member.name).name
            if name in members:
                raise SystemExit(f"Duplicate APK basename in archive: {name}")
            members[name] = member
        yield members, lambda member: file_obj.extractfile(member)


def install_archive(package_id, artifact, feed_dir, work_dir):
    archive_path = work_dir / f"{package_id}.{artifact['format'].replace('.', '-')}"
    print(f"Downloading archive for {package_id}")
    download(artifact["url"], archive_path, artifact["sha256"])

    for members, opener in archive_members(archive_path, artifact["format"]):
        for file_entry in artifact["files"]:
            source = file_entry["source"]
            member = members.get(source)
            if member is None:
                available = ", ".join(sorted(members))
                raise SystemExit(
                    f"Missing {source} in {package_id} archive; available APKs: {available}"
                )
            source_obj = opener(member)
            if source_obj is None:
                raise SystemExit(f"Cannot read {source} from {package_id} archive")
            with source_obj:
                copy_verified(source_obj, feed_dir / file_entry["output"], file_entry["sha256"])


def main():
    args = parse_args()
    manifest = load_manifest(args.manifest)
    packages = manifest["packages"]
    validate_manifest(packages)

    selected = set(args.only)
    known_ids = {package["id"] for package in packages}
    unknown = selected - known_ids
    if unknown:
        raise SystemExit(f"Unknown package ids: {', '.join(sorted(unknown))}")

    args.feed_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="release-apks-") as temporary:
        work_dir = Path(temporary)
        for package in packages:
            package_id = package["id"]
            if selected and package_id not in selected:
                continue
            print(f"\n[{package_id}]")
            for artifact in package["artifacts"]:
                if artifact["type"] == "file":
                    install_file(artifact, args.feed_dir, work_dir)
                else:
                    install_archive(package_id, artifact, args.feed_dir, work_dir)

    count = len(list(args.feed_dir.glob("*.apk")))
    print(f"\nDownloaded and verified {count} APK files in {args.feed_dir}")


if __name__ == "__main__":
    main()
