#!/usr/bin/env bash
set -euo pipefail

rm -rf feed .package-src
mkdir -p feed .package-src

LUCKY_REPO="${LUCKY_REPO:-https://github.com/levi882/luci-app-lucky.git}"
FAKEHTTP_REPO="${FAKEHTTP_REPO:-https://github.com/levi882/FakeHTTP.git}"

git clone --depth 1 "$LUCKY_REPO" .package-src/lucky
git clone --depth 1 "$FAKEHTTP_REPO" .package-src/fakehttp

cp -a .package-src/lucky/lucky feed/lucky
cp -a .package-src/lucky/luci-app-lucky feed/luci-app-lucky
cp -a .package-src/fakehttp/fakehttp feed/fakehttp
cp -a .package-src/fakehttp/luci-app-fakehttp feed/luci-app-fakehttp
cp -a .package-src/fakehttp/luci-i18n-fakehttp-zh-cn feed/luci-i18n-fakehttp-zh-cn

mkdir -p feed/fakehttp/srcroot
cp -a .package-src/fakehttp/src feed/fakehttp/srcroot/src
cp -a .package-src/fakehttp/include feed/fakehttp/srcroot/include
cp -a .package-src/fakehttp/Makefile feed/fakehttp/srcroot/Makefile.src
cp -a .package-src/fakehttp/LICENSE feed/fakehttp/srcroot/LICENSE

# The FakeHTTP package Makefile is designed to live inside its source
# repository. Keep a source copy inside the package directory and point
# Build/Prepare at that copy for this standalone feed.
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
