// SPDX-FileCopyrightText: 2026 OpenXPlay.org
// SPDX-License-Identifier: AGPL-3.0-only

//! HTTP server that serves the live MPEG-TS stream to the DLNA renderer.
//! Ported from kotlin/cast StreamServer.kt: bounded keyframe cache, per-client
//! bounded queue, keyframe wait, DLNA headers, client disconnect reporting.

use crate::model::{DLNA_FEATURES, USER_AGENT};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Chunk {
    pub bytes: Arc<[u8]>,
    pub keyframe_start: bool,
    pub at: Instant,
}

struct Client {
    queue: VecDeque<Arc<[u8]>>,
    bytes: usize,
    closed: bool,
    awaiting_keyframe: bool,
    end_reason: Option<String>,
}

impl Client {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            bytes: 0,
            closed: false,
            awaiting_keyframe: true,
            end_reason: None,
        }
    }
}

struct Shared {
    generation: u64,
    ready_generation: i64,
    cache: VecDeque<Chunk>,
    cache_bytes: usize,
    clients: HashMap<u64, Client>,
    next_client: u64,
    ended: bool,
    port: u16,
    token: String,
}

/// Live TS stream over HTTP. `publish` is non-blocking; writer threads block
/// on their own bounded queues.
pub struct StreamServer {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    need_keyframe: Arc<AtomicBool>,
    bytes_sent: Arc<AtomicU64>,
    overflows: Arc<AtomicU64>,
    listener: Mutex<Option<TcpListener>>,
    max_clients: usize,
}

impl StreamServer {
    pub fn new(requested_port: u16) -> Self {
        let shared = Shared {
            generation: 0,
            ready_generation: -1,
            cache: VecDeque::new(),
            cache_bytes: 0,
            clients: HashMap::new(),
            next_client: 1,
            ended: false,
            port: requested_port,
            token: hex_token(),
        };
        Self {
            shared: Arc::new((Mutex::new(shared), Condvar::new())),
            need_keyframe: Arc::new(AtomicBool::new(false)),
            bytes_sent: Arc::new(AtomicU64::new(0)),
            overflows: Arc::new(AtomicU64::new(0)),
            listener: Mutex::new(None),
            max_clients: 8,
        }
    }

    pub fn bind(&self) -> Result<(), std::io::Error> {
        let requested = self.shared.0.lock().unwrap().port;
        let listener = TcpListener::bind(("0.0.0.0", requested))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let (lock, cv) = &*self.shared;
        let mut shared = lock.lock().unwrap();
        shared.port = port;
        drop(shared);
        cv.notify_all();
        *self.listener.lock().unwrap() = Some(listener);
        Ok(())
    }

    pub fn port(&self) -> u16 {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap().port
    }

    pub fn path(&self) -> String {
        let (lock, _) = &*self.shared;
        let token = lock.lock().unwrap().token.clone();
        format!("/cast/{}/{}.ts", token, lock.lock().unwrap().generation)
    }

    pub fn url(&self, local: std::net::Ipv4Addr) -> String {
        format!("http://{}:{}{}", local, self.port(), self.path())
    }

    pub fn reset(&self, generation: u64) {
        let (lock, cv) = &*self.shared;
        let mut shared = lock.lock().unwrap();
        shared.generation = generation;
        shared.ready_generation = -1;
        shared.cache.clear();
        shared.cache_bytes = 0;
        for client in shared.clients.values_mut() {
            if !client.awaiting_keyframe {
                client.end_reason = Some("by=local cause=replaced".into());
                client.closed = true;
            }
        }
        self.need_keyframe.store(false, Ordering::Release);
        cv.notify_all();
    }

    pub fn publish(&self, generation: u64, chunk: Chunk) {
        let (lock, cv) = &*self.shared;
        let mut shared = lock.lock().unwrap();
        if shared.ended || shared.generation != generation {
            return;
        }
        if chunk.keyframe_start {
            shared.cache.clear();
            shared.cache_bytes = 0;
            shared.ready_generation = generation as i64;
        }
        if chunk.keyframe_start || !shared.cache.is_empty() {
            shared.cache_bytes += chunk.bytes.len();
            shared.cache.push_back(chunk.clone());
            // Keep cache bounded (about 4 MB) and no older than ~10 s.
            while shared.cache_bytes > 4 * 1024 * 1024
                || shared
                    .cache
                    .front()
                    .is_some_and(|c| c.at.elapsed() > Duration::from_secs(10))
            {
                if let Some(old) = shared.cache.pop_front() {
                    shared.cache_bytes = shared.cache_bytes.saturating_sub(old.bytes.len());
                } else {
                    break;
                }
            }
        }
        for client in shared.clients.values_mut() {
            if !client.awaiting_keyframe && !client.closed && client.bytes + chunk.bytes.len() <= 8 * 1024 * 1024 {
                client.queue.push_back(chunk.bytes.clone());
                client.bytes += chunk.bytes.len();
            } else if !client.awaiting_keyframe && !client.closed {
                self.overflows.fetch_add(1, Ordering::Relaxed);
            }
        }
        cv.notify_all();
    }

    pub fn ready(&self, generation: u64) -> bool {
        let (lock, _) = &*self.shared;
        let shared = lock.lock().unwrap();
        !shared.ended && shared.ready_generation == generation as i64
    }

    pub fn needs_keyframe(&self) -> bool {
        self.need_keyframe.swap(false, Ordering::AcqRel)
    }

