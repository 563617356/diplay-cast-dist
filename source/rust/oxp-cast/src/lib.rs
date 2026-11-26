// SPDX-FileCopyrightText: 2026 OpenXPlay.org
// SPDX-License-Identifier: AGPL-3.0-only

//! DLNA/UPnP cast output for OpenXPlay Stream.
//!
//! Ported from `kotlin/cast` to Rust with the standard library only on the
//! network side, so it cross-compiles cleanly to `aarch64-unknown-linux-musl`
//! for headless Linux/OpenWrt relays. The MPEG-TS muxer is reused from
//! `oxp-media`; this crate adds SSDP discovery, AVTransport SOAP control and
//! an HTTP stream server.

#![forbid(unsafe_code)]

mod http;
mod model;
mod soap;
mod ssdp;
mod stream;
mod timeline;
mod xml;

pub use model::*;

use oxp_core::{
    model::{
        AudioCodec, AudioFormat, AudioPacket, AudioRole, ControlEvent, DecoderSupport,
        DisplayInfo, FrameRatePreference, LatencyClass, Origin, OutputCapabilities, OutputHost,
        OutputStats, SessionConfig, SessionPhase, SessionState, VideoFormat, VideoFrame,
    },
    ProjectionOutput, Result,
};
use oxp_media::ts::TsOptions;
use oxp_media::{Adts, AudioSpecificConfig, AvcConfig, HevcConfig, TsMuxer, VideoCodec};
use soap::soap_action;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Main screen id used across OpenXPlay Stream bindings.
const MAIN_SCREEN: u32 = 0;
const OUTPUT_ID: &str = "cast";

impl CastListener for () {}

/// Mutable session state shared between callbacks and worker threads.
struct Inner {
    open: bool,
    session: Option<i64>,
    video_format: Option<Arc<VideoFormat>>,
    audio_format: Option<Arc<AudioFormat>>,
    generation: u64,
    timeline: Option<timeline::Timeline>,
    renderer: Option<CastRenderer>,
    state: CastState,
    stats: CastStatistics,
}

type Shared = Arc<(Mutex<Inner>, Condvar)>;

/// A DLNA cast output. Register it with a `Hub` via `hub.add_output(output)`,
/// exactly like `WebOutput` or `ClinkOutput`.
pub struct CastOutput {
    config: CastConfig,
    listener: Arc<dyn CastListener>,
    inner: Shared,
    active: Arc<AtomicBool>,
    server: Arc<stream::StreamServer>,
}

