#!/usr/bin/env python3
import hashlib
import io
import json
import os
import re
import sys
import textwrap
import urllib.request
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "build-feed.yml"
SCRIPT_DIR = ROOT / ".github" / "scripts"
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


def update_env(updates):
    text = WORKFLOW.read_text(encoding="utf-8")
    for name, value in updates.items():
        pattern = rf"^(  {re.escape(name)}: ).*$"
        text, count = re.subn(pattern, rf"\g<1>{value}", text, flags=re.MULTILINE)
        if count != 1:
            raise SystemExit(f"Could not update env value {name} in {WORKFLOW}")
    WORKFLOW.write_text(text, encoding="utf-8", newline="\n")


def write_script(name, body):
    path = SCRIPT_DIR / name
    path.write_text(textwrap.dedent(body).lstrip(), encoding="utf-8", newline="\n")


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
    lite, _ = pick_asset(
        release,
        rf"luci-app-smartdns-lite\.{re.escape(version)}\.all-luci-lite-all\.apk",
        "SmartDNS lite LuCI APK",
    )
    return {
        "tag": release["tag_name"],
        "version": version,
        "main_sha": sha256(download_asset(main)),
        "luci_sha": sha256(download_asset(luci)),
        "lite_sha": sha256(download_asset(lite)),
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


def render_lucky(data):
    write_script(
        "download-lucky-apks.sh",
        f"""
        #!/usr/bin/env bash
        set -euo pipefail

        repo_dir="${{1:?usage: download-lucky-apks.sh <feed-dir>}}"
        tag="${{LUCKY_RELEASE_TAG:-{data['tag']}}}"
        lucky_version="${{LUCKY_VERSION:-{data['version']}}}"
        luci_version="${{LUCI_LUCKY_VERSION:-{data['luci_version']}}}"
        i18n_version="${{LUCI_LUCKY_I18N_VERSION:-{data['i18n_version']}}}"
        base_url="https://github.com/levi882/luci-app-lucky/releases/download/${{tag}}"

        mkdir -p "$repo_dir"

        download_and_check() {{
          local file="$1"
          local sha256="$2"

          echo "Downloading ${{file}}"
          curl -fL --retry 3 --retry-delay 2 -o "${{repo_dir}}/${{file}}" "${{base_url}}/${{file}}"

          (
            cd "$repo_dir"
            printf '%s  %s\\n' "$sha256" "$file" | sha256sum -c -
          )
        }}

        download_and_check \\
          "lucky-${{lucky_version}}_x86_64.apk" \\
          "{data['main_sha']}"

        download_and_check \\
          "luci-app-lucky-${{luci_version}}_x86_64.apk" \\
          "{data['luci_sha']}"

        download_and_check \\
          "luci-i18n-lucky-zh-cn-${{i18n_version}}_x86_64.apk" \\
          "{data['i18n_sha']}"
        """,
    )


def render_easytier(data):
    write_script(
        "download-easytier-apks.sh",
        f"""
        #!/usr/bin/env bash
        set -euo pipefail

        repo_dir="${{1:?usage: download-easytier-apks.sh <feed-dir>}}"
        tag="${{EASYTIER_RELEASE_TAG:-{data['tag']}}}"
        version="${{EASYTIER_VERSION:-{data['version']}}}"
        archive="EasyTier-${{tag}}-x86_64-SNAPSHOT.zip"
        archive_sha256="{data['archive_sha']}"
        url="https://github.com/EasyTier/luci-app-easytier/releases/download/${{tag}}/${{archive}}"
        tmp_dir="$(mktemp -d)"

        command -v unzip >/dev/null 2>&1 || {{
          echo "unzip is required to extract ${{archive}}" >&2
          exit 1
        }}

        cleanup() {{
          rm -rf "$tmp_dir"
        }}
        trap cleanup EXIT

        mkdir -p "$repo_dir"

        echo "Downloading ${{archive}}"
        curl -fL --retry 3 --retry-delay 2 -o "${{tmp_dir}}/${{archive}}" "$url"

        (
          cd "$tmp_dir"
          printf '%s  %s\\n' "$archive_sha256" "$archive" | sha256sum -c -
        )

        unzip -j "${{tmp_dir}}/${{archive}}" '*.apk' -d "${{tmp_dir}}/apks"

        copy_and_check() {{
          local file="$1"
          local sha256="$2"

          [ -s "${{tmp_dir}}/apks/${{file}}" ] || {{
            echo "Expected APK not found in ${{archive}}: ${{file}}" >&2
            exit 1
          }}

          (
            cd "${{tmp_dir}}/apks"
            printf '%s  %s\\n' "$sha256" "$file" | sha256sum -c -
          )

          cp "${{tmp_dir}}/apks/${{file}}" "$repo_dir/"
        }}

        copy_and_check \\
          "{data['main_name']}" \\
          "{data['main_sha']}"

        copy_and_check \\
          "{data['luci_name']}" \\
          "{data['luci_sha']}"

        copy_and_check \\
          "{data['i18n_name']}" \\
          "{data['i18n_sha']}"
        """,
    )


def render_rtp2httpd(data):
    write_script(
        "download-rtp2httpd-apks.sh",
        f"""
        #!/usr/bin/env bash
        set -euo pipefail

        repo_dir="${{1:?usage: download-rtp2httpd-apks.sh <feed-dir>}}"
        tag="${{RTP2HTTPD_RELEASE_TAG:-{data['tag']}}}"
        version="${{RTP2HTTPD_VERSION:-{data['version']}}}"
        release="${{RTP2HTTPD_PACKAGE_RELEASE:-{data['package_release']}}}"
        base_url="https://github.com/stackia/rtp2httpd/releases/download/${{tag}}"

        mkdir -p "$repo_dir"

        download_and_check() {{
          local file="$1"
          local sha256="$2"

          echo "Downloading ${{file}}"
          curl -fL --retry 3 --retry-delay 2 -o "${{repo_dir}}/${{file}}" "${{base_url}}/${{file}}"

          (
            cd "$repo_dir"
            printf '%s  %s\\n' "$sha256" "$file" | sha256sum -c -
          )
        }}

        download_and_check \\
          "rtp2httpd-${{version}}-${{release}}_x86_64.apk" \\
          "{data['main_sha']}"

        download_and_check \\
          "luci-app-rtp2httpd-${{version}}-${{release}}.apk" \\
          "{data['luci_sha']}"

        download_and_check \\
          "luci-i18n-rtp2httpd-zh-cn-${{version}}.apk" \\
          "{data['i18n_sha']}"
        """,
    )


def render_fakehttp(data):
    write_script(
        "download-fakehttp-apks.sh",
        f"""
        #!/usr/bin/env bash
        set -euo pipefail

        repo_dir="${{1:?usage: download-fakehttp-apks.sh <feed-dir>}}"
        tag="${{FAKEHTTP_RELEASE_TAG:-{data['tag']}}}"
        openwrt_release="${{FAKEHTTP_OPENWRT_RELEASE:-{data['openwrt_release']}}}"
        version="${{FAKEHTTP_VERSION:-{data['version']}}}"
        package_release="${{FAKEHTTP_PACKAGE_RELEASE:-{data['package_release']}}}"
        i18n_release="${{FAKEHTTP_I18N_RELEASE:-{data['i18n_release']}}}"
        prefix="fakehttp-openwrt-${{openwrt_release}}-x86_64"
        base_url="https://github.com/levi882/FakeHTTP/releases/download/${{tag}}"

        mkdir -p "$repo_dir"

        download_and_check() {{
          local file="$1"
          local sha256="$2"

          echo "Downloading ${{file}}"
          curl -fL --retry 3 --retry-delay 2 -o "${{repo_dir}}/${{file}}" "${{base_url}}/${{file}}"

          (
            cd "$repo_dir"
            printf '%s  %s\\n' "$sha256" "$file" | sha256sum -c -
          )
        }}

        download_and_check \\
          "${{prefix}}-fakehttp-${{version}}-${{package_release}}.apk" \\
          "{data['main_sha']}"

        download_and_check \\
          "${{prefix}}-luci-app-fakehttp-${{version}}-${{package_release}}.apk" \\
          "{data['luci_sha']}"

        download_and_check \\
          "${{prefix}}-luci-i18n-fakehttp-zh-cn-${{version}}-${{i18n_release}}.apk" \\
          "{data['i18n_sha']}"
        """,
    )


def render_smartdns(data):
    write_script(
        "download-smartdns-apks.sh",
        f"""
        #!/usr/bin/env bash
        set -euo pipefail

        repo_dir="${{1:?usage: download-smartdns-apks.sh <feed-dir>}}"
        tag="${{SMARTDNS_RELEASE_TAG:-{data['tag']}}}"
        version="${{SMARTDNS_VERSION:-{data['version']}}}"

        mkdir -p "$repo_dir"

        download_and_check() {{
          local file="$1"
          local sha256="$2"
          local url="https://github.com/pymumu/smartdns/releases/download/${{tag}}/${{file}}"

          echo "Downloading ${{file}}"
          curl -fL --retry 3 --retry-delay 2 -o "${{repo_dir}}/${{file}}" "$url"

          (
            cd "$repo_dir"
            printf '%s  %s\\n' "$sha256" "$file" | sha256sum -c -
          )
        }}

        download_and_check \\
          "smartdns.${{version}}.x86_64-openwrt-all.apk" \\
          "{data['main_sha']}"

        download_and_check \\
          "luci-app-smartdns.${{version}}.all-luci-all.apk" \\
          "{data['luci_sha']}"

        download_and_check \\
          "luci-app-smartdns-lite.${{version}}.all-luci-lite-all.apk" \\
          "{data['lite_sha']}"
        """,
    )


def render_bandix(data):
    write_script(
        "download-bandix-apks.sh",
        f"""
        #!/usr/bin/env bash
        set -euo pipefail

        repo_dir="${{1:?usage: download-bandix-apks.sh <feed-dir>}}"
        bandix_tag="${{BANDIX_RELEASE_TAG:-{data['tag']}}}"
        bandix_version="${{BANDIX_VERSION:-{data['version']}}}"
        luci_tag="${{LUCI_BANDIX_RELEASE_TAG:-{data['luci_tag']}}}"
        luci_version="${{LUCI_BANDIX_VERSION:-{data['luci_version']}}}"
        i18n_version="${{LUCI_BANDIX_I18N_VERSION:-{data['i18n_version']}}}"

        mkdir -p "$repo_dir"

        download_and_check() {{
          local url="$1"
          local file="$2"
          local sha256="$3"

          echo "Downloading ${{file}}"
          curl -fL --retry 3 --retry-delay 2 -o "${{repo_dir}}/${{file}}" "$url"

          (
            cd "$repo_dir"
            printf '%s  %s\\n' "$sha256" "$file" | sha256sum -c -
          )
        }}

        download_and_check \\
          "https://github.com/timsaya/openwrt-bandix/releases/download/${{bandix_tag}}/bandix-${{bandix_version}}_x86_64.apk" \\
          "bandix-${{bandix_version}}_x86_64.apk" \\
          "{data['main_sha']}"

        download_and_check \\
          "https://github.com/timsaya/luci-app-bandix/releases/download/${{luci_tag}}/luci-app-bandix-${{luci_version}}_all.apk" \\
          "luci-app-bandix-${{luci_version}}_all.apk" \\
          "{data['luci_sha']}"

        download_and_check \\
          "https://github.com/timsaya/luci-app-bandix/releases/download/${{luci_tag}}/luci-i18n-bandix-zh-cn-${{i18n_version}}_all.apk" \\
          "luci-i18n-bandix-zh-cn-${{i18n_version}}_all.apk" \\
          "{data['i18n_sha']}"
        """,
    )


def main():
    print("Checking latest release APKs...")
    lucky = get_lucky()
    easytier = get_easytier()
    rtp2httpd = get_rtp2httpd()
    fakehttp = get_fakehttp()
    smartdns = get_smartdns()
    bandix = get_bandix()

    update_env(
        {
            "LUCKY_RELEASE_TAG": lucky["tag"],
            "LUCKY_VERSION": lucky["version"],
            "LUCI_LUCKY_VERSION": lucky["luci_version"],
            "LUCI_LUCKY_I18N_VERSION": lucky["i18n_version"],
            "EASYTIER_RELEASE_TAG": easytier["tag"],
            "EASYTIER_VERSION": easytier["version"],
            "RTP2HTTPD_RELEASE_TAG": rtp2httpd["tag"],
            "RTP2HTTPD_VERSION": rtp2httpd["version"],
            "RTP2HTTPD_PACKAGE_RELEASE": rtp2httpd["package_release"],
            "FAKEHTTP_RELEASE_TAG": fakehttp["tag"],
            "FAKEHTTP_OPENWRT_RELEASE": fakehttp["openwrt_release"],
            "FAKEHTTP_VERSION": fakehttp["version"],
            "FAKEHTTP_PACKAGE_RELEASE": fakehttp["package_release"],
            "FAKEHTTP_I18N_RELEASE": fakehttp["i18n_release"],
            "SMARTDNS_RELEASE_TAG": smartdns["tag"],
            "SMARTDNS_VERSION": smartdns["version"],
            "BANDIX_RELEASE_TAG": bandix["tag"],
            "BANDIX_VERSION": bandix["version"],
            "LUCI_BANDIX_RELEASE_TAG": bandix["luci_tag"],
            "LUCI_BANDIX_VERSION": bandix["luci_version"],
            "LUCI_BANDIX_I18N_VERSION": bandix["i18n_version"],
        }
    )

    render_lucky(lucky)
    render_easytier(easytier)
    render_rtp2httpd(rtp2httpd)
    render_fakehttp(fakehttp)
    render_smartdns(smartdns)
    render_bandix(bandix)

    for name, data in [
        ("Lucky", lucky),
        ("EasyTier", easytier),
        ("rtp2httpd", rtp2httpd),
        ("FakeHTTP", fakehttp),
        ("SmartDNS", smartdns),
        ("Bandix", bandix),
    ]:
        print(f"{name}: {data['tag']}")


if __name__ == "__main__":
    try:
        main()
    except urllib.error.HTTPError as exc:
        print(f"HTTP error: {exc.code} {exc.reason}", file=sys.stderr)
        print(exc.read().decode("utf-8", "replace"), file=sys.stderr)
        raise
