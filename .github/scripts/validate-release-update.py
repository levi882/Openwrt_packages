#!/usr/bin/env python3
import argparse
import json
from pathlib import Path


def parse_args():
    parser = argparse.ArgumentParser(
        description="Reject unexpected package removals in a generated release manifest."
    )
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument(
        "--allow-added-package",
        action="append",
        default=[],
        metavar="PACKAGE_ID",
        help="allow this package group to appear for the first time; may be repeated",
    )
    return parser.parse_args()


def load_packages(path):
    with path.open(encoding="utf-8") as file_obj:
        manifest = json.load(file_obj)

    if manifest.get("schema_version") != 1:
        raise SystemExit(f"Unsupported manifest schema in {path}")

    packages = manifest.get("packages")
    if not isinstance(packages, list):
        raise SystemExit(f"Manifest packages must be a list in {path}")

    result = {}
    for package in packages:
        package_id = package.get("id")
        if not isinstance(package_id, str) or not package_id:
            raise SystemExit(f"Invalid package id in {path}: {package_id!r}")
        if package_id in result:
            raise SystemExit(f"Duplicate package id in {path}: {package_id}")
        result[package_id] = package
    return result


def package_outputs(package):
    outputs = []
    for artifact in package.get("artifacts", []):
        if artifact.get("type") == "file":
            outputs.append(artifact.get("output"))
        elif artifact.get("type") == "archive":
            outputs.extend(file_entry.get("output") for file_entry in artifact.get("files", []))
    return outputs


def repositories(package):
    return {
        release.get("repository")
        for release in package.get("releases", [])
        if release.get("repository")
    }


def tags(package):
    return ", ".join(
        f"{release.get('repository')}@{release.get('tag')}"
        for release in package.get("releases", [])
    )


def main():
    args = parse_args()
    before = load_packages(args.before)
    after = load_packages(args.after)

    before_ids = set(before)
    after_ids = set(after)
    removed = sorted(before_ids - after_ids)
    added = sorted(after_ids - before_ids)
    unexpected_added = sorted(set(added) - set(args.allow_added_package))
    if removed or unexpected_added:
        details = []
        if removed:
            details.append(f"removed package groups: {', '.join(removed)}")
        if unexpected_added:
            details.append(f"unexpected package groups: {', '.join(unexpected_added)}")
        raise SystemExit("Generated manifest changed package groups; " + "; ".join(details))

    changed = len(added)
    for package_id in added:
        print(f"{package_id}: added {tags(after[package_id])}")

    for package_id in sorted(before):
        old_package = before[package_id]
        new_package = after[package_id]
        old_outputs = package_outputs(old_package)
        new_outputs = package_outputs(new_package)

        if len(new_outputs) < len(old_outputs):
            raise SystemExit(
                f"{package_id} lost APKs: {len(old_outputs)} -> {len(new_outputs)}"
            )
        if repositories(old_package) != repositories(new_package):
            raise SystemExit(f"{package_id} changed its upstream repositories")

        if old_package != new_package:
            changed += 1
            print(f"{package_id}: {tags(old_package)} -> {tags(new_package)}")

    if not changed:
        raise SystemExit("Manifest file changed, but no package metadata changed")

    print(f"Validated {changed} changed package group(s); no groups or APKs were removed")


if __name__ == "__main__":
    main()