impl CastOutput {
    pub fn new(config: CastConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self::with_listener(config, Arc::new(())))
    }

    pub fn with_listener(config: CastConfig, listener: Arc<dyn CastListener>) -> Self {
        let port = config.port;
        Self {
            config,
            listener,
            inner: Arc::new((
                Mutex::new(Inner {
                    open: false,
                    session: None,
                    video_format: None,
                    audio_format: None,
                    generation: 0,
                    timeline: None,
                    renderer: None,
                    state: CastState::Ended,
                    stats: CastStatistics::default(),
                }),
                Condvar::new(),
            )),
            active: Arc::new(AtomicBool::new(false)),
            server: Arc::new(stream::StreamServer::new(port)),
        }
    }

    fn set_state(&self, state: CastState) {
        let renderer = {
            let (lock, _) = &*self.inner;
            let mut inner = lock.lock().unwrap();
            if inner.state == state {
                return;
            }
            inner.state = state;
            inner.renderer.clone()
        };
        self.listener.state(state, renderer.as_ref());
    }

    fn reset_generation(&self, format: Option<Arc<VideoFormat>>, audio: bool) {
        let (lock, _) = &*self.inner;
        let mut inner = lock.lock().unwrap();
        inner.generation += 1;
        inner.video_format = format;
        inner.timeline = inner
            .video_format
            .as_ref()
            .map(|_| timeline::Timeline::new(audio && self.config.audio));
        let generation = inner.generation;
        drop(inner);
        self.server.reset(generation);
    }

    /// Controller: discover renderer, configure AVTransport, issue Play.
    fn spawn_control(&self, _host: Arc<dyn OutputHost>) {
        let active = self.active.clone();
        let config = self.config.clone();
        let listener = self.listener.clone();
        let server = self.server.clone();
        let shared = self.inner.clone();

        thread::Builder::new()
            .name("cast-control".into())
            .spawn(move || {
                let mut search_round = 0u32;
                while active.load(Ordering::Acquire) {
                    let discovery = match ssdp::discover(&config) {
                        Ok(d) => d,
                        Err(e) => {
                            listener.diagnostic(&format!("discovery_failed reached=ssdp {}", e));
                            if sleep_or_stop(&active, 2000) {
                                return;
                            }
                            continue;
                        }
                    };
                    let found = if config.renderer_udn.is_empty() {
                        discovery.renderers.first().cloned()
                    } else {
                        discovery
                            .renderers
                            .iter()
                            .find(|r| r.renderer.udn == config.renderer_udn)
                            .cloned()
                    };
                    let Some(found) = found else {
                        if discovery.without_transport.contains(&config.renderer_udn) {
                            listener.failure(&CastFailure {
                                code: "renderer_no_avtransport".into(),
                                detail: "Selected renderer has no AVTransport service".into(),
                            });
                        }
                        search_round += 1;
                        if search_round >= 3 {
                            listener.failure(&CastFailure {
                                code: "renderer_not_found".into(),
                                detail: "Renderer not found after three searches".into(),
                            });
                        }
                        if sleep_or_stop(&active, 2000) {
                            return;
                        }
                        continue;
                    };
                    search_round = 0;
                    {
                        let (lock, _) = &*shared;
                        lock.lock().unwrap().renderer = Some(found.renderer.clone());
                    }
                    listener.state(CastState::WaitingForVideo, Some(&found.renderer));

                    // Wait for a keyframe-primed stream before telling the TV to play.
                    let (lock, cv) = &*shared;
                    loop {
                        if !active.load(Ordering::Acquire) {
                            return;
                        }
                        let generation = lock.lock().unwrap().generation;
                        if server.ready(generation) {
                            break;
                        }
                        let (guard, _) = cv.wait_timeout(lock.lock().unwrap(), Duration::from_millis(100)).unwrap();
                        drop(guard);
                    }

                    let local = local_route_ip(&found.renderer.address);
                    let url = server.url(local);
                    let _ = soap_action(&found.transport, "Stop", &[], Duration::from_millis(2000));
                    let meta = soap::metadata(&config.title, &url);
                    let uri_args = [
                        ("InstanceID", "0".to_string()),
                        ("CurrentURI", url),
                        ("CurrentURIMetaData", meta),
                    ];
                    if let Err(e) = soap_action(
                        &found.transport,
                        "SetAVTransportURI",
                        &uri_args,
                        Duration::from_millis(7000),
                    ) {
                        listener.diagnostic(&format!("soap_failed action=SetAVTransportURI {:?}", e));
                        if sleep_or_stop(&active, 2000) {
                            return;
                        }
                        continue;
                    }
                    if let Err(e) = soap_action(
                        &found.transport,
                        "Play",
                        &[("InstanceID", "0".to_string())],
                        Duration::from_millis(7000),
                    ) {
                        listener.diagnostic(&format!("soap_failed action=Play {:?}", e));
                        if sleep_or_stop(&active, 2000) {
                            return;
                        }
                        continue;
                    }
                    listener.state(CastState::Playing, Some(&found.renderer));

                    // Poll transport state so a renderer stopping playback is detected.
                    while active.load(Ordering::Acquire) {
                        if sleep_or_stop(&active, 10_000) {
                            return;
                        }
                        match soap_action(
                            &found.transport,
                            "GetTransportInfo",
                            &[("InstanceID", "0".to_string())],
                            Duration::from_millis(3000),
                        ) {
                            Ok(args) => {
                                let status = args.get("CurrentTransportState").map(String::as_str).unwrap_or("");
                                let (lock, _) = &*shared;
                                lock.lock().unwrap().stats.renderer_state = Some(status.to_string());
                                if status == "STOPPED" {
                                    listener.state(CastState::StoppedByRenderer, Some(&found.renderer));
                                    break;
                                }
                            }
                            Err(e) => listener.diagnostic(&format!("soap_failed action=GetTransportInfo {:?}", e)),
                        }
                    }
                    // Re-discover on next loop iteration.
                    listener.state(CastState::Searching, None);
                }
            })
            .ok();
    }

    /// Media worker: schedule events and write MPEG-TS to the HTTP server.
    /// TS packets for one access unit are accumulated into one chunk so a
    /// keyframe chunk boundary is preserved (matches Kotlin CastMediaMuxer).
    fn spawn_media(&self, host: Arc<dyn OutputHost>) {
        let active = self.active.clone();
        let config = self.config.clone();
        let listener = self.listener.clone();
        let server = self.server.clone();
        let shared = self.inner.clone();

        thread::Builder::new()
            .name("cast-media".into())
            .spawn(move || {
                let aac = match AudioSpecificConfig::new(2, 48_000, 2) {
                    Ok(c) => c,
                    Err(e) => {
                        listener.diagnostic(&format!("audio_config_failed {}", e));
                        return;
                    }
                };
                let mut last_generation = u64::MAX;
                let mut muxer: Option<TsMuxer<Box<dyn FnMut(&[u8; 188])>>> = None;
                // Per-access-unit TS packet buffer, shared with the muxer sink.
                let au_buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::with_capacity(16 * 1024)));
                let mut last_report = Instant::now();

                while active.load(Ordering::Acquire) {
                    let (generation, format, audio_enabled, keyframe_request, events) = {
                        let (lock, _) = &*shared;
                        let mut inner = lock.lock().unwrap();
                        let generation = inner.generation;
                        let format = inner.video_format.clone();
                        let audio = inner.audio_format.is_some() && config.audio;
                        let request = server.needs_keyframe()
                            || inner.timeline.as_ref().is_some_and(|t| t.request_keyframe);
                        if let Some(t) = inner.timeline.as_mut() {
                            t.request_keyframe = false;
                        }
                        let events = inner
                            .timeline
                            .as_mut()
                            .map(|t| t.take(Instant::now()))
                            .unwrap_or_default();
                        (generation, format, audio, request, events)
                    };

                    if keyframe_request {
                        host.control(ControlEvent::KeyFrameRequest {
                            screen: MAIN_SCREEN,
                            origin: Origin {
                                output_id: OUTPUT_ID.into(),
                                client_id: String::new(),
                            },
                        });
                    }

                    if generation != last_generation {
                        last_generation = generation;
                        au_buffer.lock().unwrap().clear();
                        muxer = format.as_ref().and_then(|f| {
                            let buffer = au_buffer.clone();
                            let srv = server.clone();
                            let gen = generation;
                            let sink: Box<dyn FnMut(&[u8; 188])> = Box::new(move |packet: &[u8; 188]| {
                                buffer.lock().unwrap().extend_from_slice(packet);
                                let _ = &srv;
                                let _ = gen;
                            });
                            TsMuxer::with_options(
                                sink,
                                TsOptions {
                                    audio_enabled,
                                    video_codec: f.codec,
                                    ..TsOptions::default()
                                },
                            )
                            .ok()
                        });
                    }

                    let Some(muxer) = muxer.as_mut() else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    };

                    for event in events {
                        let (is_video, keyframe, silent) = match &event {
                            timeline::MediaEvent::Video { frame, .. } => (true, frame.key_frame, false),
                            timeline::MediaEvent::Audio { silent, .. } => (false, false, *silent),
                        };
                        au_buffer.lock().unwrap().clear();

                        let write_ok = match event {
                            timeline::MediaEvent::Video { frame, pts_us } => {
                                let config_bytes = format
                                    .as_ref()
                                    .map(|f| f.config.clone())
                                    .unwrap_or_default();
                                match format.as_ref().map(|f| f.codec) {
                                    Some(VideoCodec::H264) => match AvcConfig::parse(&config_bytes) {
                                        Ok(cfg) => muxer
                                            .write_video(&frame.data, &cfg, pts_us, frame.key_frame)
                                            .is_ok(),
                                        Err(_) => false,
                                    },
                                    Some(VideoCodec::H265) => match HevcConfig::parse(&config_bytes) {
                                        Ok(cfg) => muxer
                                            .write_video(&frame.data, &cfg, pts_us, frame.key_frame)
                                            .is_ok(),
                                        Err(_) => false,
                                    },
                                    None => false,
                                }
                            }
                            timeline::MediaEvent::Audio { data, pts_us, .. } => {
                                match Adts::build_header(&aac, data.len()) {
                                    Ok(h) => {
                                        let mut frame = Vec::with_capacity(data.len() + 7);
                                        frame.extend_from_slice(&h);
                                        frame.extend_from_slice(&data);
                                        muxer.write_audio(&frame, pts_us).is_ok()
                                    }
                                    Err(_) => false,
                                }
                            }
                        };

                        let chunk = {
                            let mut buffer = au_buffer.lock().unwrap();
                            if buffer.is_empty() {
                                Vec::new()
                            } else {
                                std::mem::take(&mut *buffer)
                            }
                        };
                        if !chunk.is_empty() {
                            server.publish(
                                generation,
                                stream::Chunk {
                                    bytes: Arc::from(chunk),
                                    keyframe_start: is_video && keyframe,
                                    at: Instant::now(),
                                },
                            );
                        }

                        let (lock, _) = &*shared;
                        let mut inner = lock.lock().unwrap();
                        if !write_ok && is_video {
                            inner.stats.dropped_frames += 1;
                            if let Some(t) = inner.timeline.as_mut() {
                                t.request_keyframe = true;
                            }
                        } else if is_video {
                            inner.stats.video_frames += 1;
                        } else {
                            inner.stats.audio_frames += 1;
                            if silent {
                                inner.stats.silent_frames += 1;
                            }
                        }
                    }

                    if last_report.elapsed() >= Duration::from_secs(1) {
                        last_report = Instant::now();
                        let (lock, _) = &*shared;
                        let mut inner = lock.lock().unwrap();
                        inner.stats.clients = server.client_count();
                        inner.stats.bytes_sent = server.bytes_sent();
                        inner.stats.renderer_state = inner.stats.renderer_state.clone();
                        listener.statistics(&inner.stats);
                        host.stats(OutputStats {
                            session: inner.session,
                            origin: Origin {
                                output_id: OUTPUT_ID.into(),
                                client_id: String::new(),
                            },
                            screen: MAIN_SCREEN,
                            decode_time_us: None,
                            decode_queue: 0,
                            dropped: inner.stats.dropped_frames,
                            rendered: inner.stats.video_frames,
                            end_to_end_latency_us: None,
                            audio_buffered_us: None,
                            audio_underruns: 0,
                        });
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
            .ok();
    }
}

