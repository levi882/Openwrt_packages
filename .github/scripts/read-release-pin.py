#!/usr/bin/env python3
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PINS = ROOT / ".github" / "release-apk-pins.json"


def resolve(data, dotted_key):
    value = data
    for part in dotted_key.split("."):
        value = value[part]
    return value


def main():
    if len(sys.argv) not in (2, 3):
        raise SystemExit("usage: read-release-pin.py [--pairs] <dotted-key>")

    with PINS.open(encoding="utf-8") as file_obj:
        data = json.load(file_obj)

    if len(sys.argv) == 3:
        if sys.argv[1] != "--pairs":
            raise SystemExit("usage: read-release-pin.py [--pairs] <dotted-key>")
        for name, digest in resolve(data, sys.argv[2]).items():
            print(f"{name}\t{digest}")
        return

    value = resolve(data, sys.argv[1])
    if isinstance(value, (dict, list)):
        print(json.dumps(value, sort_keys=True))
        return
    print(value)


if __name__ == "__main__":
    main()
