#!/usr/bin/env python3
import hashlib
import io
import json
import os
import re
import sys
import tarfile
import urllib.error
import urllib.request
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / ".github" / "release-apks.json"
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


def releases_list(repo):
    data = github_request(f"https://api.github.com/repos/{repo}/releases?per_page=100")
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


def read_manifest():
    if not MANIFEST.exists():
        return {}
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def package_metadata(package_id):
    for package in read_manifest().get("packages", []):
        if package.get("id") == package_id:
            return package.get("metadata", {})
    return {}


def write_manifest(data):
    MANIFEST.write_text(
        json.dumps(data, indent=2) + "\n",
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

    main_name, main_sha = pick_apk(apks, r"easytier-(?!noweb).+\.apk", "EasyTier daemon")
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


def get_iptv():
    try:
        release = latest_release("levi882/iptv")
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            print("IPTV: no published release yet; keeping it out of the manifest")
            return None
        raise

    main, main_match = pick_asset(
        release,
        r"iptv-refresh-(?P<version>.+)-(?P<release>r\d+)\.apk",
        "IPTV Refresh x86_64 APK",
    )
    version = main_match.group("version")
    package_release = main_match.group("release")
    luci, _ = pick_asset(
        release,
        rf"luci-app-iptv-refresh-{re.escape(version)}-{re.escape(package_release)}\.apk",
        "IPTV Refresh LuCI APK",
    )
    i18n, _ = pick_asset(
        release,
        rf"luci-i18n-iptv-refresh-zh-cn-{re.escape(version)}-{re.escape(package_release)}\.apk",
        "IPTV Refresh zh-cn APK",
    )
    return {
        "tag": release["tag_name"],
        "main_name": main["name"],
        "main_sha": sha256(download_asset(main)),
        "luci_name": luci["name"],
        "luci_sha": sha256(download_asset(luci)),
        "i18n_name": i18n["name"],
        "i18n_sha": sha256(download_asset(i18n)),
    }


def select_smartdns_assets(releases):
    luci_by_version = {}
    for release in releases:
        for asset in release.get("assets", []):
            match = re.fullmatch(
                r"luci-app-smartdns\.(?P<version>.+)\.luci-all\.apk",
                asset["name"],
            )
            if match and match.group("version") not in luci_by_version:
                luci_by_version[match.group("version")] = (release, asset)

    # GitHub returns releases newest first. SmartDNS publishes the daemon with
    # its loadable WebUI plugin in a separate *_with_ui release, while the
    # matching LuCI package is in the ordinary release for the same version.
    for main_release in releases:
        for main in main_release.get("assets", []):
            # Deliberately exclude x86_64-openwrt.apk: that artifact is a
            # static executable and cannot load smartdns_ui.so when a restored
            # configuration has the built-in WebUI enabled.
            match = re.fullmatch(
                r"smartdns\.(?P<version>.+)\.x86_64\.apk",
                main["name"],
            )
            if not match:
                continue

            version = match.group("version")
            luci_match = luci_by_version.get(version)
            if luci_match is None:
                continue
            luci_release, luci = luci_match
            return main_release, main, luci_release, luci, version

    return None


def get_smartdns():
    repo = "PikuZheng/smartdns"
    selected = select_smartdns_assets(releases_list(repo))
    if selected is None:
        raise SystemExit(
            f"No {repo} releases found with matching WebUI daemon and LuCI APK assets"
        )

    main_release, main, luci_release, luci, version = selected
    return {
        "main_tag": main_release["tag_name"],
        "luci_tag": luci_release["tag_name"],
        "version": version,
        "main_name": main["name"],
        "luci_name": luci["name"],
        "main_sha": sha256(download_asset(main)),
        "luci_sha": sha256(download_asset(luci)),
    }


def get_temp_status():
    release = latest_release("levi882/luci-app-temp-status")
    main, _ = pick_asset(
        release,
        r"luci-app-temp-status-.+\.apk",
        "temp-status LuCI APK",
    )
    i18n, _ = pick_asset(
        release,
        r"luci-i18n-temp-status-zh-cn-.+\.apk",
        "temp-status zh-cn APK",
    )
    return {
        "tag": release["tag_name"],
        "main_name": main["name"],
        "main_sha": sha256(download_asset(main)),
        "i18n_name": i18n["name"],
        "i18n_sha": sha256(download_asset(i18n)),
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
        openwrt_release = package_metadata("nikki").get("openwrt_release", "25.12")
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


def get_aurora():
    theme_release = latest_release("eamonxg/luci-theme-aurora")
    theme, theme_match = pick_asset(
        theme_release,
        r"luci-theme-aurora-(?P<version>.+)\.apk",
        "Aurora theme APK",
    )

    config_release = latest_release("eamonxg/luci-app-aurora-config")
    config, config_match = pick_asset(
        config_release,
        r"luci-app-aurora-config-(?P<version>.+)\.apk",
        "Aurora config LuCI APK",
    )
    i18n, i18n_match = pick_asset(
        config_release,
        r"luci-i18n-aurora-config-zh-cn-(?P<version>.+)\.apk",
        "Aurora config zh-cn APK",
    )

    return {
        "theme_tag": theme_release["tag_name"],
        "theme_version": theme_match.group("version"),
        "theme_sha": sha256(download_asset(theme)),
        "config_tag": config_release["tag_name"],
        "config_version": config_match.group("version"),
        "config_sha": sha256(download_asset(config)),
        "i18n_version": i18n_match.group("version"),
        "i18n_sha": sha256(download_asset(i18n)),
    }


def release_url(repo, tag, filename):
    return f"https://github.com/{repo}/releases/download/{tag}/{filename}"


def file_artifact(repo, tag, source, digest, output=None):
    return {
        "type": "file",
        "url": release_url(repo, tag, source),
        "sha256": digest,
        "output": output or source,
    }


def archive_file(source, digest, output=None):
    return {
        "source": source,
        "sha256": digest,
        "output": output or source,
    }


def release_ref(repository, tag):
    return {"repository": repository, "tag": tag}


def i18n_apk_version(version):
    head, separator, tail = version.rpartition(".")
    return f"{head}~{tail}" if separator else version


def build_manifest(
    aurora,
    bandix,
    easytier,
    fakehttp,
    iptv,
    lucky,
    nikki,
    rtp2httpd,
    smartdns,
    temp_status,
):
    packages = []

    aurora_theme_repo = "eamonxg/luci-theme-aurora"
    aurora_config_repo = "eamonxg/luci-app-aurora-config"
    packages.append(
        {
            "id": "aurora",
            "releases": [
                release_ref(aurora_theme_repo, aurora["theme_tag"]),
                release_ref(aurora_config_repo, aurora["config_tag"]),
            ],
            "artifacts": [
                file_artifact(
                    aurora_theme_repo,
                    aurora["theme_tag"],
                    f"luci-theme-aurora-{aurora['theme_version']}.apk",
                    aurora["theme_sha"],
                ),
                file_artifact(
                    aurora_config_repo,
                    aurora["config_tag"],
                    f"luci-app-aurora-config-{aurora['config_version']}.apk",
                    aurora["config_sha"],
                ),
                file_artifact(
                    aurora_config_repo,
                    aurora["config_tag"],
                    f"luci-i18n-aurora-config-zh-cn-{aurora['i18n_version']}.apk",
                    aurora["i18n_sha"],
                    f"luci-i18n-aurora-config-zh-cn-{i18n_apk_version(aurora['i18n_version'])}.apk",
                ),
            ],
        }
    )

    bandix_repo = "timsaya/openwrt-bandix"
    bandix_luci_repo = "timsaya/luci-app-bandix"
    packages.append(
        {
            "id": "bandix",
            "releases": [
                release_ref(bandix_repo, bandix["tag"]),
                release_ref(bandix_luci_repo, bandix["luci_tag"]),
            ],
            "artifacts": [
                file_artifact(
                    bandix_repo,
                    bandix["tag"],
                    f"bandix-{bandix['version']}_x86_64.apk",
                    bandix["main_sha"],
                    f"bandix-{bandix['version']}.apk",
                ),
                file_artifact(
                    bandix_luci_repo,
                    bandix["luci_tag"],
                    f"luci-app-bandix-{bandix['luci_version']}_all.apk",
                    bandix["luci_sha"],
                    f"luci-app-bandix-{bandix['luci_version']}.apk",
                ),
                file_artifact(
                    bandix_luci_repo,
                    bandix["luci_tag"],
                    f"luci-i18n-bandix-zh-cn-{bandix['i18n_version']}_all.apk",
                    bandix["i18n_sha"],
                    f"luci-i18n-bandix-zh-cn-{i18n_apk_version(bandix['i18n_version'])}.apk",
                ),
            ],
        }
    )

    easytier_repo = "EasyTier/luci-app-easytier"
    easytier_archive = f"EasyTier-{easytier['tag']}-x86_64-SNAPSHOT.zip"
    packages.append(
        {
            "id": "easytier",
            "releases": [release_ref(easytier_repo, easytier["tag"])],
            "artifacts": [
                {
                    "type": "archive",
                    "format": "zip",
                    "url": release_url(easytier_repo, easytier["tag"], easytier_archive),
                    "sha256": easytier["archive_sha"],
                    "files": [
                        archive_file(easytier["main_name"], easytier["main_sha"]),
                        archive_file(easytier["luci_name"], easytier["luci_sha"]),
                        archive_file(easytier["i18n_name"], easytier["i18n_sha"]),
                    ],
                }
            ],
        }
    )

    fakehttp_repo = "levi882/FakeHTTP"
    fakehttp_prefix = f"fakehttp-openwrt-{fakehttp['openwrt_release']}-x86_64"
    packages.append(
        {
            "id": "fakehttp",
            "releases": [release_ref(fakehttp_repo, fakehttp["tag"])],
            "metadata": {"openwrt_release": fakehttp["openwrt_release"]},
            "artifacts": [
                file_artifact(
                    fakehttp_repo,
                    fakehttp["tag"],
                    f"{fakehttp_prefix}-fakehttp-{fakehttp['version']}-{fakehttp['package_release']}.apk",
                    fakehttp["main_sha"],
                    f"fakehttp-{fakehttp['version']}-{fakehttp['package_release']}.apk",
                ),
                file_artifact(
                    fakehttp_repo,
                    fakehttp["tag"],
                    f"{fakehttp_prefix}-luci-app-fakehttp-{fakehttp['version']}-{fakehttp['package_release']}.apk",
                    fakehttp["luci_sha"],
                    f"luci-app-fakehttp-{fakehttp['version']}-{fakehttp['package_release']}.apk",
                ),
                file_artifact(
                    fakehttp_repo,
                    fakehttp["tag"],
                    f"{fakehttp_prefix}-luci-i18n-fakehttp-zh-cn-{fakehttp['version']}-{fakehttp['i18n_release']}.apk",
                    fakehttp["i18n_sha"],
                    f"luci-i18n-fakehttp-zh-cn-{fakehttp['version']}-{fakehttp['i18n_release']}.apk",
                ),
            ],
        }
    )

    if iptv is not None:
        iptv_repo = "levi882/iptv"
        packages.append(
            {
                "id": "iptv",
                "releases": [release_ref(iptv_repo, iptv["tag"])],
                "artifacts": [
                    file_artifact(
                        iptv_repo,
                        iptv["tag"],
                        iptv["main_name"],
                        iptv["main_sha"],
                    ),
                    file_artifact(
                        iptv_repo,
                        iptv["tag"],
                        iptv["luci_name"],
                        iptv["luci_sha"],
                    ),
                    file_artifact(
                        iptv_repo,
                        iptv["tag"],
                        iptv["i18n_name"],
                        iptv["i18n_sha"],
                    ),
                ],
            }
        )

    lucky_repo = "levi882/luci-app-lucky"
    packages.append(
        {
            "id": "lucky",
            "releases": [release_ref(lucky_repo, lucky["tag"])],
            "artifacts": [
                file_artifact(
                    lucky_repo,
                    lucky["tag"],
                    f"lucky-{lucky['version']}_x86_64.apk",
                    lucky["main_sha"],
                    f"lucky-{lucky['version']}.apk",
                ),
                file_artifact(
                    lucky_repo,
                    lucky["tag"],
                    f"luci-app-lucky-{lucky['luci_version']}_x86_64.apk",
                    lucky["luci_sha"],
                    f"luci-app-lucky-{lucky['luci_version']}.apk",
                ),
                file_artifact(
                    lucky_repo,
                    lucky["tag"],
                    f"luci-i18n-lucky-zh-cn-{lucky['i18n_version']}_x86_64.apk",
                    lucky["i18n_sha"],
                    f"luci-i18n-lucky-zh-cn-{lucky['i18n_version']}.apk",
                ),
            ],
        }
    )

    nikki_repo = "morytyann/OpenWrt-nikki"
    nikki_archive = f"nikki_x86_64-openwrt-{nikki['openwrt_release']}.tar.gz"
    packages.append(
        {
            "id": "nikki",
            "releases": [release_ref(nikki_repo, nikki["tag"])],
            "metadata": {"openwrt_release": nikki["openwrt_release"]},
            "artifacts": [
                {
                    "type": "archive",
                    "format": "tar.gz",
                    "url": release_url(nikki_repo, nikki["tag"], nikki_archive),
                    "sha256": nikki["archive_sha"],
                    "files": [
                        archive_file(name, digest)
                        for name, digest in sorted(nikki["apks"].items())
                    ],
                }
            ],
        }
    )

    rtp2httpd_repo = "stackia/rtp2httpd"
    rtp_version = f"{rtp2httpd['version']}-{rtp2httpd['package_release']}"
    packages.append(
        {
            "id": "rtp2httpd",
            "releases": [release_ref(rtp2httpd_repo, rtp2httpd["tag"])],
            "artifacts": [
                file_artifact(
                    rtp2httpd_repo,
                    rtp2httpd["tag"],
                    f"rtp2httpd-{rtp_version}_x86_64.apk",
                    rtp2httpd["main_sha"],
                    f"rtp2httpd-{rtp_version}.apk",
                ),
                file_artifact(
                    rtp2httpd_repo,
                    rtp2httpd["tag"],
                    f"luci-app-rtp2httpd-{rtp_version}.apk",
                    rtp2httpd["luci_sha"],
                ),
                file_artifact(
                    rtp2httpd_repo,
                    rtp2httpd["tag"],
                    f"luci-i18n-rtp2httpd-zh-cn-{rtp2httpd['version']}.apk",
                    rtp2httpd["i18n_sha"],
                ),
            ],
        }
    )

    smartdns_repo = "PikuZheng/smartdns"
    smartdns_apk_version = smartdns["version"]
    packages.append(
        {
            "id": "smartdns",
            "releases": [
                release_ref(smartdns_repo, smartdns["main_tag"]),
                release_ref(smartdns_repo, smartdns["luci_tag"]),
            ],
            "artifacts": [
                file_artifact(
                    smartdns_repo,
                    smartdns["main_tag"],
                    smartdns["main_name"],
                    smartdns["main_sha"],
                    f"smartdns-{smartdns_apk_version}.apk",
                ),
                file_artifact(
                    smartdns_repo,
                    smartdns["luci_tag"],
                    smartdns["luci_name"],
                    smartdns["luci_sha"],
                    f"luci-app-smartdns-{smartdns_apk_version}.apk",
                ),
            ],
        }
    )

    temp_status_repo = "levi882/luci-app-temp-status"
    packages.append(
        {
            "id": "temp-status",
            "releases": [release_ref(temp_status_repo, temp_status["tag"])],
            "artifacts": [
                file_artifact(
                    temp_status_repo,
                    temp_status["tag"],
                    temp_status["main_name"],
                    temp_status["main_sha"],
                ),
                file_artifact(
                    temp_status_repo,
                    temp_status["tag"],
                    temp_status["i18n_name"],
                    temp_status["i18n_sha"],
                ),
            ],
        }
    )

    return {"schema_version": 1, "packages": packages}


def main():
    print("Checking latest release APKs...")
    aurora = get_aurora()
    lucky = get_lucky()
    easytier = get_easytier()
    rtp2httpd = get_rtp2httpd()
    fakehttp = get_fakehttp()
    iptv = get_iptv()
    smartdns = get_smartdns()
    temp_status = get_temp_status()
    bandix = get_bandix()
    nikki = get_nikki()

    write_manifest(
        build_manifest(
            aurora,
            bandix,
            easytier,
            fakehttp,
            iptv,
            lucky,
            nikki,
            rtp2httpd,
            smartdns,
            temp_status,
        )
    )

    for name, data in [
        ("Aurora", aurora),
        ("Lucky", lucky),
        ("EasyTier", easytier),
        ("rtp2httpd", rtp2httpd),
        ("FakeHTTP", fakehttp),
        ("IPTV", iptv),
        ("SmartDNS", smartdns),
        ("temp-status", temp_status),
        ("Bandix", bandix),
        ("Nikki", nikki),
    ]:
        if data is None:
            continue
        if name == "Aurora":
            print(f"{name}: theme {data['theme_tag']}, config {data['config_tag']}")
        elif name == "SmartDNS":
            print(
                f"{name}: daemon {data['main_tag']}, LuCI {data['luci_tag']}"
            )
        else:
            print(f"{name}: {data['tag']}")


if __name__ == "__main__":
    try:
        main()
    except urllib.error.HTTPError as exc:
        print(f"HTTP error: {exc.code} {exc.reason}", file=sys.stderr)
        print(exc.read().decode("utf-8", "replace"), file=sys.stderr)
        raise
