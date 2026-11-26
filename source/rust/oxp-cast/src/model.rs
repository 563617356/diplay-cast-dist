// SPDX-FileCopyrightText: 2026 OpenXPlay.org
// SPDX-License-Identifier: AGPL-3.0-only

use std::net::Ipv4Addr;

/// User agent advertised in SSDP, HTTP and DLNA headers.
pub const USER_AGENT: &str = "OpenXPlay/0 UPnP/1.1 OpenXPlay-Stream/0";
/// DLNA transfer clues for the MPEG-TS stream served to the renderer.
pub const DLNA_FEATURES: &str =
    "DLNA.ORG_OP=00;DLNA.ORG_CI=0;DLNA.ORG_FLAGS=01700000000000000000000000000000";

pub const RENDERER_TYPE: &str = "urn:schemas-upnp-org:device:MediaRenderer:";
pub const AVTRANSPORT_TYPE: &str = "urn:schemas-upnp-org:service:AVTransport:";
pub const CONNECTION_TYPE: &str = "urn:schemas-upnp-org:service:ConnectionManager:";

/// A UPnP AV MediaRenderer discovered through SSDP.
#[derive(Clone, Debug)]
pub struct CastRenderer {
    pub udn: String,
    pub friendly_name: String,
    pub manufacturer: String,
    pub model_name: String,
    pub address: Ipv4Addr,
    pub location: String,
}

/// A service control endpoint exposed by a renderer.
#[derive(Clone, Debug)]
pub struct CastService {
    pub service_type: String,
    pub control: String,
}

#[derive(Clone, Debug)]
pub struct DiscoveredRenderer {
    pub renderer: CastRenderer,
    pub transport: CastService,
    pub connection: Option<CastService>,
}

#[derive(Clone, Debug, Default)]
pub struct DiscoveryResult {
    pub renderers: Vec<DiscoveredRenderer>,
    pub without_transport: Vec<String>,
}

/// Static configuration for a cast output instance.
#[derive(Clone, Debug)]
pub struct CastConfig {
    /// UDN of the renderer to use. Empty selects the first one found.
    pub renderer_udn: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub audio: bool,
    /// HTTP stream port. 0 picks an ephemeral port.
    pub port: u16,
    /// Title sent in DIDL-Lite metadata.
    pub title: String,
    /// SSDP M-SEARCH timeout in milliseconds.
    pub search_ms: u64,
    /// Optional network interface IPv4 for multicast (LAN selection).
    pub interface: Option<Ipv4Addr>,
}

impl Default for CastConfig {
    fn default() -> Self {
        Self {
            renderer_udn: String::new(),
            width: 1280,
            height: 720,
            fps: 30,
            audio: true,
            port: 0,
            title: "CarPlay".into(),
            search_ms: 3000,
            interface: None,
        }
    }
}

impl CastConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.width == 0 || self.height == 0 {
            return Err("Invalid cast size");
        }
        if !(1..=65535).contains(&self.fps) {
            return Err("Invalid cast fps");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastState {
    Searching,
    WaitingForVideo,
    Starting,
    Playing,
    Paused,
    StoppedByRenderer,
    Ended,
}

#[derive(Clone, Debug)]
pub struct CastFailure {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default)]
pub struct CastStatistics {
    pub clients: usize,
    pub video_frames: u64,
    pub audio_frames: u64,
    pub silent_frames: u64,
    pub bytes_sent: u64,
    pub dropped_frames: u64,
    pub overflows: u64,
    pub renderer_state: Option<String>,
}

pub trait CastListener: Send + Sync {
    fn state(&self, _state: CastState, _renderer: Option<&CastRenderer>) {}
    fn failure(&self, _failure: &CastFailure) {}
    fn diagnostic(&self, _message: &str) {}
    fn statistics(&self, _statistics: &CastStatistics) {}
}
