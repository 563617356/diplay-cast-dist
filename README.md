# DiPlay Cast — OpenWrt 安装包与配置

把 iPhone 的 CarPlay 画面，通过一台 OpenWrt 路由器中转，投送到只支持 DLNA 的车机屏幕上。

```
iPhone --CarPlay--> OpenWrt（接收 + 转封装 + DLNA 推送）--DLNA--> 车机屏幕
```

OpenWrt 在这里是**协议网关**：它一边假装成车机接住 iPhone 的 CarPlay 流，另一边假装成 DLNA 媒体服务器把画面推给车机。两端都不需要改造。

## 两个仓库

| 仓库 | 用途 |
|---|---|
| [OpenXPlay-DiPlay-Cast](https://github.com/563617356/OpenXPlay-DiPlay-Cast) | **源码主库**：开发、CI 云编译、Release 发布都在这里 |
| **本仓库** `diplay-cast-dist` | **分发库**：开箱即用的安装包 + 配置 + 完整源码镜像（`source/`），方便直接下载 |

两库内容各自保留。本仓库的 `source/` 是源码主库的完整快照（v0.1.2），若要做二次开发或重新编译，请到源码主库。

## 选哪个文件

先看你的路由器架构，再看固件版本：

### 第 1 步：确认架构

```sh
uname -m
# aarch64 → 用 aarch64_cortex-a53 目录（ARM 路由器，如 IPQ60xx/MT7622/RK3568 等）
# x86_64  → 用 x86_64 目录（x86 软路由/迷你主机）
```

### 第 2 步：确认包格式

```sh
command -v apk && echo "apk 固件" || echo "opkg 固件"
# 有 apk → 25.12+，装 .apk
# 只有 opkg → 24.10 及更早，装 .ipk
```

### 第 3 步：安装

以 `aarch64_cortex-a53` + apk 固件为例：

```sh
cd /tmp
# 把 packages/aarch64_cortex-a53/diplay-cast-0.1.2-r1-aarch64_cortex-a53.apk 传上来
apk add --allow-untrusted diplay-cast-0.1.2-r1-aarch64_cortex-a53.apk
```

opkg 固件：

```sh
opkg install diplay-cast_0.1.2-1_aarch64_cortex-a53.ipk
```

## 目录说明

```
packages/
  aarch64_cortex-a53/          ARM 路由器
    *.apk                      25.12+ 安装包
    *.ipk                      24.10 及更早安装包
    openxplay-receiver-*       接收端裸二进制（手动部署时用）
    openxplay-relay-*          输出端裸二进制（手动部署时用）
  x86_64/                      x86 软路由，文件同上
config/
  diplay-reference.txt         完整配置模板 + 中文注释（推荐）
```

安装包内已含三个程序：`openxplay-receiver`（接收端主进程）、`oxp-relay`（输出端）、`oxp-relay-wrapper`（注入 DLNA 参数）。裸二进制仅供不想用包管理器、手工部署的场景。

## 部署步骤

1. **安装**对应架构和格式的安装包（见上）。
2. **放 MFi 证书**（不随包分发，需自备有效的 Apple MFi 证书）：
   ```sh
   mkdir -p /etc/diplay/identity
   # 把 identity.pk8 和 certificate.p7b 放到该目录
   chmod 0700 /etc/diplay/identity
   chmod 0600 /etc/diplay/identity/*
   ```
3. **改配置**：`vi /etc/config/diplay`，或把 `config/diplay-reference.txt` 拷过去。
   - 有蓝牙 → `option transport 'wireless'`（默认）
   - **无蓝牙** → 必须改成 `option transport 'usb'`，iPhone 用数据线插路由器 USB 口
4. **启动**：
   ```sh
   /etc/init.d/diplay-cast enable
   /etc/init.d/diplay-cast start
   ```
5. **使用**：iPhone 连接（首次需亮屏解锁配对）→ 车机打开 DLNA 接收 → 应自动发现并推流。

排错：`logread -f | grep -iE 'oxp|diplay|carplay'`

## 内存要求

推荐 **256 MB RAM 起步，512 MB 流畅**；128 MB 勉强可跑但建议关闭无关服务。720p30 下接收端 + 输出端双进程约新增 45–80 MB。Flash 建议 16 MB 起步（安装包 4.6–4.9 MB）。

## 版本与来源

- 版本：**v0.1.2**（首个同时提供 ipk + apk 的版本）
- 编译源码与 CI：[OpenXPlay-DiPlay-Cast](https://github.com/563617356/OpenXPlay-DiPlay-Cast)
- 本仓库只存放编译产物和配置，不存放源码
- 上游依赖遵循 AGPL-3.0，详见源码仓库的 LICENSE / NOTICE / THIRD-PARTY-LICENSES

## 已知限制

- 车机只做 DLNA 接收端（DMR）；所有转换都在路由器上完成
- apk 内部架构标记为 `aarch64_generic`，与 `aarch64_cortex-a53` ABI 兼容，`apk add --allow-untrusted` 可正常安装
- 视频经过协议转换，会有约 200–500 ms 额外延迟
