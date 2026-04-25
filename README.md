# OpenWrt Personal APK Feed

Personal OpenWrt 25.12 `x86_64` APK feed for packages that are not in the firmware's default repositories.

The workflow builds:

- `lucky`
- `luci-app-lucky`
- `easytier`
- `luci-app-easytier`
- `smartdns`
- `bandix`
- `luci-app-bandix`
- `rtp2httpd`
- `luci-app-rtp2httpd`
- `fakehttp`
- `luci-app-fakehttp`
- `luci-i18n-fakehttp-zh-cn`

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

After a successful run, the router can use:

```sh
wget -O /etc/apk/keys/myfeed.pem https://openwrt-packages.pages.dev/public-key.pem
echo "https://openwrt-packages.pages.dev/openwrt-25.12/x86_64/myfeed/packages.adb" > /etc/apk/repositories.d/00-myfeed.list
apk update
```

## Restore Script

Copy `router/add-myfeed.sh` into the `POSTEOF` section of your restore script, then call `add_myfeed` before `apk update`.
