#!/usr/bin/env bash
set -euo pipefail

rm -rf feed .package-src
mkdir -p feed .package-src

LUCKY_REPO="${LUCKY_REPO:-https://github.com/levi882/luci-app-lucky.git}"
EASYTIER_REPO="${EASYTIER_REPO:-https://github.com/EasyTier/luci-app-easytier.git}"
SMARTDNS_REPO="${SMARTDNS_REPO:-https://github.com/pymumu/smartdns.git}"
LUCI_BANDIX_REPO="${LUCI_BANDIX_REPO:-https://github.com/timsaya/luci-app-bandix.git}"
BANDIX_REPO="${BANDIX_REPO:-https://github.com/timsaya/openwrt-bandix.git}"
RTP2HTTPD_REPO="${RTP2HTTPD_REPO:-https://github.com/stackia/rtp2httpd.git}"
FAKEHTTP_REPO="${FAKEHTTP_REPO:-https://github.com/levi882/FakeHTTP.git}"

git clone --depth 1 "$LUCKY_REPO" .package-src/lucky
git clone --depth 1 "$EASYTIER_REPO" .package-src/easytier
git clone --depth 1 "$SMARTDNS_REPO" .package-src/smartdns
git clone --depth 1 "$LUCI_BANDIX_REPO" .package-src/luci-app-bandix
git clone --depth 1 "$BANDIX_REPO" .package-src/openwrt-bandix
git clone --depth 1 "$RTP2HTTPD_REPO" .package-src/rtp2httpd
git clone --depth 1 "$FAKEHTTP_REPO" .package-src/fakehttp

cp -a .package-src/lucky/lucky feed/lucky
cp -a .package-src/lucky/luci-app-lucky feed/luci-app-lucky
cp -a .package-src/easytier/easytier feed/easytier
cp -a .package-src/easytier/luci-app-easytier feed/luci-app-easytier
cp -a .package-src/smartdns/package/openwrt feed/smartdns
cp -a .package-src/luci-app-bandix/luci-app-bandix feed/luci-app-bandix
cp -a .package-src/openwrt-bandix/openwrt-bandix feed/bandix
cp -a .package-src/rtp2httpd/openwrt-support/rtp2httpd feed/rtp2httpd
cp -a .package-src/rtp2httpd/openwrt-support/luci-app-rtp2httpd feed/luci-app-rtp2httpd
cp -a .package-src/fakehttp/fakehttp feed/fakehttp
cp -a .package-src/fakehttp/luci-app-fakehttp feed/luci-app-fakehttp
cp -a .package-src/fakehttp/luci-i18n-fakehttp-zh-cn feed/luci-i18n-fakehttp-zh-cn

mkdir -p feed/rtp2httpd/srcroot feed/fakehttp/srcroot
tar --exclude=.git --exclude=openwrt-support -C .package-src/rtp2httpd -cf - . | tar -C feed/rtp2httpd/srcroot -xf -
cp -a .package-src/fakehttp/src feed/fakehttp/srcroot/src
cp -a .package-src/fakehttp/include feed/fakehttp/srcroot/include
cp -a .package-src/fakehttp/Makefile feed/fakehttp/srcroot/Makefile.src
cp -a .package-src/fakehttp/LICENSE feed/fakehttp/srcroot/LICENSE

# The upstream SmartDNS package Makefile is laid out for the OpenWrt packages
# feed, where ../../lang/rust exists. In this standalone feed, use the SDK's
# cloned packages feed instead.
sed -i 's#include ../../lang/rust/rust-package.mk#include $(TOPDIR)/feeds/packages/lang/rust/rust-package.mk#' \
  feed/smartdns/Makefile

# rtp2httpd and FakeHTTP package Makefiles are designed to live inside their
# source repositories. Keep source copies inside the package directory and point
# Build/Prepare at those copies for this standalone feed.
tmp_makefile="$(mktemp)"
awk '{
  if ($0 == "\t$(CP) $(CURDIR)/../../* $(PKG_BUILD_DIR)/") {
    print "\t$(CP) $(CURDIR)/srcroot/* $(PKG_BUILD_DIR)/"
  } else {
    print
  }
}' feed/rtp2httpd/Makefile > "$tmp_makefile"
mv "$tmp_makefile" feed/rtp2httpd/Makefile
sed -i \
  -e 's#$(CURDIR)/../src#$(CURDIR)/srcroot/src#g' \
  -e 's#$(CURDIR)/../include#$(CURDIR)/srcroot/include#g' \
  -e 's#$(CURDIR)/../Makefile#$(CURDIR)/srcroot/Makefile.src#g' \
  -e 's#$(CURDIR)/../LICENSE#$(CURDIR)/srcroot/LICENSE#g' \
  feed/fakehttp/Makefile

tmp_makefile="$(mktemp)"
awk '{
  if ($0 == "\t$(CP) $(CURDIR)/srcroot/Makefile.src $(PKG_BUILD_DIR)/") {
    print "\t$(CP) $(CURDIR)/srcroot/Makefile.src $(PKG_BUILD_DIR)/Makefile"
  } else {
    print
  }
}' feed/fakehttp/Makefile > "$tmp_makefile"
mv "$tmp_makefile" feed/fakehttp/Makefile

for pkg in feed/fakehttp feed/luci-app-fakehttp feed/luci-i18n-fakehttp-zh-cn; do
  [ -f .package-src/fakehttp/version.mk ] && cp .package-src/fakehttp/version.mk "$pkg/version.mk"
  sed -i 's#-include $(THIS_DIR)/../version.mk#-include $(THIS_DIR)/version.mk#' "$pkg/Makefile"
done

echo "Collected package Makefiles:"
find feed -mindepth 2 -maxdepth 2 -name Makefile -print | sort
