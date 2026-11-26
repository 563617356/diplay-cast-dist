// SPDX-FileCopyrightText: 2026 OpenXPlay.org
// SPDX-License-Identifier: AGPL-3.0-only

//! SOAP/UPnP AVTransport control, ported from kotlin/cast Soap.kt.

use crate::http::{HttpError, HttpRequest};
use crate::model::{CastService, DLNA_FEATURES};
use crate::xml;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug)]
#[allow(dead_code)] // Fault details retained for diagnostics.
pub enum SoapError {
    Fault { code: i32, description: String },
    Http(HttpError),
    Parse(&'static str),
}

impl From<HttpError> for SoapError {
    fn from(e: HttpError) -> Self {
        SoapError::Http(e)
    }
}

pub type SoapResult = BTreeMap<String, String>;

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn envelope(service: &str, action: &str, arguments: &[(&str, String)]) -> String {
    let args: String = arguments
        .iter()
        .map(|(k, v)| format!("<{}>{}</{}>", k, xml_escape(v), k))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
<u:{action} xmlns:u=\"{service}\">{args}</u:{action}>\
</s:Body></s:Envelope>",
        action = action,
        service = xml_escape(service),
        args = args
    )
}

pub fn metadata(title: &str, url: &str) -> String {
    format!(
        "<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\">\
<item id=\"openxplay-cast\" parentID=\"-1\" restricted=\"1\"><dc:title>{title}</dc:title>\
<upnp:class>object.item.videoItem</upnp:class>\
<res protocolInfo=\"http-get:*:video/mp2t:{features}\">{url}</res></item></DIDL-Lite>",
        title = xml_escape(title),
        features = DLNA_FEATURES,
        url = xml_escape(url)
    )
}

fn parse_response(action: &str, status: u16, body: &[u8]) -> Result<SoapResult, SoapError> {
    if status != 200 && status != 500 {
        return Err(SoapError::Parse("SOAP HTTP status"));
    }
    let root = xml::parse(&String::from_utf8_lossy(body)).map_err(|_| SoapError::Parse("Bad SOAP XML"))?;
    let envelope_node = if root.name == "Envelope" {
        &root
    } else {
        root.child("Envelope").ok_or(SoapError::Parse("Missing SOAP envelope"))?
    };
    let body_node = envelope_node
        .child("Body")
        .ok_or(SoapError::Parse("Missing SOAP body"))?;
    if status == 500 || body_node.child("Fault").is_some() {
        let mut errors = Vec::new();
        body_node.descendants("UPnPError", &mut errors);
        let error = errors
            .first()
            .ok_or(SoapError::Parse("Missing UPnP fault"))?;
        return Err(SoapError::Fault {
            code: error.value("errorCode").parse().unwrap_or(-1),
            description: error.value("errorDescription"),
        });
    }
    let response_name = format!("{}Response", action);
    let response = body_node
        .child(&response_name)
        .ok_or(SoapError::Parse("Missing SOAP response"))?;
    let mut args = BTreeMap::new();
    for child in &response.children {
        args.insert(child.name.clone(), child.text.trim().to_string());
    }
    Ok(args)
}

/// Send a SOAP action, tolerating HTTP 500 responses that carry a parsed
/// AVTransport error. We read the raw status ourselves instead of relying on
/// `http_request` to reject non-200 by adding a permissive variant below.
pub fn soap_action(
    service: &CastService,
    action: &str,
    arguments: &[(&str, String)],
    timeout: Duration,
) -> Result<SoapResult, SoapError> {
    let body = envelope(&service.service_type, action, arguments);
    let headers = [
        ("Content-Type", "text/xml; charset=\"utf-8\""),
        (
            "SOAPACTION",
            Box::leak(format!("\"{}#{}\"", service.service_type, action).into_boxed_str()),
        ),
    ];
    let owned_headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (*k, &**v))
        .collect();
    let req = HttpRequest {
        method: "POST",
        uri: &service.control,
        headers: &owned_headers,
        body: body.as_bytes(),
    };
    // For SOAP we need the body even on HTTP 500 (UPnP faults). Use the raw
    // request helper that returns (status, body) instead of requiring 200.
    let (status, raw) = http_request_raw(&req, timeout)?;
    parse_response(action, status, &raw)
}

/// Internal raw variant: same as http_request but returns status plus body on
/// any 2xx/4xx/5xx response.
fn http_request_raw(req: &HttpRequest, timeout: Duration) -> Result<(u16, Vec<u8>), HttpError> {
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, ToSocketAddrs};

    let rest = req
        .uri
        .strip_prefix("http://")
        .ok_or(HttpError::Invalid("Not an HTTP URL"))?;
    let (authority, path) = match rest.find('/') {
        Some(p) => (&rest[..p], &rest[p..]),
        None => (rest, "/"),
    };
    let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
        if p.chars().all(|c| c.is_ascii_digit()) {
            (h, p.parse::<u16>().map_err(|_| HttpError::Invalid("Bad port"))?)
        } else {
            (authority, 80)
        }
    } else {
        (authority, 80)
    };
    let addr: SocketAddr = if let Ok(ip) = host.parse::<Ipv4Addr>() {
        SocketAddr::V4(SocketAddrV4::new(ip, port))
    } else {
        (host, port)
            .to_socket_addrs()
            .map_err(|_| HttpError::Invalid("DNS failed"))?
            .find(|a| a.is_ipv4())
            .ok_or(HttpError::Invalid("No IPv4 address"))?
    };
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let host_header = if port == 80 {
        host.to_string()
    } else {
        format!("{}:{}", host, port)
    };
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: OpenXPlay/0 UPnP/1.1 OpenXPlay-Stream/0\r\nConnection: close\r\n",
        req.method, path, host_header
    );
    for (k, v) in req.headers {
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", req.body.len()));
    stream.write_all(head.as_bytes())?;
    stream.write_all(req.body)?;
    stream.flush()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(HttpError::Invalid("No HTTP header terminator"))?;
    let status_line = String::from_utf8_lossy(&raw[..split]);
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or(HttpError::Invalid("Bad HTTP status"))?;
    Ok((status, raw[split + 4..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_envelope() {
        let args = [("InstanceID", "0".to_string())];
        let xml_text = envelope("urn:service", "Play", &args);
        assert!(xml_text.contains("<u:Play"));
        assert!(xml_text.contains("<InstanceID>0</InstanceID>"));
    }

    #[test]
    fn metadata_escapes_title() {
        let m = metadata("A&B", "http://x/y");
        assert!(m.contains("A&amp;B"));
        assert!(m.contains("video/mp2t"));
    }
}
