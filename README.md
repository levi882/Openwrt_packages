# OpenWrt Personal APK Feed

Personal OpenWrt 25.12 `x86_64` APK feed and LuCI backup migration application.

The feed is published at:

```text
https://openwrt-packages.pages.dev/openwrt-25.12/x86_64/myfeed/packages.adb
```

It currently carries personal-use packages such as Aurora theme, Bandix,
EasyTier, Homebox, Lucky, Nikki, rtp2httpd, SmartDNS, and temp-status plus their
LuCI packages where available. ESA certificate synchronization is also included
with its LuCI page. IPTV Refresh joins automatically after its first tagged
Release is published.

ESA packages are published in this repository's `esa-cert-sync-v*` Releases.
The release updater selects the newest published matching release and verifies
both APK checksums. After configuring this feed, install them with:

```sh
apk update
apk add luci-app-esa-cert-sync
```

The backend is installed as a dependency. Configure credentials and certificate
targets under **Services → ESA 证书同步** before enabling synchronization.

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

The build also compiles the local `overlay-restore`, `luci-app-overlay-restore`,
`docker-guard` and `luci-app-docker-exposure` packages in `packages/` before
signing the feed index.
The backend uses pinned stable Rust 1.99.0 and produces a static x86_64 musl
executable; the LuCI frontend uses JavaScript. Python is not required on the router.

## Router Feed Setup

```sh
MYFEED_BASE=https://openwrt-packages.pages.dev
wget -O /etc/apk/keys/myfeed.pem "$MYFEED_BASE/public-key.pem"
echo "$MYFEED_BASE/openwrt-25.12/x86_64/myfeed/packages.adb" > /etc/apk/repositories.d/00-myfeed.list
apk update
```

## Restore After Upgrade

After publishing the new packages to the feed, install the application:

```sh
apk update
apk add luci-app-overlay-restore
```

Open **System → 备份迁移恢复** to edit the recovery profile, upload an overlay or
sysupgrade backup or select one already on the router, inspect the file/package
plan, and explicitly confirm it. Router file and directory browsing uses the
embedded QuickFile page. If QuickFile is already available on the router, use it
to manage backups or choose the IPTV and Home Assistant directories without
leaving the restore page. Paths can also be entered directly.
Click a backup file in QuickFile's list or grid to fill its path automatically.
Use **Save** at the bottom of the page to persist settings separately before
inspecting a backup from the CLI. Saving settings does not start recovery.
The same backend is available from the router CLI:

```sh
./router/restore_overlay.sh --inspect overlay_backup.tar.gz
./router/restore_overlay.sh overlay_backup.tar.gz
```

The helper now requires the installed `overlay-restore` package. It migrates
configuration and custom regular files through the mounted root filesystem;
by default, files absent from the backup remain on the current system.
Optional clean recovery prepares a fresh internal or external ext4/f2fs overlay,
can select and re-enable an external extroot after an upgrade, switches from RAM,
and retains the old environment for rollback. Other disk
directories are kept. In the default mode, current kernel files, package database, feeds,
keys, LuCI runtime, and recovery tools are preserved. Selected packages are
installed after reboot by a persistent `procd` worker, with a retry action for
failed package/service work.

Local development APKs have not been published by merely building
them; publishing still follows the normal feed workflow.

## Docker Port Access Control

After publishing the local packages through the feed workflow:

```sh
apk update
apk add luci-app-docker-exposure
```

The `docker-guard` backend is installed as a dependency. Open
**Services → Docker 端口访问控制** to choose **阻止 WAN / 允许 WAN** for a
configured container, inspect its host-to-container port mappings and actual
rule coverage, then apply pending changes. Host-network containers and
containers without published ports are listed separately. The page refreshes
read-only status every 15 seconds.

The backend preserves `/etc/config/docker_exposure` on upgrades. Installation
and service startup register currently running containers that are not yet
configured, using `settings.default_mode` (`lan` by default), and install one
minute-based check job without removing other cron tasks. Existing per-container
choices are retained. Generated port rules are not embedded in the package;
the last generated fw4 include is retained for sysupgrade. The package does not
reload fw4 or restart Docker during installation.

This packages the current guard implementation rather than redesigning it:

- Discovery runs at package installation and guard service startup. A container
  added later is marked **尚未纳入管理** until it is registered; running
  `/usr/libexec/docker-guard-setup` followed by `/usr/bin/docker-guard apply`
  registers current containers and applies their policies.
- The guard's periodic check still compares rule counts. The LuCI status API
  separately checks current port/protocol/interface coverage and exposes missing
  or partial rules. Content-based automatic repair remains future work.
- The guard uses the first IPv4 default-route interface, falls back to
  `pppoe-wan`, and does not yet implement a configurable WAN interface list.
  Review this behavior before deploying on a different or multi-WAN router.
- No port exceptions are bundled. If needed, add a local UCI list named
  `exempt_ports` under `docker_exposure.settings`; the guard and page read the
  same list. Selecting **阻止 WAN** does not override configured exceptions.
  Loaded rules are not a substitute for external IPv4/IPv6 testing.
- Removing the package stops automatic synchronization but retains the generated
  include and existing kernel rules. Remove those only after arranging the
  desired replacement access policy.

Validate sources with `bash scripts/check-docker-exposure.sh`. To build only
these two packages with an existing OpenWrt SDK, without the recovery Rust build:

```sh
LOCAL_PACKAGES='docker-guard luci-app-docker-exposure' \
SDK_DIR=/path/to/openwrt-sdk bash scripts/build-local-apks.sh
```
