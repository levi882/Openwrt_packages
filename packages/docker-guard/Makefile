include $(TOPDIR)/rules.mk

PKG_NAME:=docker-guard
PKG_VERSION:=0.1.0
PKG_RELEASE:=1
PKG_LICENSE:=MIT
PKGARCH:=all

include $(INCLUDE_DIR)/package.mk

define Package/docker-guard
  SECTION:=net
  CATEGORY:=Network
  TITLE:=Docker published port WAN access guard for firewall4
  DEPENDS:=+docker +dockerd +firewall4 +nftables-json +uci +procd +busybox
endef

define Package/docker-guard/description
  Maintain persistent firewall4 prerouting rules for configured Docker ports.
  Includes Docker events synchronization, periodic checks and startup setup.
endef

define Package/docker-guard/conffiles
/etc/config/docker_exposure
endef

define Build/Compile
endef

define Package/docker-guard/postinst
#!/bin/sh
if [ -z "$${IPKG_INSTROOT}" ]; then
	/usr/libexec/docker-guard-setup || exit 1
	/etc/init.d/cron enable
	/etc/init.d/cron start >/dev/null 2>&1 || true
	/etc/init.d/docker-guard enable
	/etc/init.d/docker-guard restart || exit 1
fi
exit 0
endef

define Package/docker-guard/prerm
#!/bin/sh
if [ -z "$${IPKG_INSTROOT}" ]; then
	/etc/init.d/docker-guard stop >/dev/null 2>&1 || true
	/etc/init.d/docker-guard disable >/dev/null 2>&1 || true
	/usr/libexec/docker-guard-setup --remove-cron || exit 1
fi
exit 0
endef

define Package/docker-guard/install
	$(INSTALL_DIR) $(1)/usr/bin $(1)/usr/libexec $(1)/etc/init.d $(1)/etc/config $(1)/lib/upgrade/keep.d
	$(INSTALL_BIN) ./files/usr/bin/docker-guard $(1)/usr/bin/
	$(INSTALL_BIN) ./files/usr/libexec/docker-guard-setup $(1)/usr/libexec/
	$(INSTALL_BIN) ./files/etc/init.d/docker-guard $(1)/etc/init.d/
	$(INSTALL_CONF) ./files/etc/config/docker_exposure $(1)/etc/config/
	$(INSTALL_DATA) ./files/lib/upgrade/keep.d/docker-guard $(1)/lib/upgrade/keep.d/
endef

$(eval $(call BuildPackage,docker-guard))