fn sleep_or_stop(active: &AtomicBool, millis: u64) -> bool {
    let step = 100u64;
    let mut elapsed = 0;
    while elapsed < millis {
        if !active.load(Ordering::Acquire) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(step.min(millis - elapsed)));
        elapsed += step;
    }
    !active.load(Ordering::Acquire)
}

fn local_route_ip(_renderer: &Ipv4Addr) -> Ipv4Addr {
    ssdp::local_addresses(None)
        .into_iter()
        .next()
        .unwrap_or(Ipv4Addr::LOCALHOST)
}

fn same_format(a: Option<&VideoFormat>, b: Option<&VideoFormat>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.codec == y.codec && x.codec_string == y.codec_string && x.config == y.config
        }
        _ => false,
    }
}

impl ProjectionOutput for CastOutput {
    fn id(&self) -> &str {
        OUTPUT_ID
    }

    fn open(&self, host: Arc<dyn OutputHost>) -> Result<()> {
        {
            let (lock, _) = &*self.inner;
            let mut inner = lock.lock().unwrap();
            if inner.open {
                return Err("cast output already open".into());
            }
            inner.open = true;
        }
        self.active.store(true, Ordering::Release);

        host.capabilities(OutputCapabilities {
            prefers_h264: true,
            latency: LatencyClass::Buffered,
            decoders: vec![
                DecoderSupport {
                    codec_string: "avc1.64002a".into(),
                    hardware: true,
                },
                DecoderSupport {
                    codec_string: "hvc1.1.6.L150".into(),
                    hardware: true,
                },
            ],
            displays: {
                let mut map = std::collections::BTreeMap::new();
                map.insert(
                    MAIN_SCREEN,
                    DisplayInfo {
                        width_px: self.config.width,
                        height_px: self.config.height,
                        density_dpi: 160,
                        refresh_hz: self.config.fps as f64,
                        frame_rate: if self.config.fps == 60 {
                            FrameRatePreference::Fps60
                        } else {
                            FrameRatePreference::Fps30
                        },
                        ..DisplayInfo::default()
                    },
                );
                map
            },
            touch: 0,
            keys: Default::default(),
            audio: if self.config.audio {
                [AudioCodec::AacLc].into_iter().collect()
            } else {
                Default::default()
            },
            microphone: false,
        });

        if let Err(e) = self.server.bind() {
            self.active.store(false, Ordering::Release);
            return Err(e.into());
        }

        let accept_active = self.active.clone();
        let server = self.server.clone();
        thread::Builder::new()
            .name("cast-http".into())
            .spawn(move || {
                let noop: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
                server.accept(accept_active, noop);
            })
            .ok();

        self.spawn_control(host.clone());
        self.spawn_media(host);
        self.set_state(CastState::Searching);
        Ok(())
    }

