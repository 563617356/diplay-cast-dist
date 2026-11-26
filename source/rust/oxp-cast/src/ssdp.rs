// SPDX-FileCopyrightText: 2026 OpenXPlay.org
// SPDX-License-Identifier: AGPL-3.0-only

//! SSDP discovery of UPnP MediaRenderers, plus device description parsing.
//! Standard library only: raw UDP multicast and small HTTP GETs.

use crate::http::{http_get, HttpError};
use crate::model::*;
use crate::xml;
use std::collections::HashSet;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

const MULTICAST: (Ipv4Addr, u16) = (Ipv4Addr::new(239, 255, 255, 250), 1900);

struct SsdpReply {
    location: String,
    #[allow(dead_code)]
    usn: String,
    #[allow(dead_code)]
    st: String,
}

fn search_request(st: &str) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\n\
HOST: 239.255.255.250:1900\r\n\
MAN: \"ssdp:discover\"\r\n\
MX: 2\r\n\
USER-AGENT: {}\r\n\
ST: {}\r\n\
\r\n",
        USER_AGENT, st
    )
}

fn parse_reply(bytes: &[u8]) -> Option<SsdpReply> {
    let text = String::from_utf8_lossy(bytes);
    let normalized = text.replace("\r\n", "\n");
    let mut lines = normalized.split('\n');
    let status = lines.next()?;
    if !status.starts_with("HTTP/1.0 200") && !status.starts_with("HTTP/1.1 200") {
        return None;
    }
    let mut location = None;
    let mut usn = None;
    let mut st = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "location" => location = Some(value.trim().to_string()),
            "usn" => usn = Some(value.trim().to_string()),
            "st" => st = Some(value.trim().to_string()),
            _ => {}
        }
    }
    Some(SsdpReply {
        location: location?,
        usn: usn.unwrap_or_default(),
        st: st.unwrap_or_default(),
    })
}

/// Enumerate local IPv4 addresses. On Linux, parse /proc/net/fib_trie for
/// interface addresses. Falls back to a UDP-connect probe for the default route.
pub fn local_addresses(preferred: Option<Ipv4Addr>) -> Vec<Ipv4Addr> {
    if let Some(ip) = preferred {
        return vec![ip];
    }
    let mut result = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/proc/net/fib_trie") {
        let mut host_section = false;
        for line in text.lines() {
            if line.contains("|--") || line.contains("+--") {
                host_section = line.trim_end().ends_with("/32 host LOCAL");
                continue;
            }
            if host_section {
                let token = line.trim();
                if let Ok(ip) = token.parse::<Ipv4Addr>() {
                    if !ip.is_loopback() && !ip.is_unspecified() && !result.contains(&ip) {
                        result.push(ip);
                    }
                }
            }
        }
    }
    if result.is_empty() {
        if let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
            if socket.connect((Ipv4Addr::new(198, 51, 100, 8), 80)).is_ok() {
                if let Ok(SocketAddr::V4(addr)) = socket.local_addr() {
                    result.push(*addr.ip());
                }
            }
        }
    }
    result
}

fn resolve_base(location: &str, xml_text: &str) -> Result<String, &'static str> {
    let root = xml::parse(xml_text)?;
    if let Some(url_base) = root.child("URLBase").map(|n| n.text.trim()).filter(|s| !s.is_empty()) {
        return Ok(url_base.trim_end_matches('/').to_string());
    }
    let cut = location.rfind('/').map(|p| p + 1).unwrap_or(location.len());
    Ok(location[..cut].to_string())
}

fn join_url(base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    if path.starts_with('/') {
        // Origin from base, e.g. http://10.77.0.1:80
        let scheme_end = base.find("://").map(|p| p + 3).unwrap_or(0);
        let slash = base[scheme_end..]
            .find('/')
            .map(|p| scheme_end + p)
            .unwrap_or(base.len());
        return format!("{}{}", &base[..slash], path);
    }
    format!("{}/{}", base.trim_end_matches('/'), path)
}

