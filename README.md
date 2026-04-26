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
echo "@myfeed $MYFEED_BASE/openwrt-25.12/x86_64/myfeed/packages.adb" > /etc/apk/repositories.d/00-myfeed.list
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

Disable that behavior only for troubleshooting:

```sh
RESTORE_PROXY_UP=0 /root/post_restore_reinstall.sh
```

Advanced debugging knobs:

```sh
KEEP_PROXY_UP=0 /root/post_restore_reinstall.sh
RESTORE_BOOTSTRAP_ROOTS="smartdns nikki" ./router/restore_overlay.sh overlay_backup.tar.gz
```
