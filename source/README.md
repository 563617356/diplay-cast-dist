# DiPlay Cast — OpenWrt 无头 CarPlay → DLNA 接收端

> This is a fork of [OpenXPlay-Stream](https://github.com/OpenXPlay/OpenXPlay-Stream) that adds a **DLNA/UPnP cast output** (`rust/oxp-cast`), so a headless OpenWrt box can receive CarPlay and stream the picture to a head unit that **only has a DLNA receiver** (no browser). [中文说明](README-zh.md) · [OpenWrt 打包文档](openwrt/README.md)

## What this fork adds

| Path | Change |
| --- | --- |
| `rust/oxp-cast/` | **New crate**: DLNA cast output. SSDP discovery → SOAP AVTransport → HTTP `video/mp2t` stream. Ported from the upstream pure-JVM `kotlin/cast`, reusing `oxp-media`'s MPEG-TS muxer. Implements the standard `ProjectionOutput` trait. |
| `rust/oxp-bin/src/bin/oxp-relay.rs` | Added `--cast` family of flags to mount the cast output into the relay hub. |
| `relay/rust/` | Vendored [OpenXPlay-Relay](https://github.com/OpenXPlay/OpenXPlay-Relay) `direlay-linux`: the receiver that talks to the iPhone (iAP2/MFi) and supervises the output process. Since v0.1.1 it ships in the ipk — everything runs on the router. |
| `openwrt/` | OpenWrt package skeleton: procd init, UCI config, LuCI page, musl cross-build + ipk pack scripts. |
| `.github/workflows/build.yml` | **Cloud build**: tests `oxp-cast`, cross-compiles both receiver and output relay for aarch64 + x86_64, packages `.ipk`, publishes a Release on `v*` tags. |

## Download prebuilt ipk

Go to [Releases](../../releases). Pick the ipk for your router arch:

- `*_aarch64_cortex-a53.ipk` — ARMv8 routers (N1 / R4S / MT6000)
- `*_x86_64.ipk` — x86 soft routers

**MFi identity files are NOT bundled.** After install, copy your own `identity.pk8` / `certificate.p7b` to `/etc/diplay/identity/` (mode 600), then configure via `/etc/config/diplay` or LuCI → Services → DiPlay Cast. See [openwrt/README.md](openwrt/README.md).

## Build it yourself

Push a `v*` tag (or trigger the workflow manually) and GitHub Actions produces both ipks. To build locally on Linux/macOS:

```bash
cd openwrt
TARGET=aarch64-unknown-linux-musl bash build-musl.sh
TARGET=aarch64-unknown-linux-musl bash pack-ipk.sh
```

## Credit & license

Derived from [OpenXPlay-Stream](https://github.com/OpenXPlay/OpenXPlay-Stream), AGPL-3.0-only. Retains the "Powered by OpenXPlay." attribution.

---

<p align="center"><b>English</b> · <a href="README-zh.md">简体中文</a></p>

# OpenXPlay-Stream (upstream)

The output side of an [OpenXPlay](https://github.com/OpenXPlay/OpenXPlay) relay. Stream takes a projection session (video, audio, touch and keys) from the relay and delivers it to software a head unit already has, so nothing is installed on the head unit:

| Output | Head unit uses | Touch | Status |
| --- | --- | --- | --- |
| Web | Its browser: H.264 over a secure WebSocket, decoded with WebCodecs | Yes | In development |
| Carlink | Its Carlink app, over Wi-Fi | Yes | Experimental |
| Cast | Screen-casting receivers (DLNA) | No | Planned |

Every output talks only to the OXP-Stream model, either in process or over the Stream wire protocol. Inputs and outputs never depend on each other, so the same outputs serve every relay. The [OpenXPlay Relay](https://github.com/OpenXPlay/OpenXPlay-Relay) provides the CarPlay input.

## Layout

The model and the outputs are implemented three times, one per runtime. Shared protocol test vectors keep the three implementations in step.

| Directory | Language | Contents | Used by |
| --- | --- | --- | --- |
| [`kotlin/`](kotlin) | Kotlin/JVM (Gradle) | `core` (session model and hub), `wire` (wire codec), `web-server`, `clink` (Carlink output), `tools/` (replay, a development relay, a simulated head unit) | The Android relay app |
| [`rust/`](rust) | Rust (Cargo) | `oxp-core`, `oxp-wire`, `oxp-web`, `oxp-clink`, `oxp-media`, `oxp-link-linux` (Carlink Wi-Fi link through BlueZ and NetworkManager), `oxp-bin` (the `oxp-relay` output process) | The Linux relay |
| [`web/`](web) | TypeScript (Vite) | `receiver/` (the page the head-unit browser opens), `protocol/`, `shared/`, `probe/` (a decoder capability probe) | Served by both relays |
| [`spec/vectors/`](spec/vectors) | Data | Wire and web protocol test vectors | Tests in all three implementations |

## Build and test

| Part | Requirements | Commands |
| --- | --- | --- |
| Kotlin | JDK 17 | `cd kotlin && ./gradlew test` |
| Rust | Rust stable | `cd rust && cargo test --workspace` |
| Rust for aarch64 Linux | `rustup target add aarch64-unknown-linux-musl`, `llvm-tools`, clang | `rust/scripts/build-aarch64.sh` |
| Web receivers | Node.js 20.19+ or 22.12+ | `npm ci && npm test && npm run build` in `kotlin/web-receiver` and in `rust/web-receiver` (output in each `dist`) |

The Android relay packages `kotlin/web-receiver/dist` and the Linux relay `rust/web-receiver/dist`; build that receiver before building the relay. The two receivers are separate code.

## Contributing

Contributions are accepted under the [OpenXPlay Contributor License Agreement](https://github.com/OpenXPlay/OpenXPlay/blob/main/CLA.md). Accept it
once by replying on a pull request to any OpenXPlay repository; the CLA check then passes in all of
them.

## License

This project is licensed under the [GNU Affero General Public License v3.0 (AGPLv3)](./LICENSE),
version 3 only.

Additional terms under AGPLv3 Section 7 apply. Modified versions must preserve the attribution
notice `Powered by OpenXPlay.` in the appropriate legal notices and in any prominent about, legal,
footer, or attribution location presented by the user interface, together with a visible link to
the original project: <https://github.com/OpenXPlay/OpenXPlay-Stream>. See [NOTICE](./NOTICE).

If your organization's policies do not permit the use of AGPLv3-licensed software, or if you wish
to avoid the open-source obligations of AGPLv3, please contact us at:
[support@openxplay.org](mailto:support@openxplay.org)

CarPlay is a trademark of Apple Inc. OpenXPlay is an independent project and is not affiliated with,
endorsed or certified by Apple.