    pub fn client_count(&self) -> usize {
        let (lock, _) = &*self.shared;
        let shared = lock.lock().unwrap();
        shared
            .clients
            .values()
            .filter(|c| !c.closed && !c.awaiting_keyframe)
            .count()
    }

    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    /// Accept loop. Calls `on_disconnect` when a streaming client goes away.
    pub fn accept(&self, active: Arc<AtomicBool>, on_disconnect: Arc<dyn Fn() + Send + Sync>) {
        let listener_guard = self.listener.lock().unwrap();
        let Some(listener) = listener_guard.as_ref() else {
            return;
        };
        while active.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    let id = {
                        let (lock, _) = &*self.shared;
                        let mut shared = lock.lock().unwrap();
                        if shared.clients.len() >= self.max_clients {
                            drop(shared);
                            let _ = stream.shutdown(Shutdown::Both);
                            continue;
                        }
                        let id = shared.next_client;
                        shared.next_client += 1;
                        shared.clients.insert(id, Client::new());
                        id
                    };
                    let shared = self.shared.clone();
                    let need_keyframe = self.need_keyframe.clone();
                    let bytes_sent = self.bytes_sent.clone();
                    let path = self.path();
                    let active = active.clone();
                    let on_disconnect = on_disconnect.clone();
                    std::thread::Builder::new()
                        .name("cast-http-client".into())
                        .spawn(move || {
                            serve_client(stream, addr, id, shared, need_keyframe, bytes_sent, path, active, on_disconnect);
                        })
                        .ok();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    }

    pub fn close(&self) {
        let (lock, cv) = &*self.shared;
        let mut shared = lock.lock().unwrap();
        shared.ended = true;
        for client in shared.clients.values_mut() {
            client.end_reason = Some("by=local cause=stopped".into());
            client.closed = true;
        }
        cv.notify_all();
    }
}

fn hex_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        ^ (std::process::id() as u128);
    let mut x = seed as u64 ^ 0x9e3779b97f4a7c15;
    let mut out = String::with_capacity(16);
    for _ in 0..8 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push_str(&format!("{:02x}", x & 0xff));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn serve_client(
    mut stream: TcpStream,
    _addr: SocketAddr,
    id: u64,
    shared: Arc<(Mutex<Shared>, Condvar)>,
    need_keyframe: Arc<AtomicBool>,
    bytes_sent: Arc<AtomicU64>,
    expected_path: String,
    active: Arc<AtomicBool>,
    on_disconnect: Arc<dyn Fn() + Send + Sync>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut head = [0u8; 8192];
    let n = match stream.read(&mut head) {
        Ok(n) => n,
        Err(_) => {
            remove_client(&shared, id);
            return;
        }
    };
    let request = String::from_utf8_lossy(&head[..n]);
    let first = request.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    if method != "GET" && method != "HEAD" {
        let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n");
        remove_client(&shared, id);
        return;
    }
    if path != expected_path {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        remove_client(&shared, id);
        return;
    }

    // Wait for a fresh keyframe header to prime this client.
    let (lock, cv) = &*shared;
    let prime: Vec<Arc<[u8]>> = {
        let mut state = lock.lock().unwrap();
        need_keyframe.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_millis(2500);
        while !state.ended && state.cache.is_empty() && Instant::now() < deadline {
            let (s, timeout) = cv.wait_timeout(state, Duration::from_millis(100)).unwrap();
            state = s;
            if timeout.timed_out() && state.cache.is_empty() {
                need_keyframe.store(true, Ordering::Release);
            }
        }
        if state.ended || state.cache.is_empty() {
            drop(state);
            let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n");
            remove_client(&shared, id);
            return;
        }
        if let Some(client) = state.clients.get_mut(&id) {
            client.awaiting_keyframe = false;
        }
        state.cache.iter().map(|c| c.bytes.clone()).collect()
    };

    let header = format!(
        "HTTP/1.1 200 OK\r\n\
Content-Type: video/mp2t\r\n\
Cache-Control: no-cache\r\n\
Connection: close\r\n\
transferMode.dlna.org: Streaming\r\n\
contentFeatures.dlna.org: {}\r\n\
Server: {}\r\n\
\r\n",
        DLNA_FEATURES, USER_AGENT
    );
    if stream.write_all(header.as_bytes()).is_err() {
        remove_client(&shared, id);
        return;
    }
    let started = method == "GET";
    if !started {
        remove_client(&shared, id);
        return;
    }

    for chunk in prime {
        if stream.write_all(&chunk).is_err() {
            remove_client(&shared, id);
            on_disconnect();
            return;
        }
        bytes_sent.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }

    loop {
        if !active.load(Ordering::Acquire) {
            break;
        }
        let next = {
            let mut state = lock.lock().unwrap();
            let client = match state.clients.get_mut(&id) {
                Some(c) if !c.closed => c,
                _ => break,
            };
            if let Some(bytes) = client.queue.pop_front() {
                client.bytes -= bytes.len();
                Some(bytes)
            } else {
                let (s, _) = cv.wait_timeout(state, Duration::from_millis(100)).unwrap();
                state = s;
                None
            }
        };
        if let Some(bytes) = next {
            if stream.write_all(&bytes).is_err() {
                break;
            }
            bytes_sent.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
    }
    remove_client(&shared, id);
    on_disconnect();
}

fn remove_client(shared: &Arc<(Mutex<Shared>, Condvar)>, id: u64) {
    let (lock, _) = &**shared;
    let mut state = lock.lock().unwrap();
    state.clients.remove(&id);
}
