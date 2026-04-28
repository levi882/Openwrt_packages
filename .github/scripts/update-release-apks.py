#!/usr/bin/env python3
import hashlib
import io
import json
import os
import re
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PINS = ROOT / ".github" / "release-apk-pins.json"
TOKEN = os.environ.get("GITHUB_TOKEN")


def github_request(url):
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "openwrt-packages-release-updater",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if TOKEN and "api.github.com" in url:
        headers["Authorization"] = f"Bearer {TOKEN}"
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=180) as response:
        return response.read()


def latest_release(repo):
    data = github_request(f"https://api.github.com/repos/{repo}/releases/latest")
    return json.loads(data.decode("utf-8"))


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def download_asset(asset):
    return github_request(asset["browser_download_url"])


def pick_asset(release, pattern, label):
    matches = []
    for asset in release.get("assets", []):
        match = re.fullmatch(pattern, asset["name"])
        if match:
            matches.append((asset, match))

    if not matches:
        names = "\n".join(f"  - {asset['name']}" for asset in release.get("assets", []))
        raise SystemExit(f"Missing {label} asset in {release['html_url']}\nAvailable assets:\n{names}")

    if len(matches) > 1:
        names = ", ".join(asset["name"] for asset, _ in matches)
        raise SystemExit(f"Multiple {label} assets matched: {names}")

    return matches[0]


def pick_apk(apks, pattern, label):
    matches = [(name, digest) for name, digest in apks.items() if re.fullmatch(pattern, name)]
    if not matches:
        names = "\n".join(f"  - {name}" for name in sorted(apks))
        raise SystemExit(f"Missing {label} APK in archive\nAvailable APKs:\n{names}")

    if len(matches) > 1:
        names = ", ".join(name for name, _ in matches)
        raise SystemExit(f"Multiple {label} APKs matched: {names}")

    return matches[0]


def read_pins():
    if not PINS.exists():
        return {}
    return json.loads(PINS.read_text(encoding="utf-8"))


