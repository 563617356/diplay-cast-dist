# DiPlay Cast — OpenWrt 无头 CarPlay → DLNA 接收端

> 本仓库是 [OpenXPlay-Stream](https://github.com/OpenXPlay/OpenXPlay-Stream) 的 fork，新增了 **DLNA/UPnP cast 输出**（`rust/oxp-cast`），让无头 OpenWrt 盒子接收 CarPlay 后，把画面推给**只有 DLNA 接收器、没有浏览器**的车机屏幕。[English](README.md) · [OpenWrt 打包文档](openwrt/README.md)

## 本 fork 的改动

| 路径 | 说明 |
| --- | --- |
| `rust/oxp-cast/` | **新增 crate**：DLNA 输出。SSDP 发现 → SOAP AVTransport → HTTP `video/mp2t` 推流。从上游纯 JVM 的 `kotlin/cast` 移植，复用 `oxp-media` 的 MPEG-TS 封装，实现标准 `ProjectionOutput` trait。 |
| `rust/oxp-bin/src/bin/oxp-relay.rs` | 增加 `--cast` 系列参数，把 cast 输出挂进 relay hub。 |
| `relay/rust/` | 内置 [OpenXPlay-Relay](https://github.com/OpenXPlay/OpenXPlay-Relay) 的 `direlay-linux`（接收端）：连 iPhone、iAP2/MFi 认证、拉起输出进程。v0.1.1 起随包分发，所有组件都在路由器上。 |
| `openwrt/` | OpenWrt 打包骨架：procd 服务、UCI 配置、LuCI 页面、musl 交叉编译 + ipk 打包脚本。 |
| `.github/workflows/build.yml` | **云编译**：测试 oxp-cast、交叉编译接收端 + 输出端 aarch64/x86_64 静态二进制、打包 ipk、推 `v*` tag 自动发布 Release。 |

## 运行架构（v0.1.1 起）

```
iPhone ──蓝牙/Wi-Fi──> openxplay-receiver (direlay-linux, 主进程)
                          │  iAP2 + MFi 认证
                          │  spawn --oxp-relay=oxp-relay-wrapper
                          ▼
                       oxp-relay-wrapper ──注入 --cast──> oxp-relay (输出端)
                          │  unix socket 传帧
                          ▼  video/mp2t
                       车机 DLNA MediaRenderer
```

`oxp-relay-wrapper` 用 `exec` 保持 PID 与 stdio 管道不变，把 `--cast` 系列参数加在接收端传入的参数前面——不改动上游接收端一行代码。

## 下载预编译 ipk

前往 [Releases](../../releases)，按路由器架构选择：

- `*_aarch64_cortex-a53.ipk` — ARMv8 软路由（N1 / R4S / MT6000）
- `*_x86_64.ipk` — x86 软路由

**MFi 证书不随包分发**：安装后自行把 `identity.pk8` / `certificate.p7b` 放到 `/etc/diplay/identity/`（权限 600），再通过 `/etc/config/diplay` 或 LuCI → 服务 → DiPlay Cast 配置。详见 [openwrt/README.md](openwrt/README.md)。

## 自行编译

推送 `v*` tag（或手动触发 workflow），GitHub Actions 会自动产出两个 ipk。本地 Linux/macOS 编译：

```bash
cd openwrt
TARGET=aarch64-unknown-linux-musl bash build-musl.sh
TARGET=aarch64-unknown-linux-musl bash pack-ipk.sh
```

## 署名与许可

派生自 [OpenXPlay-Stream](https://github.com/OpenXPlay/OpenXPlay-Stream)，AGPL-3.0-only，保留 "Powered by OpenXPlay." 署名。

---

<p align="center"><a href="README.md">English</a> · <b>简体中文</b></p>

# OpenXPlay-Stream (upstream)

[OpenXPlay](https://github.com/OpenXPlay/OpenXPlay) 中继的输出端。Stream 从中继接收投屏会话（视频、音频、触控和按键），交给车机上已有的软件，车机无需安装任何软件：

| 输出 | 车机使用 | 触控 | 状态 |
| --- | --- | --- | --- |
| Web | 车机浏览器：H.264 经安全 WebSocket 传输，由 WebCodecs 解码 | 支持 | 开发中 |
| Carlink | 车机的 Carlink 应用，经 Wi-Fi 连接 | 支持 | 实验性 |
| Cast | 投屏接收端（DLNA） | 不支持 | 计划中 |

每个输出只与 OXP-Stream 模型交互，或在进程内，或通过 Stream 传输协议。输入与输出互不依赖，因此同一套输出可服务所有中继。CarPlay 输入由 [OpenXPlay Relay](https://github.com/OpenXPlay/OpenXPlay-Relay) 提供。

## 目录结构

模型和各输出按运行时分别实现，共三套；共享的协议测试向量让三套实现保持一致。

| 目录 | 语言 | 内容 | 使用方 |
| --- | --- | --- | --- |
| [`kotlin/`](kotlin) | Kotlin/JVM（Gradle） | `core`（会话模型和 hub）、`wire`（传输协议编解码）、`web-server`、`clink`（Carlink 输出）、`tools/`（回放、开发用中继、模拟车机） | Android 中继应用 |
| [`rust/`](rust) | Rust（Cargo） | `oxp-core`、`oxp-wire`、`oxp-web`、`oxp-clink`、`oxp-media`、`oxp-link-linux`（通过 BlueZ 和 NetworkManager 建立 Carlink Wi-Fi 连接）、`oxp-bin`（`oxp-relay` 输出进程） | Linux 中继 |
| [`web/`](web) | TypeScript（Vite） | `receiver/`（车机浏览器打开的页面）、`protocol/`、`shared/`、`probe/`（解码器能力探测） | 由两种中继提供 |
| [`spec/vectors/`](spec/vectors) | 数据 | 传输协议和 Web 协议的测试向量 | 三套实现的测试 |

## 构建与测试

| 部分 | 要求 | 命令 |
| --- | --- | --- |
| Kotlin | JDK 17 | `cd kotlin && ./gradlew test` |
| Rust | Rust 稳定版 | `cd rust && cargo test --workspace` |
| 面向 aarch64 Linux 的 Rust | `rustup target add aarch64-unknown-linux-musl`、`llvm-tools`、clang | `rust/scripts/build-aarch64.sh` |
| Web 接收端 | Node.js 20.19+ 或 22.12+ | 在 `kotlin/web-receiver` 和 `rust/web-receiver` 中分别运行 `npm ci && npm test && npm run build`（输出在各自的 `dist`） |

Android 中继打包 `kotlin/web-receiver/dist`，Linux 中继打包 `rust/web-receiver/dist`，请先构建对应的接收端，再构建中继。两份接收端是相互独立的代码。

## 参与贡献

贡献须遵循 [OpenXPlay 贡献者许可协议](https://github.com/OpenXPlay/OpenXPlay/blob/main/CLA.md)（协议文本为英文）。在任意一个 OpenXPlay 仓库的 pull request 里按提示回复一次即可，之后所有仓库的 CLA 检查都会通过。

## 许可证

本项目采用 [GNU Affero General Public License v3.0 (AGPLv3)](./LICENSE) 授权，仅限第 3 版。

本项目适用依据 AGPLv3 第 7 条设定的附加条款。修改版本必须在相应的法律声明中，以及用户界面中任何显著的“关于”、法律、页脚或署名位置，保留署名声明 `Powered by OpenXPlay.`，并附上指向原项目的可见链接：<https://github.com/OpenXPlay/OpenXPlay-Stream>。详见 [NOTICE](./NOTICE)。

如果贵组织的政策不允许使用 AGPLv3 许可的软件，或你希望免于履行 AGPLv3 的开源义务，请联系：[support@openxplay.org](mailto:support@openxplay.org)

CarPlay 为 Apple Inc. 的商标。OpenXPlay 为独立项目，与 Apple 不存在隶属、认可或认证关系。
