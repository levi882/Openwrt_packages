#!/usr/bin/env python3
import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("update-release-apks.py")
SPEC = importlib.util.spec_from_file_location("update_release_apks", MODULE_PATH)
UPDATER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(UPDATER)


def release(tag, *asset_names):
    return {
        "tag_name": tag,
        "assets": [
            {"name": name, "browser_download_url": f"https://example.invalid/{name}"}
            for name in asset_names
        ],
    }


class SelectHomeboxAssetsTests(unittest.TestCase):
    def test_selects_matching_x86_64_openwrt_apks(self):
        candidate = release(
            "v0.1.4",
            "homebox-0.1.4-r1-openwrt-25.12.5-aarch64_generic.apk",
            "homebox-0.1.4-r1-openwrt-25.12.5-x86_64.apk",
            "luci-app-homebox-0.1.4-openwrt-25.12.5-x86_64.apk",
            "luci-i18n-homebox-zh-cn-0.1.4-openwrt-25.12.5-x86_64.apk",
        )

        main, luci, i18n, version, package_release, openwrt_release = (
            UPDATER.select_homebox_assets(candidate)
        )

        self.assertEqual(
            main["name"], "homebox-0.1.4-r1-openwrt-25.12.5-x86_64.apk"
        )
        self.assertEqual(
            luci["name"], "luci-app-homebox-0.1.4-openwrt-25.12.5-x86_64.apk"
        )
        self.assertEqual(
            i18n["name"],
            "luci-i18n-homebox-zh-cn-0.1.4-openwrt-25.12.5-x86_64.apk",
        )
        self.assertEqual(version, "0.1.4")
        self.assertEqual(package_release, "r1")
        self.assertEqual(openwrt_release, "25.12.5")


class SelectSmartdnsAssetsTests(unittest.TestCase):
    def test_pairs_webui_daemon_and_luci_across_releases(self):
        releases = [
            release(
                "1.2026.v48.4.1_with_ui",
                "smartdns.1.2026.v48.4.1.x86_64.apk",
            ),
            release(
                "1.2026.v48.4.1",
                "smartdns.1.2026.v48.4.1.x86_64-openwrt.apk",
                "luci-app-smartdns.1.2026.v48.4.1.luci-all.apk",
            ),
        ]

        selected = UPDATER.select_smartdns_assets(releases)

        self.assertIsNotNone(selected)
        main_release, main, luci_release, luci, version = selected
        self.assertEqual(main_release["tag_name"], "1.2026.v48.4.1_with_ui")
        self.assertEqual(main["name"], "smartdns.1.2026.v48.4.1.x86_64.apk")
        self.assertEqual(luci_release["tag_name"], "1.2026.v48.4.1")
        self.assertEqual(
            luci["name"], "luci-app-smartdns.1.2026.v48.4.1.luci-all.apk"
        )
        self.assertEqual(version, "1.2026.v48.4.1")

    def test_rejects_static_openwrt_daemon(self):
        releases = [
            release(
                "1.2026.v48.4.1",
                "smartdns.1.2026.v48.4.1.x86_64-openwrt.apk",
                "luci-app-smartdns.1.2026.v48.4.1.luci-all.apk",
            )
        ]

        self.assertIsNone(UPDATER.select_smartdns_assets(releases))


if __name__ == "__main__":
    unittest.main()