    fn on_session(&self, state: &SessionState, _config: &SessionConfig) {
        let (lock, _) = &*self.inner;
        let mut inner = lock.lock().unwrap();
        if !inner.open {
            return;
        }
        if inner.session != Some(state.id) || state.phase == SessionPhase::Ended {
            inner.session = if state.phase == SessionPhase::Ended {
                None
            } else {
                Some(state.id)
            };
            let format = if state.phase == SessionPhase::Ended {
                None
            } else {
                inner.video_format.clone()
            };
            let audio = inner.audio_format.is_some();
            let generation = inner.generation + 1;
            inner.generation = generation;
            inner.video_format = format.clone();
            inner.timeline = format.map(|_| timeline::Timeline::new(audio && self.config.audio));
            drop(inner);
            self.server.reset(generation);
        }
    }

    fn on_video_format(&self, screen: u32, format: Option<Arc<VideoFormat>>) {
        if screen != MAIN_SCREEN {
            return;
        }
        let changed = {
            let (lock, _) = &*self.inner;
            let mut inner = lock.lock().unwrap();
            if !inner.open {
                return;
            }
            let changed = !same_format(inner.video_format.as_deref(), format.as_deref());
            inner.video_format = format.clone();
            changed
        };
        if changed {
            let audio = {
                let (lock, _) = &*self.inner;
                lock.lock().unwrap().audio_format.is_some()
            };
            self.reset_generation(format, audio);
        }
    }

