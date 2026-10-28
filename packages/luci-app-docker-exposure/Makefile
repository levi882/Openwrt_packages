include $(TOPDIR)/rules.mk

PKG_NAME:=luci-app-docker-exposure
PKG_VERSION:=0.1.0
PKG_RELEASE:=2
PKG_LICENSE:=MIT
PKGARCH:=all

include $(INCLUDE_DIR)/package.mk

define Package/luci-app-docker-exposure
  SECTION:=luci
  CATEGORY:=LuCI
  SUBMENU:=3. Applications
  TITLE:=Docker published port WAN access control
  DEPENDS:=+docker-guard +luci-base +rpcd +jshn +jsonfilter
endef

define Build/Compile
endef

define Package/luci-app-docker-exposure/postinst
#!/bin/sh
if [ -z "$${IPKG_INSTROOT}" ]; then
	rm -f /tmp/luci-indexcache.*
	/etc/init.d/rpcd reload >/dev/null 2>&1 || true
fi
exit 0
endef

define Package/luci-app-docker-exposure/install
	$(INSTALL_DIR) $(1)/usr/libexec/rpcd $(1)/usr/share/rpcd/acl.d $(1)/usr/share/luci/menu.d
	$(INSTALL_BIN) ./root/usr/libexec/rpcd/docker-exposure $(1)/usr/libexec/rpcd/
	$(INSTALL_DATA) ./root/usr/share/rpcd/acl.d/*.json $(1)/usr/share/rpcd/acl.d/
	$(INSTALL_DATA) ./root/usr/share/luci/menu.d/*.json $(1)/usr/share/luci/menu.d/
	$(INSTALL_DIR) $(1)/www/luci-static/resources/view/docker-exposure
	$(INSTALL_DATA) ./htdocs/luci-static/resources/view/docker-exposure/overview.js $(1)/www/luci-static/resources/view/docker-exposure/
endef

$(eval $(call BuildPackage,luci-app-docker-exposure))
