# SPDX-FileCopyrightText: 2026 OpenXPlay.org
# SPDX-License-Identifier: AGPL-3.0-only

include $(TOPDIR)/rules.mk

PKG_NAME:=diplay-cast
PKG_VERSION:=0.1.3
PKG_RELEASE:=1

# Prebuilt musl static binaries are produced by ../build-musl.sh and placed in
# ./src/. No PKG_SOURCE: Build/Prepare copies ./src/* directly, so the SDK
# never tries to download a source tarball (there isn't one for a binary-only
# package). Defining PKG_SOURCE here would fail with "No download method
# available" during `make package/diplay-cast/compile`.
PKG_BUILD_DIR:=$(BUILD_DIR)/$(PKG_NAME)-$(PKG_VERSION)

PKG_LICENSE:=AGPL-3.0-only
PKG_MAINTAINER:=OpenXPlay contributors

include $(INCLUDE_DIR)/package.mk

define Package/diplay-cast
  SECTION:=multimedia
  CATEGORY:=Multimedia
  TITLE:=Headless CarPlay receiver with DLNA cast output
  DEPENDS:=+hostapd +wpa-supplicant
endef

define Package/diplay-cast/description
  A headless CarPlay receiver based on OpenXPlay Linux relay, with a DLNA/UPnP
  output that streams MPEG-TS to a TV or head unit that only has a DLNA
  receiver. MFi identity files (identity.pk8, certificate.p7b) must be supplied
  by the user and are NOT included in this package.
endef

define Package/diplay-cast/conffiles
/etc/config/diplay
endef

define Build/Prepare
	mkdir -p $(PKG_BUILD_DIR)
	$(CP) ./src/* $(PKG_BUILD_DIR)/
endef

define Build/Compile
	# Binaries are prebuilt static musl executables; nothing to compile here.
	true
endef

define Package/diplay-cast/install
	$(INSTALL_DIR) $(1)/usr/bin
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/openxplay-relay $(1)/usr/bin/oxp-relay
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/openxplay-receiver $(1)/usr/bin/openxplay-receiver
	$(INSTALL_BIN) ./files/usr/bin/oxp-relay-wrapper $(1)/usr/bin/oxp-relay-wrapper

	$(INSTALL_DIR) $(1)/etc/init.d
	$(INSTALL_BIN) ./files/etc/init.d/diplay-cast $(1)/etc/init.d/diplay-cast

	$(INSTALL_DIR) $(1)/etc/config
	$(INSTALL_CONF) ./files/etc/config/diplay $(1)/etc/config/diplay

	$(INSTALL_DIR) $(1)/etc/diplay/identity
	$(INSTALL_DIR) $(1)/etc/diplay
	chmod 0700 $(1)/etc/diplay/identity

	$(INSTALL_DIR) $(1)/usr/share/rpcd/acl.d
	$(INSTALL_DATA) ./files/usr/share/rpcd/acl.d/luci-app-diplay.json $(1)/usr/share/rpcd/acl.d/luci-app-diplay.json

	$(INSTALL_DIR) $(1)/usr/share/luci/menu.d
	$(INSTALL_DATA) ./files/usr/share/luci/menu.d/luci-app-diplay.json $(1)/usr/share/luci/menu.d/luci-app-diplay.json

	$(INSTALL_DIR) $(1)/www/luci-static/resources/view/diplay
	$(INSTALL_DATA) ./files/www/luci-static/resources/view/diplay/cast.js $(1)/www/luci-static/resources/view/diplay/cast.js
endef

$(eval $(call BuildPackage,diplay-cast))
