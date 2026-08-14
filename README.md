# OpenWrt Personal APK Feed

Personal OpenWrt 25.12 `x86_64` APK feed and router restore helper.

The feed is published at:

```text
https://openwrt-packages.pages.dev/openwrt-25.12/x86_64/myfeed/packages.adb
```

It currently carries personal-use packages such as Aurora theme, Bandix,
EasyTier, Homebox, Lucky, Nikki, rtp2httpd, SmartDNS, and temp-status plus their
LuCI packages where available. IPTV Refresh joins automatically after its first
tagged Release is published.

## Build

Generate the APK signing key once:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\generate-apk-key.ps1
```

Add these GitHub secrets:

```text
CLOUDFLARE_ACCOUNT_ID
CLOUDFLARE_API_TOKEN
PRIVATE_KEY
AUTOMATION_TOKEN
```

`AUTOMATION_TOKEN` is a fine-grained GitHub token limited to this repository
with read/write access to Contents and Pull requests. It allows the scheduled
release updater to open and automatically merge a verified PR while still
triggering the normal `build-feed` workflow after the merge.

Push to `main`, or run the `build-feed` workflow manually. The
`update-release-apks` workflow can be run manually to refresh the centralized
`.github/release-apks.json` manifest. The manifest contains the resolved release
URLs, output filenames, archive members, and SHA256 checksums consumed by the
generic downloader.

## Router Feed Setup

```sh
MYFEED_BASE=https://openwrt-packages.pages.dev
wget -O /etc/apk/keys/myfeed.pem "$MYFEED_BASE/public-key.pem"
echo "$MYFEED_BASE/openwrt-25.12/x86_64/myfeed/packages.adb" > /etc/apk/repositories.d/00-myfeed.list
apk update
```

## Restore After Upgrade

Run the restore helper on the router with your overlay backup:

```sh
./router/restore_overlay.sh overlay_backup.tar.gz
```

In broad strokes it:

- restores the overlay backup
- keeps the current firmware's APK feed state
- removes stale kernel/package-manager/LuCI runtime files from the backup
- reinstalls selected packages on first boot
- preserves the current extroot entry by default
- reinstalls and configures the packaged IPTV Refresh service when available
- leaves IPTV stopped and records a warning when the APK is unavailable
