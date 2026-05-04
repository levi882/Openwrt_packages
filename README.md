# OpenWrt Personal APK Feed

Personal OpenWrt 25.12 `x86_64` APK feed and router restore helper.

The feed is published at:

```text
https://openwrt-packages.pages.dev/openwrt-25.12/x86_64/myfeed/packages.adb
```

It currently carries personal-use packages such as Aurora theme, Bandix,
EasyTier, Lucky, Nikki, rtp2httpd, and SmartDNS plus their LuCI packages where
available.

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
```

Push to `main`, or run the `build-feed` workflow manually. The
`update-release-apks` workflow can be run manually to refresh upstream APK pins.

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
- restores the nginx-based IPTV refresh trigger when `/mnt/sda1/iptv` exists

The IPTV trigger is nginx-only. HA can use a stable URL without knowing the
token:

```yaml
rest_command:
  iptv_refresh:
    url: "http://10.1.1.1/iptv/refresh?iface=eth3.3927"
    method: GET
```

Nginx injects the current token when proxying to the local daemon. Generated
IPTV values are also written to:

```text
/mnt/sda1/iptv/config/local/iptv_refresh.env
/mnt/sda1/iptv/config/local/home_assistant_rest_command.yaml
```

## Useful Overrides

```sh
RESTORE_KEEP_EXTROOT=1
RESTORE_INSTALL_PACKAGES="..."
RESTORE_MYFEED_INSTALL_PACKAGES="..."
RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES="..."
RESTORE_IPTV_ENABLE=0
RESTORE_IPTV_REPO_ROOT=/mnt/sda1/iptv
RESTORE_IPTV_REFRESH_IFACE=eth3.3927
RESTORE_IPTV_NGINX_SERVER_CONF=/etc/nginx/conf.d/luci.locations
```

Use `RESTORE_KEEP_EXTROOT=1` only when you want the backup's old extroot entry
restored instead of keeping the current firmware's extroot mount.
