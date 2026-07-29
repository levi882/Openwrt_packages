#!/usr/bin/env python3
import argparse
import json
import re
import subprocess
import uuid
from pathlib import Path


PACKAGE_COMPONENT_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+~:-]*")


def parse_args():
    parser = argparse.ArgumentParser(
        description="Rename APK v3 files to the canonical NAME-VERSION.apk form."
    )
    parser.add_argument("apk_bin", type=Path)
    parser.add_argument("packages", nargs="+", type=Path)
    return parser.parse_args()


def package_identity(metadata, source):
    identities = set()

    def visit(value):
        if isinstance(value, dict):
            info = value.get("info")
            if isinstance(info, dict):
                name = info.get("name")
                version = info.get("version")
                if isinstance(name, str) and isinstance(version, str):
                    identities.add((name, version))
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)

    visit(metadata)
    if len(identities) != 1:
        raise SystemExit(
            f"Expected one package identity in {source}, found: {sorted(identities)!r}"
        )

    name, version = identities.pop()
    for label, component in (("name", name), ("version", version)):
        if not PACKAGE_COMPONENT_RE.fullmatch(component):
            raise SystemExit(
                f"Unsafe package {label} in {source}: {component!r}"
            )
    return name, version


def read_identity(apk_bin, package):
    result = subprocess.run(
        [
            str(apk_bin),
            "--allow-untrusted",
            "adbdump",
            "--format",
            "json",
            str(package),
        ],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        raise SystemExit(
            f"Cannot read APK metadata from {package}: {result.stderr.strip()}"
        )
    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"Invalid adbdump JSON for {package}: {exc}") from exc
    return package_identity(metadata, package)


def canonicalize(apk_bin, packages):
    mappings = []
    sources = {package.resolve() for package in packages}
    targets = {}

    for package in packages:
        name, version = read_identity(apk_bin, package)
        target = package.with_name(f"{name}-{version}.apk")
        resolved_target = target.resolve()
        previous = targets.get(resolved_target)
        if previous is not None and previous != package:
            raise SystemExit(
                f"Canonical APK filename collision: {previous} and {package} -> {target}"
            )
        if target.exists() and resolved_target not in sources:
            raise SystemExit(
                f"Refusing to overwrite existing canonical APK file: {target}"
            )
        targets[resolved_target] = package
        mappings.append((package, target))

    staged = []
    for source, target in mappings:
        if source == target:
            continue
        temporary = source.with_name(
            f".canonicalize-{uuid.uuid4().hex}-{source.name}"
        )
        source.rename(temporary)
        staged.append((temporary, target, source))

    for temporary, target, source in staged:
        temporary.rename(target)
        print(f"Canonicalized {source.name} -> {target.name}")


def main():
    args = parse_args()
    canonicalize(args.apk_bin, args.packages)


if __name__ == "__main__":
    main()
