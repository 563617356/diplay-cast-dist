# DiPlay Cast — OpenWrt 无头 CarPlay → DLNA 接收端

把 iPhone 的 CarPlay 接收到一台无头 Linux/OpenWrt 盒子上，再通过 DLNA 把画面推给**只有 DLNA 接收器、没有浏览器**的车机屏幕。

## 架构

```
iPhone ──(Wi-Fi / USB)──> direlay-linux (CarPlay 接收, Rust)
                              │  oxp-wire 协议 (unix socket)
                              ▼
                          oxp-relay --cast
                              │  oxp-cast (本项目新增)
                              │   SSDP 发现 + SOAP AVTransport + HTTP
                              ▼  video/mp2t (MPEG-TS) 流
                         车机 DLNA MediaRenderer (只显示画面)
```

- `upstream/OpenXPlay-Stream/rust/oxp-cast/` 是新增的 DLNA 输出 crate，实现了 `ProjectionOutput`，复用 `oxp-media` 的 `TsMuxer`。
- `oxp-relay` 增加了 `--cast` 系列参数；`direlay-linux` 无需改动。
- 车机端**不能触控**，操作在 iPhone 上完成；画面/声音有 DLNA 固有延迟（数秒）。

## 目录

```
openwrt/
├── diplay-cast/                 # OpenWrt package
│   ├── Makefile
│   ├── src/                     # 预编译 musl 静态二进制放这里
│   └── files/
│       ├── etc/config/diplay    # UCI 配置
│       ├── etc/init.d/diplay-cast  # procd 服务
│       ├── etc/diplay/identity/ # 用户自放 MFi 证书 (不打包!)
│       └── www/luci-static/...  # LuCI 页面
├── build-musl.sh                # 交叉编译脚本
└── README.md
```

## 构建

### 云编译（推荐，无需本地工具链）

在本仓库推送 `v*` tag，或在 Actions 页手动触发 `Build OpenWrt ipk + apk` workflow。它会自动：

1. 运行 `oxp-cast` 单元测试
2. 交叉编译 aarch64 + x86_64 两个架构的静态 musl 二进制
3. 各打一个 `.ipk`（opkg 固件）和一个 `.apk`（apk 固件）
4. 在 Release 页发布（tag 触发时）

编译产物在 workflow run 的 Artifacts 里也能下载。

**为什么要两种格式**：OpenWrt/ImmortalWrt 25.12 起把包管理器从 opkg 换成 apk，
`.ipk`（ar 归档）和 `.apk`（ADB 签名归档，魔数 `ADBd`）互不兼容，装错格式会直接报错。
`.ipk` 可以用 `ar + tar` 手工打包；`.apk` 必须用对应版本的官方 SDK 编译产出
（见 `build-apk.sh`），不能手工伪造。按你的固件选：

| 固件 | 包管理器 | 安装包 | 安装命令 |
|---|---|---|---|
| 24.10 及更早 | opkg | `.ipk` | `opkg install diplay-cast_*.ipk` |
| 25.12+ | apk-tools 3.x | `.apk` | `apk add --allow-untrusted diplay-cast-*.apk` |

### 本地编译

在 Linux / macOS 主机上（Windows 原生不行，musl 交叉编译需要 Unix 工具链；需要 clang、make、curl、tar、binutils）：

```bash
cd openwrt
# 1. 编译静态二进制（含 oxp-cast）到 diplay-cast/src/
TARGET=aarch64-unknown-linux-musl bash build-musl.sh
# x86 软路由: TARGET=x86_64-unknown-linux-musl bash build-musl.sh

# 2a. 打成 ipk（不依赖 OpenWrt SDK，用 ar + tar）
TARGET=aarch64-unknown-linux-musl bash pack-ipk.sh
# 产物: openwrt/dist/diplay-cast_0.1.2-1_<arch>.ipk

# 2b. 打成 apk（需要下载 OpenWrt 25.12.5 SDK，Linux/macOS 主机）
TARGET=aarch64-unknown-linux-musl bash build-apk.sh
# 产物: openwrt/dist/diplay-cast-0.1.2-r1_<arch>.apk
```

`build-musl.sh` 已处理 ring（加密依赖）所需的 musl sysroot、clang 参数与静态归档器，并支持 aarch64 / x86_64 两个目标。

## 安装与配置

1. 安装包（按固件选一种，不要混用）：
   ```sh
   # 24.10 及更早 (opkg)
   opkg install diplay-cast_0.1.2-1_<arch>.ipk
   # 25.12+ (apk)
   apk add --allow-untrusted diplay-cast-0.1.2-r1_<arch>.apk
   ```
2. **放置 MFi 身份**（必须自己做，包里绝不含）：

   ```sh
   mkdir -p /etc/diplay/identity
   # 用 scp 上传 identity.pk8 和 certificate.p7b
   scp identity.pk8 root@路由器:/etc/diplay/identity/
   scp certificate.p7b root@路由器:/etc/diplay/identity/
   chmod 0700 /etc/diplay/identity
   chmod 0600 /etc/diplay/identity/*
   ```

3. 编辑 `/etc/config/diplay`（或 LuCI → 服务 → DiPlay Cast），重点是：
   - `connection`：`wireless` 或 `wired`（USB Host 连 iPhone）
   - `wifi_ssid` / `wifi_key` / `wifi_country`：中继热点，车机连它
   - `cast_udn`：可选，指定车机 DMR 的 UDN；留空自动选第一个
   - `cast_interface`：可选，指定 LAN 口 IPv4，避免多网卡选错
4. 启动：`/etc/init.d/diplay-cast enable && /etc/init.d/diplay-cast start`
5. 车机打开 DLNA/投屏接收应用，中继会自动发现它并开始推流。

## 硬件要求

- **架构**：aarch64（N1/R4S/MT6000）或 x86_64 软路由；16MB flash 的 MIPS 老路由不行
- **RAM**：≥256MB 更稳（relay 本身占用小，但 Wi-Fi/hostapd 要空间）
- **USB 有线**：需要 USB Host，且必要时用 GPIO 给 VBUS 供电
- **无线**：需要 5GHz AP + 蓝牙 BR/EDR（多数路由无蓝牙，需 USB 双模狗 + BlueZ）
- 首次无线配对用 Just-Works（NoInputNoOutput），iPhone 在蓝牙设置页点一次

## 许可证与合规（重要）

- 中继与输出是 **AGPL-3.0**：网络使用需开源，保留 "Powered by OpenXPlay." 署名；派生部分（DiPlay/xcertplay）为 GPL-3.0。
- **MFi 身份是你自行提供的实验性身份**（通常从公开固件恢复），不是 Apple 签发给你的；项目明确不附带、未来 iOS 可能拒绝。
- **绝不要把 identity.pk8 / certificate.p7b 打进固件镜像或提交仓库**——那等于公开私钥。证书放在 `/etc/diplay/identity/`，权限 600，且排除在 sysupgrade 镜像之外。
- 本方案仅供个人研究/自用；商业化分发有 Apple MFi 合规风险。

## 已知限制

- 车机端无触控、无 DLNA 反向控制
- DLNA 延迟数秒；声音可改用 iPhone 蓝牙直连车机规避
- Siri/通话用 iPhone 麦克风
- Linux 中继官方不支持 DLNA，本功能是社区移植（AGPL），需自行测试车机 DMR 兼容性（`GetProtocolInfo` 应含 `video/mp2t`）