fn parse_description(location: &str, xml_text: &str) -> Result<DiscoveryResult, &'static str> {
    let root = xml::parse(xml_text)?;
    let base = resolve_base(location, xml_text)?;

    fn visit(node: &xml::XmlNode, base: &str, location: &str, out: &mut DiscoveryResult) {
        if node.name == "device" && node.value("deviceType").starts_with(RENDERER_TYPE) {
            let udn = node.value("UDN");
            let mut transport = None;
            let mut connection = None;
            if let Some(services) = node.child("serviceList") {
                for svc in services.children.iter().filter(|n| n.name == "service") {
                    let service_type = svc.value("serviceType");
                    let control = svc.value("controlURL");
                    if control.is_empty() {
                        continue;
                    }
                    let service = CastService {
                        service_type: service_type.clone(),
                        control: join_url(base, &control),
                    };
                    if service_type.starts_with(AVTRANSPORT_TYPE) {
                        transport = Some(service);
                    } else if service_type.starts_with(CONNECTION_TYPE) {
                        connection = Some(service);
                    }
                }
            }
            if !udn.is_empty() {
                match transport {
                    Some(t) => {
                        let addr = location_host(location);
                        out.renderers.push(DiscoveredRenderer {
                            renderer: CastRenderer {
                                udn: udn.clone(),
                                friendly_name: node.value("friendlyName"),
                                manufacturer: node.value("manufacturer"),
                                model_name: node.value("modelName"),
                                address: addr,
                                location: location.to_string(),
                            },
                            transport: t,
                            connection,
                        });
                    }
                    None => out.without_transport.push(udn),
                }
            }
        }
        for child in &node.children {
            visit(child, base, location, out);
        }
    }

    let mut out = DiscoveryResult::default();
    visit(&root, &base, location, &mut out);
    Ok(out)
}

fn location_host(location: &str) -> Ipv4Addr {
    let after = location.split("://").nth(1).unwrap_or(location);
    let host = after.split('/').next().unwrap_or(after);
    let host = host.split(':').next().unwrap_or(host);
    host.parse().unwrap_or(Ipv4Addr::UNSPECIFIED)
}

/// Blocking SSDP search across local interfaces. Collect LOCATION values,
/// then fetch and parse each device description.
pub fn discover(config: &CastConfig) -> Result<DiscoveryResult, &'static str> {
    let deadline = Instant::now() + Duration::from_millis(config.search_ms.max(500));
    let mut locations: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let addresses = local_addresses(config.interface);
    let mut sockets: Vec<UdpSocket> = Vec::new();
    for addr in addresses {
        let bind = SocketAddr::V4(SocketAddrV4::new(addr, 0));
        if let Ok(socket) = UdpSocket::bind(bind) {
            let _ = socket.set_multicast_ttl_v4(2);
            // Binding the socket to the interface address already selects the
            // outgoing interface for multicast on Linux, so no explicit
            // IP_MULTICAST_IF is needed (std has no stable setter for it).
            let _ = socket.set_read_timeout(Some(Duration::from_millis(200)));
            sockets.push(socket);
        }
    }
    if sockets.is_empty() {
        // Last resort: wildcard socket, kernel picks the interface.
        if let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
            let _ = socket.set_multicast_ttl_v4(2);
            let _ = socket.set_read_timeout(Some(Duration::from_millis(200)));
            sockets.push(socket);
        }
    }
    if sockets.is_empty() {
        return Err("Cannot bind SSDP socket");
    }

    let target = SocketAddr::V4(SocketAddrV4::new(MULTICAST.0, MULTICAST.1));
    let ms = search_request(&format!("{}1", RENDERER_TYPE));
    let ms_av = search_request(&format!("{}1", AVTRANSPORT_TYPE));
    let mut sent_rounds = 0u8;
    let mut buf = [0u8; 16 * 1024];
    while Instant::now() < deadline {
        let elapsed_ms = config.search_ms.saturating_sub(deadline.saturating_duration_since(Instant::now()).as_millis() as u64);
        if sent_rounds < 3 && elapsed_ms >= sent_rounds as u64 * 500 {
            for socket in &sockets {
                let _ = socket.send_to(ms.as_bytes(), target);
                let _ = socket.send_to(ms_av.as_bytes(), target);
            }
            sent_rounds += 1;
        }
        for socket in &sockets {
            loop {
                match socket.recv_from(&mut buf) {
                    Ok((n, _)) => {
                        if let Some(reply) = parse_reply(&buf[..n]) {
                            if !seen.contains(&reply.location) && seen.len() < 256 {
                                seen.insert(reply.location.clone());
                                locations.push(reply.location);
                            }
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => break,
                    Err(_) => break,
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut result = DiscoveryResult::default();
    let mut renderer_udns = HashSet::new();
    for location in locations {
        let connect_ms = 1500u64;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let budget = connect_ms.max(remaining.as_millis() as u64).min(2000);
        match http_get(&location, Duration::from_millis(budget)) {
            Ok(body) => {
                if let Ok(parsed) = parse_description(&location, &String::from_utf8_lossy(&body)) {
                    for found in parsed.renderers {
                        if renderer_udns.insert(found.renderer.udn.clone()) {
                            result.renderers.push(found);
                        }
                    }
                    for udn in parsed.without_transport {
                        if !result.without_transport.contains(&udn) {
                            result.without_transport.push(udn);
                        }
                    }
                }
            }
            Err(HttpError::Status(code)) => {
                // Non-200 descriptions are ignored, same as the Kotlin discovery.
                let _ = code;
            }
            Err(_) => {}
        }
    }
    result.renderers.sort_by(|a, b| a.renderer.friendly_name.cmp(&b.renderer.friendly_name));
    Ok(result)
}
