# OpenWrt Personal APK Feed

Personal OpenWrt 25.12 `x86_64` APK feed for packages that are not in the firmware's default repositories.

The workflow downloads these release APKs and adds them to the same signed feed:

- `luci-theme-aurora`
- `luci-app-aurora-config`
- `luci-i18n-aurora-config-zh-cn`
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
- `bandix`
- `luci-app-bandix`
- `luci-i18n-bandix-zh-cn`
- `nikki`
- `luci-app-nikki`
- `luci-i18n-nikki-zh-cn`

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

`router/restore_overlay.sh` is intentionally simple. After a firmware upgrade,
run it with an overlay backup:

```sh
./router/restore_overlay.sh overlay_backup.tar.gz
```

It only does this:

- clears `/overlay`
- restores the backup
- removes old kernel module state from the backup
- removes old APK/OPKG package-manager state from the backup
- removes old package files from the backup, except for the temporary runtime
  packages listed in `RESTORE_KEEP_RUNTIME_PACKAGES`
- keeps the current system's pre-restore
  `/etc/apk/repositories.d/distfeeds.list`, so firmware-generated mirrors such
  as Tencent, cooluc, and core3 are not replaced by the backup or by ROM
  defaults
- keeps the current system's pre-restore myfeed repository file and public key,
  if they already exist
- removes old LuCI runtime, static files, RPC ACL files, CGI entry files, and
  common overlay whiteouts so the new firmware uses its own LuCI
- detects LuCI theme and theme-config packages from the backup, then reinstalls
  them from the current repositories on first boot so the theme selector is
  populated again; if APK still thinks a theme is installed after LuCI files
  were cleaned, the first-boot action runs `apk fix --reinstall` for those
  theme packages. If a theme APK is no longer available from the current
  repositories, the restore keeps that theme's backed-up LuCI files as a
  fallback instead of deleting them.
- schedules a one-shot first-boot `apk del` for selected LuCI packages and
  Chinese translations that are preinstalled by the new firmware: USB printer,
  nlbwmon, eqos, sqm, PassWall, HomeProxy, qBittorrent, MosDNS, DDNS,
  AirConnect, AirPlay2, frpc, mentohust, natmap, OpenList2, socat, wolplus,
  ZeroTier, WireGuard protocol UI, Argon theme, and Argon config

It does not add new feeds or run a second manual post-restore step.

By default, only `smartdns` and `nikki` runtime files are temporarily kept from
the backup. The first-boot package action tries to start them before `apk
update`, which gives the router a chance to use the restored DNS/proxy path if
Cloudflare Pages or other HTTPS routes are flaky. Their LuCI files and APK
ownership state are still removed; once the feed is reachable, APK reinstalls
and takes ownership again. Change that list with `RESTORE_KEEP_RUNTIME_PACKAGES`,
or set it to an empty string to keep no package runtime files.

On first boot after restore, it installs the official-source `omcproxy` packages.
It also temporarily rewrites the existing myfeed repository as tagged `@myfeed`,
installs the personal-feed packages for Bandix, EasyTier, FakeHTTP, Lucky,
Nikki, rtp2httpd, SmartDNS, their LuCI apps, and their translations with
`package@myfeed`, then restores myfeed back to an ordinary untagged repository
and removes `@myfeed` from `/etc/apk/world`.
If the existing myfeed repository file is missing or cannot be tagged, those
personal-feed packages are left pending instead of being installed from the
firmware repositories by accident.

Change the official-source install list with `RESTORE_INSTALL_PACKAGES`.
Change the forced-myfeed install list with `RESTORE_MYFEED_INSTALL_PACKAGES`.
Set either variable to an empty string to skip that install group.

Configs and service data from the backup are restored normally. Package binaries,
LuCI files, translations, and APK ownership state are not preserved from the
backup; selected packages, including detected LuCI themes and theme-config
packages, are reinstalled on first boot from the current repositories. Theme
files from unavailable APKs are preserved from the backup.

For `/etc/config/fstab`, the default behavior matches an extroot restore flow:
it restores normal mount points from the backup, but drops backup entries whose
target is `/overlay` or `/`, then appends the current system's pre-restore
extroot entry. That keeps a freshly formatted external overlay while bringing
back data-disk mounts such as Docker storage.

If you explicitly want the backup's full fstab restored, including its old
extroot entry, run:

```sh
RESTORE_KEEP_EXTROOT=1 ./router/restore_overlay.sh overlay_backup.tar.gz
```

To change which preinstalled LuCI pages are removed, set
`RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES`. Set it to an empty string to skip
this step.
