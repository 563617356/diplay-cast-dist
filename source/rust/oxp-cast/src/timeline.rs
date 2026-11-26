// SPDX-FileCopyrightText: 2026 OpenXPlay.org
// SPDX-License-Identifier: AGPL-3.0-only

//! Buffered audio/video scheduling: aligns video frames to a 48 kHz AAC
//! clock and inserts silence when audio is missing. Ported from the Kotlin
//! `CastTimeline`/`CastMediaMuxer` pipeline. The actual TS muxing is done by
//! `oxp_media::TsMuxer`; this module only decides event order and timestamps.

use oxp_core::model::{AudioPacket, VideoFrame};
use oxp_core::monotonic_nanos;
use std::time::Instant;

/// One silent AAC-LC frame (1024 samples) used to keep the renderer clock fed.
pub const SILENT_AAC: &[u8] = &[0x21, 0x10, 0x04, 0x60, 0x8c, 0x1c];

pub enum MediaEvent {
    Video { frame: VideoFrame, pts_us: i64 },
    Audio { data: Vec<u8>, silent: bool, pts_us: i64 },
}

struct PendingVideo {
    frame: VideoFrame,
    label: Option<i64>,
}

/// Pure scheduler. No muxing or I/O happens here.
pub struct Timeline {
    start: Instant,
    /// Monotonic clock origin, shared with VideoFrame.received_at_nanos.
    start_nanos: i64,
    audio_enabled: bool,
    video: std::collections::VecDeque<PendingVideo>,
    audio: std::collections::VecDeque<AudioPacket>,
    video_bytes: usize,
    waiting_keyframe: bool,
    previous_video: i64,
    emitted: i64,
    emitted_video: i64,
    sample_count: i64,
    pub dropped: u64,
    pub request_keyframe: bool,
}

impl Timeline {
    pub fn new(audio_enabled: bool) -> Self {
        Self {
            start: Instant::now(),
            start_nanos: monotonic_nanos(),
            audio_enabled,
            video: std::collections::VecDeque::new(),
            audio: std::collections::VecDeque::new(),
            video_bytes: 0,
            waiting_keyframe: true,
            previous_video: -1,
            emitted: 1_000_000,
            emitted_video: -1,
            sample_count: 0,
            dropped: 0,
            request_keyframe: true,
        }
    }

    pub fn video(&mut self, frame: VideoFrame) {
        if frame.data.is_empty() || frame.data.len() > 8 * 1024 * 1024 {
            self.dropped += 1;
            self.video.clear();
            self.video_bytes = 0;
            self.waiting_keyframe = true;
            self.request_keyframe = true;
            return;
        }
        if self.waiting_keyframe && !frame.key_frame {
            self.dropped += 1;
            self.request_keyframe = true;
            return;
        }
        if frame.key_frame {
            self.waiting_keyframe = false;
        }
        self.video_bytes += frame.data.len();
        self.video.push_back(PendingVideo {
            frame,
            label: None,
        });
        while self.video.len() > 60 || self.video_bytes > 8 * 1024 * 1024 {
            if let Some(dropped) = self.video.pop_front() {
                self.video_bytes = self.video_bytes.saturating_sub(dropped.frame.data.len());
                self.dropped += 1;
                self.waiting_keyframe = self.video.front().is_none();
                self.request_keyframe = true;
            } else {
                break;
            }
        }
    }

    pub fn audio(&mut self, packet: AudioPacket) {
        if !self.audio_enabled || packet.data.is_empty() || packet.data.len() > 8184 {
            return;
        }
        // Fourteen 1024-sample slots fit in 300 ms at 48 kHz.
        while self.audio.len() >= 14 {
            self.audio.pop_front();
            self.dropped += 1;
        }
        self.audio.push_back(packet);
    }

    pub fn take(&mut self, now: Instant) -> Vec<MediaEvent> {
        let elapsed_us = now.saturating_duration_since(self.start).as_micros().min(i64::MAX as u128) as i64;
        let horizon = 1_000_000 + elapsed_us - 200_000;
        for pending in self.video.iter_mut() {
            if pending.label.is_none() {
                let arrival = if pending.frame.received_at_nanos > 0 {
                    // Both timestamps share the process monotonic clock.
                    (pending.frame.received_at_nanos - self.start_nanos)
                        .max(0)
                        .min(elapsed_us)
                } else {
                    elapsed_us
                };
                let label = (1_000_000 + arrival).max(self.previous_video + 1000);
                pending.label = Some(label);
                self.previous_video = label;
            }
        }
        let mut events = Vec::new();
        for _ in 0..256 {
            let video_pts = self
                .video
                .front()
                .and_then(|p| p.label)
                .map(|p| p.max(self.emitted).max(self.emitted_video + 1000))
                .unwrap_or(i64::MAX);
            let audio_pts = if self.audio_enabled {
                1_000_000 + (self.sample_count * 1_000_000) / 48_000
            } else {
                i64::MAX
            };
            let next = video_pts.min(audio_pts);
            if next > horizon {
                break;
            }
            if video_pts <= audio_pts {
                let pending = self.video.pop_front().unwrap();
                self.video_bytes = self.video_bytes.saturating_sub(pending.frame.data.len());
                events.push(MediaEvent::Video {
                    frame: pending.frame,
                    pts_us: next,
                });
                self.emitted_video = next;
            } else {
                let live = self.audio.pop_front();
                let (data, silent) = match live {
                    Some(p) => (p.data.to_vec(), false),
                    None => (SILENT_AAC.to_vec(), true),
                };
                events.push(MediaEvent::Audio {
                    data,
                    silent,
                    pts_us: next,
                });
                self.sample_count += 1024;
            }
            self.emitted = next;
        }
        events
    }
}
