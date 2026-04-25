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
wget -O /etc/apk/keys/myfeed.pem https://openwrt-packages.pages.dev/public-key.pem
echo "https://openwrt-packages.pages.dev/openwrt-25.12/x86_64/myfeed/packages.adb" > /etc/apk/repositories.d/00-myfeed.list
apk update
```

## Restore Script

Copy `router/add-myfeed.sh` into the `POSTEOF` section of your restore script, then call `add_myfeed` before `apk update`.
