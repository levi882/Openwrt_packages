include $(TOPDIR)/rules.mk

PKG_NAME:=overlay-restore
PKG_VERSION:=0.2.0
PKG_RELEASE:=7
PKG_LICENSE:=MIT

OVERLAY_RESTORE_CARGO?=cargo
OVERLAY_RESTORE_RUSTC?=rustc
OVERLAY_RESTORE_CARGO_HOME?=$(HOME)/.cargo

include $(INCLUDE_DIR)/package.mk

define Package/overlay-restore
  SECTION:=utils
  CATEGORY:=Utilities
  TITLE:=Validated configuration migration from OpenWrt overlay backups
  DEPENDS:=@x86_64 +apk-mbedtls +uci +uclient-fetch +libustream-mbedtls +ca-bundle
endef

define Package/overlay-restore/description
  Inspect overlay or sysupgrade backups, migrate configuration and user files,
  and resume package installation after reboot. Optional clean recovery switches
  an internal or external overlay from RAM and retains the previous tree.
endef

define Package/overlay-restore/conffiles
/etc/config/overlay_restore
endef

define Build/Prepare
	$(INSTALL_DIR) $(PKG_BUILD_DIR)
	$(CP) ./src/. $(PKG_BUILD_DIR)/
endef

define Build/Compile
	$(if $(filter x86_64,$(ARCH)),,$(error overlay-restore supports x86_64 only))
	$(OVERLAY_RESTORE_RUSTC) --version | grep -q '^rustc 1\.99\.0 '
	cd $(PKG_BUILD_DIR) && \
		CARGO_HOME="$(OVERLAY_RESTORE_CARGO_HOME)" \
		RUSTC="$(OVERLAY_RESTORE_RUSTC)" \
		CARGO_TARGET_DIR="$(PKG_BUILD_DIR)/target" \
		CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$(TARGET_CC)" \
		RUSTFLAGS='-C target-cpu=x86-64 -C target-feature=+crt-static' \
		$(OVERLAY_RESTORE_CARGO) build --locked --offline --release --target x86_64-unknown-linux-musl
endef

define Package/overlay-restore/postinst
#!/bin/sh
if [ -z "$${IPKG_INSTROOT}" ]; then
	/etc/init.d/overlay-restore enable
	/etc/init.d/overlay-restore restart >/dev/null 2>&1 || true
fi
exit 0
endef

define Package/overlay-restore/install
	$(INSTALL_DIR) $(1)/usr/sbin $(1)/usr/libexec $(1)/lib/upgrade $(1)/etc/init.d $(1)/etc/config
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/target/x86_64-unknown-linux-musl/release/overlay-restore $(1)/usr/sbin/
	$(INSTALL_BIN) ./files/etc/init.d/overlay-restore $(1)/etc/init.d/
	$(INSTALL_DATA) ./files/etc/config/overlay_restore $(1)/etc/config/
	$(INSTALL_BIN) ./files/usr/libexec/overlay-restore-switch $(1)/usr/libexec/
	$(INSTALL_DATA) ./files/lib/upgrade/zz-overlay-restore.sh $(1)/lib/upgrade/
	chmod 0600 $(1)/etc/config/overlay_restore
endef

$(eval $(call BuildPackage,overlay-restore))