    fn on_video_frame(&self, frame: VideoFrame) {
        if frame.screen != MAIN_SCREEN {
            return;
        }
        let (lock, _) = &*self.inner;
        let mut inner = lock.lock().unwrap();
        if !inner.open || inner.session != Some(frame.session) {
            return;
        }
        if let Some(t) = inner.timeline.as_mut() {
            t.video(frame);
        }
    }

    fn on_audio_format(&self, stream: u32, format: Option<Arc<AudioFormat>>) {
        if !self.config.audio || stream != 0 {
            return;
        }
        let is_media = format
            .as_ref()
            .is_some_and(|f| f.role == AudioRole::Media && f.codec == AudioCodec::AacLc);
        let (lock, _) = &*self.inner;
        let mut inner = lock.lock().unwrap();
        if !inner.open {
            return;
        }
        inner.audio_format = if is_media { format } else { None };
    }

    fn on_audio_packet(&self, packet: AudioPacket) {
        if !self.config.audio || packet.stream != 0 {
            return;
        }
        let (lock, _) = &*self.inner;
        let mut inner = lock.lock().unwrap();
        if !inner.open || inner.session != Some(packet.session) || inner.audio_format.is_none() {
            return;
        }
        if let Some(t) = inner.timeline.as_mut() {
            t.audio(packet);
        }
    }

    fn close(&self) {
        self.active.store(false, Ordering::Release);
        self.server.close();
        self.set_state(CastState::Ended);
        let (lock, _) = &*self.inner;
        let mut inner = lock.lock().unwrap();
        inner.open = false;
        inner.session = None;
    }
}
