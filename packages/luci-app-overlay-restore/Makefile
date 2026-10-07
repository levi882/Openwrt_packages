include $(TOPDIR)/rules.mk

PKG_NAME:=luci-app-overlay-restore
PKG_VERSION:=0.2.0
PKG_RELEASE:=23
PKG_LICENSE:=MIT
PKGARCH:=all

include $(INCLUDE_DIR)/package.mk

define Package/luci-app-overlay-restore
  SECTION:=luci
  CATEGORY:=LuCI
  SUBMENU:=3. Applications
  TITLE:=Backup migration and post-reboot package recovery
  DEPENDS:=+overlay-restore +luci-base +luci-theme-bootstrap +rpcd-mod-file
  EXTRA_DEPENDS:=overlay-restore (>=0.2.0-r18)
endef

define Build/Compile
endef

define Package/luci-app-overlay-restore/postinst
#!/bin/sh
if [ -z "$${IPKG_INSTROOT}" ]; then
	rm -f /tmp/luci-indexcache.*
	rm -rf /tmp/luci-modulecache/
	/etc/init.d/rpcd reload >/dev/null 2>&1 || true
fi
exit 0
endef

define Package/luci-app-overlay-restore/install
	$(INSTALL_DIR) $(1)/usr/libexec/rpcd $(1)/usr/share/rpcd/acl.d $(1)/usr/share/luci/menu.d
	$(INSTALL_BIN) ./root/usr/libexec/rpcd/overlay-restore $(1)/usr/libexec/rpcd/
	$(INSTALL_DATA) ./root/usr/share/rpcd/acl.d/*.json $(1)/usr/share/rpcd/acl.d/
	$(INSTALL_DATA) ./root/usr/share/luci/menu.d/*.json $(1)/usr/share/luci/menu.d/
	$(INSTALL_DIR) $(1)/www/luci-static/resources/view/system
	$(INSTALL_DATA) ./htdocs/luci-static/resources/view/system/overlay-restore.js $(1)/www/luci-static/resources/view/system/
	$(INSTALL_DIR) $(1)/www/luci-static/resources/overlay_restore
	$(INSTALL_DATA) ./htdocs/luci-static/resources/overlay_restore/*.js $(1)/www/luci-static/resources/overlay_restore/
endef

$(eval $(call BuildPackage,luci-app-overlay-restore))