def write_pins(data):
    PINS.write_text(
        json.dumps(data, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )


def get_lucky():
    release = latest_release("levi882/luci-app-lucky")
    main, main_match = pick_asset(
        release,
        r"lucky-(?P<version>.+)_x86_64\.apk",
        "Lucky x86_64 APK",
    )
    luci, luci_match = pick_asset(
        release,
        r"luci-app-lucky-(?P<version>.+)_x86_64\.apk",
        "Lucky LuCI APK",
    )
    i18n, i18n_match = pick_asset(
        release,
        r"luci-i18n-lucky-zh-cn-(?P<version>.+)_x86_64\.apk",
        "Lucky zh-cn APK",
    )
    return {
        "tag": release["tag_name"],
        "version": main_match.group("version"),
        "luci_version": luci_match.group("version"),
        "i18n_version": i18n_match.group("version"),
        "main_sha": sha256(download_asset(main)),
        "luci_sha": sha256(download_asset(luci)),
        "i18n_sha": sha256(download_asset(i18n)),
    }


def get_easytier():
    release = latest_release("EasyTier/luci-app-easytier")
    asset, match = pick_asset(
        release,
        r"EasyTier-v?(?P<version>[^-]+)-x86_64-SNAPSHOT\.zip",
        "EasyTier x86_64 SNAPSHOT zip",
    )
    archive_data = download_asset(asset)
    archive_sha = sha256(archive_data)

    apks = {}
    with zipfile.ZipFile(io.BytesIO(archive_data)) as archive:
        for info in archive.infolist():
            if info.is_dir() or not info.filename.endswith(".apk"):
                continue
            name = Path(info.filename).name
            apks[name] = sha256(archive.read(info))

    main_name, main_sha = pick_apk(apks, r"easytier-.+\.apk", "EasyTier daemon")
    luci_name, luci_sha = pick_apk(apks, r"luci-app-easytier-.+\.apk", "EasyTier LuCI app")
    i18n_name, i18n_sha = pick_apk(
        apks,
        r"luci-i18n-easytier-zh-cn-.+\.apk",
        "EasyTier zh-cn translation",
    )

    return {
        "tag": release["tag_name"],
        "version": match.group("version"),
        "archive_sha": archive_sha,
        "main_name": main_name,
        "main_sha": main_sha,
        "luci_name": luci_name,
        "luci_sha": luci_sha,
        "i18n_name": i18n_name,
        "i18n_sha": i18n_sha,
    }


def get_rtp2httpd():
    release = latest_release("stackia/rtp2httpd")
    main, main_match = pick_asset(
        release,
        r"rtp2httpd-(?P<version>.+)-(?P<release>r\d+)_x86_64\.apk",
        "rtp2httpd x86_64 APK",
    )
    version = main_match.group("version")
    package_release = main_match.group("release")
    luci, _ = pick_asset(
        release,
        rf"luci-app-rtp2httpd-{re.escape(version)}-{re.escape(package_release)}\.apk",
        "rtp2httpd LuCI APK",
    )
    i18n, _ = pick_asset(
        release,
        rf"luci-i18n-rtp2httpd-zh-cn-{re.escape(version)}\.apk",
        "rtp2httpd zh-cn APK",
    )
    return {
        "tag": release["tag_name"],
        "version": version,
        "package_release": package_release,
        "main_sha": sha256(download_asset(main)),
        "luci_sha": sha256(download_asset(luci)),
        "i18n_sha": sha256(download_asset(i18n)),
    }


def get_fakehttp():
    release = latest_release("levi882/FakeHTTP")
    main, main_match = pick_asset(
        release,
        r"fakehttp-openwrt-(?P<openwrt>[^-]+)-x86_64-fakehttp-(?P<version>.+)-(?P<release>r\d+)\.apk",
        "FakeHTTP x86_64 APK",
    )
    openwrt_release = main_match.group("openwrt")
    version = main_match.group("version")
    package_release = main_match.group("release")
    prefix = f"fakehttp-openwrt-{openwrt_release}-x86_64"
    luci, _ = pick_asset(
        release,
        rf"{re.escape(prefix)}-luci-app-fakehttp-{re.escape(version)}-{re.escape(package_release)}\.apk",
        "FakeHTTP LuCI APK",
    )
    i18n, i18n_match = pick_asset(
        release,
        rf"{re.escape(prefix)}-luci-i18n-fakehttp-zh-cn-{re.escape(version)}-(?P<release>r\d+)\.apk",
        "FakeHTTP zh-cn APK",
    )
    return {
        "tag": release["tag_name"],
        "openwrt_release": openwrt_release,
        "version": version,
        "package_release": package_release,
        "i18n_release": i18n_match.group("release"),
        "main_sha": sha256(download_asset(main)),
        "luci_sha": sha256(download_asset(luci)),
        "i18n_sha": sha256(download_asset(i18n)),
    }


def get_smartdns():
    release = latest_release("pymumu/smartdns")
    main, main_match = pick_asset(
        release,
        r"smartdns\.(?P<version>.+)\.x86_64-openwrt-all\.apk",
        "SmartDNS x86_64 APK",
    )
    version = main_match.group("version")
    luci, _ = pick_asset(
        release,
        rf"luci-app-smartdns\.{re.escape(version)}\.all-luci-all\.apk",
        "SmartDNS LuCI APK",
    )
    return {
        "tag": release["tag_name"],
        "version": version,
        "main_sha": sha256(download_asset(main)),
        "luci_sha": sha256(download_asset(luci)),
    }


def get_bandix():
    main_release = latest_release("timsaya/openwrt-bandix")
    main, main_match = pick_asset(
        main_release,
        r"bandix-(?P<version>.+)_x86_64\.apk",
        "Bandix x86_64 APK",
    )

    luci_release = latest_release("timsaya/luci-app-bandix")
    luci, luci_match = pick_asset(
        luci_release,
        r"luci-app-bandix-(?P<version>.+)_all\.apk",
        "Bandix LuCI APK",
    )
    i18n, i18n_match = pick_asset(
        luci_release,
        r"luci-i18n-bandix-zh-cn-(?P<version>.+)_all\.apk",
        "Bandix zh-cn APK",
    )

    return {
        "tag": main_release["tag_name"],
        "version": main_match.group("version"),
        "main_sha": sha256(download_asset(main)),
        "luci_tag": luci_release["tag_name"],
        "luci_version": luci_match.group("version"),
        "luci_sha": sha256(download_asset(luci)),
        "i18n_version": i18n_match.group("version"),
        "i18n_sha": sha256(download_asset(i18n)),
    }


def get_nikki():
    openwrt_release = os.environ.get("NIKKI_OPENWRT_RELEASE")
    if not openwrt_release:
        openwrt_release = read_pins().get("nikki", {}).get("openwrt_release", "25.12")
    release = latest_release("morytyann/OpenWrt-nikki")
    asset, _ = pick_asset(
        release,
        rf"nikki_x86_64-openwrt-{re.escape(openwrt_release)}\.tar\.gz",
        f"Nikki x86_64 OpenWrt {openwrt_release} archive",
    )
    archive_data = download_asset(asset)

    apks = {}
    with tarfile.open(fileobj=io.BytesIO(archive_data), mode="r:gz") as archive:
        for member in archive.getmembers():
            if not member.isfile() or not member.name.endswith(".apk"):
                continue
            file_obj = archive.extractfile(member)
            if file_obj is None:
                continue
            apks[Path(member.name).name] = sha256(file_obj.read())

    if not apks:
        raise SystemExit(f"No APK files found in {asset['name']}")

    excluded_apks = (
        r"luci-i18n-nikki-ru-.+\.apk",
        r"luci-i18n-nikki-zh-tw-.+\.apk",
    )
    apks = {
        name: digest
        for name, digest in apks.items()
        if not any(re.fullmatch(pattern, name) for pattern in excluded_apks)
    }

    return {
        "tag": release["tag_name"],
        "openwrt_release": openwrt_release,
        "archive_sha": sha256(archive_data),
        "apks": dict(sorted(apks.items())),
    }


def main():
    print("Checking latest release APKs...")
    lucky = get_lucky()
    easytier = get_easytier()
    rtp2httpd = get_rtp2httpd()
    fakehttp = get_fakehttp()
    smartdns = get_smartdns()
    bandix = get_bandix()
    nikki = get_nikki()

    write_pins(
        {
            "bandix": bandix,
            "easytier": easytier,
            "fakehttp": fakehttp,
            "lucky": lucky,
            "nikki": nikki,
            "rtp2httpd": rtp2httpd,
            "smartdns": smartdns,
        }
    )

    for name, data in [
        ("Lucky", lucky),
        ("EasyTier", easytier),
        ("rtp2httpd", rtp2httpd),
        ("FakeHTTP", fakehttp),
        ("SmartDNS", smartdns),
        ("Bandix", bandix),
        ("Nikki", nikki),
    ]:
        print(f"{name}: {data['tag']}")


if __name__ == "__main__":
    try:
        main()
    except urllib.error.HTTPError as exc:
        print(f"HTTP error: {exc.code} {exc.reason}", file=sys.stderr)
        print(exc.read().decode("utf-8", "replace"), file=sys.stderr)
        raise
