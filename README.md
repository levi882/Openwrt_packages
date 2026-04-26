# OpenWrt Personal APK Feed

Personal OpenWrt 25.12 `x86_64` APK feed for packages that are not in the firmware's default repositories.

The workflow downloads these release APKs and adds them to the same signed feed:

- `lucky`
- `luci-app-lucky`
- `luci-i18n-lucky-zh-cn`
- `easytier`
- `luci-app-easytier`
- `luci-i18n-easytier-zh-cn`
- `rtp2httpd`
- `luci-app-rtp2httpd`
- `luci-i18n-rtp2httpd-zh-cn`
- `fakehttp`
- `luci-app-fakehttp`
- `luci-i18n-fakehttp-zh-cn`
- `smartdns`
- `luci-app-smartdns`
- `luci-app-smartdns-lite`
- `bandix`
- `luci-app-bandix`
- `luci-i18n-bandix-zh-cn`
- `nikki`
- `luci-app-nikki`
- `luci-i18n-nikki-ru`
- `luci-i18n-nikki-zh-cn`
- `luci-i18n-nikki-zh-tw`

The published repository path is:

```text
https://openwrt-packages.pages.dev/openwrt-25.12/x86_64/myfeed/packages.adb
```

## First Setup

Generate the APK feed signing key:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\generate-apk-key.ps1
```

Add the full content of `private-key.pem` as a GitHub repository secret named:

```text
PRIVATE_KEY
```

Commit `public-key.pem`, `.github/`, `scripts/`, `router/`, and this README. Do not commit `private-key.pem`.

Create a Cloudflare Pages project named:

```text
openwrt-packages
```

Then add these GitHub repository secrets:

```text
CLOUDFLARE_ACCOUNT_ID
CLOUDFLARE_API_TOKEN
PRIVATE_KEY
```

## Build

Push to `main`, or run the `build-feed` workflow manually.

## Update Release APK Pins

Run the `update-release-apks` workflow manually when you want to check for new
upstream releases. It also runs weekly and opens a pull request when versions or
sha256 pins changed. Merge that pull request to trigger the normal feed build.

After a successful run, the router can use:

```sh
MYFEED_BASE=https://openwrt-packages.pages.dev
wget -O /etc/apk/keys/myfeed.pem "$MYFEED_BASE/public-key.pem"
echo "$MYFEED_BASE/openwrt-25.12/x86_64/myfeed/packages.adb" > /etc/apk/repositories.d/00-myfeed.list
apk update
```

## Restore Script

`router/restore_overlay.sh` probes `MYFEED_BASES` in order and writes the first
reachable mirror as tagged `@myfeed`. If Cloudflare Pages is unstable, mirror
the generated `public/` tree to a domestic HTTPS site, keeping this layout:

```text
public-key.pem
openwrt-25.12/x86_64/myfeed/packages.adb
openwrt-25.12/x86_64/myfeed/*.apk
```

Then prefer that mirror during restore:

```sh
MYFEED_BASES="https://core3.cooluc.com/openwrt-packages https://openwrt-packages.pages.dev" /root/post_restore_reinstall.sh
```

During the post-restore reinstall step, the script tries to start `smartdns`
and `nikki` before `apk update`, so router-originated package traffic can use
the restored proxy path. The first restore stage preserves the dependency
closure for that bootstrap path from the backup APK database, while still
dropping old `kernel=` and `kmod-*` state.

The restore step uses tagged `@myfeed` internally so it can force selected
packages to come from the personal feed. After the post-restore reinstall
finishes, it switches `00-myfeed.list` back to an untagged repository by
default. That makes LuCI's package manager install buttons work with myfeed
packages, because LuCI calls `apk add <package>` rather than
`apk add <package>@myfeed`. If the untagged feed makes `apk update` fail, the
script rolls back to tagged `@myfeed`; if that still fails, it disables myfeed
so official package installs keep working. The compatibility step also removes
stale `@myfeed` suffixes from `/etc/apk/world`, otherwise ordinary installs can
fail with "repository tag ... does not exist" after changing the feed mode.
When myfeed has to be disabled, myfeed-only root packages are removed from
`/etc/apk/world` as well, leaving already-installed files in place but avoiding
future `apk add` failures caused by unavailable myfeed packages.

Packages that still fail after restore are quarantined out of `/etc/apk/world`
before the script exits, including tagged or version-constrained variants. This
keeps later LuCI or SSH installs from being blocked by failed restore targets.

By default, `smartdns`, `lucky`, and their LuCI packages are treated as
backup-kept runtime packages. The first restore stage preserves their package
files from the backup, and the post-restore reinstall step skips them so `apk
add` does not overwrite the restored binaries or configs. Override the default
list with `RESTORE_KEEP_BACKUP_PACKAGES` if needed.

The first stage also keeps LuCI files for personal-feed packages as a fallback.
If myfeed is unreachable during post-restore, those packages are left in place
from the backup and removed from `/etc/apk/world` instead of being reported as
missing repository packages.

Disable that behavior only for troubleshooting:

```sh
RESTORE_PROXY_UP=0 /root/post_restore_reinstall.sh
```

Advanced debugging knobs:

```sh
KEEP_PROXY_UP=0 /root/post_restore_reinstall.sh
RESTORE_LUCI_WRAPPER=0 /root/post_restore_reinstall.sh
MYFEED_LUCI_COMPAT=0 /root/post_restore_reinstall.sh
RESTORE_KEEP_BACKUP_PACKAGES="smartdns lucky luci-app-lucky" /root/post_restore_reinstall.sh
RESTORE_KEEP_MYFEED_WHEN_FEED_DOWN=0 /root/post_restore_reinstall.sh
RESTORE_BOOTSTRAP_ROOTS="smartdns nikki" ./router/restore_overlay.sh overlay_backup.tar.gz
```
