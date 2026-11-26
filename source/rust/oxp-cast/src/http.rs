// SPDX-FileCopyrightText: 2026 OpenXPlay.org
// SPDX-License-Identifier: AGPL-3.0-only

//! Tiny synchronous HTTP/1.1 client, enough for UPnP description GETs and
//! SOAP POSTs. Standard library only; no TLS (UPnP control is plain HTTP).

use crate::model::USER_AGENT;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug)]
#[allow(dead_code)] // Error payloads retained for callers/diagnostics.
pub enum HttpError {
    Io(std::io::Error),
    Status(u16),
    Invalid(&'static str),
}

impl From<std::io::Error> for HttpError {
    fn from(e: std::io::Error) -> Self {
        HttpError::Io(e)
    }
}

#[derive(Clone, Debug)]
pub struct HttpRequest<'a> {
    pub method: &'a str,
    pub uri: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
}

fn parse_uri(uri: &str) -> Result<(String, u16, String), HttpError> {
    let rest = uri
        .strip_prefix("http://")
        .ok_or(HttpError::Invalid("Not an HTTP URL"))?;
    let (authority, path) = match rest.find('/') {
        Some(p) => (&rest[..p], &rest[p..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(HttpError::Invalid("Empty host"));
    }
    let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
        if p.chars().all(|c| c.is_ascii_digit()) {
            (h, p.parse::<u16>().map_err(|_| HttpError::Invalid("Bad port"))?)
        } else {
            (authority, 80)
        }
    } else {
        (authority, 80)
    };
    Ok((host.to_string(), port, path.to_string()))
}

fn resolve(host: &str, port: u16) -> Result<SocketAddr, HttpError> {
    // Prefer IPv4 for UPnP; renderers advertise IPv4 locations.
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Ok(SocketAddr::V4(SocketAddrV4::new(ip, port)));
    }
    let mut addrs = (host, port)
        .to_socket_addrs()
        .map_err(|_| HttpError::Invalid("DNS failed"))?;
    addrs
        .find(|a| a.is_ipv4())
        .ok_or(HttpError::Invalid("No IPv4 address"))
}

/// Perform an HTTP request and return the response body on HTTP 200.
pub fn http_request(req: &HttpRequest, timeout: Duration) -> Result<Vec<u8>, HttpError> {
    let (host, port, path) = parse_uri(req.uri)?;
    let addr = resolve(&host, port)?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let host_header = if port == 80 {
        host.clone()
    } else {
        format!("{}:{}", host, port)
    };
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nConnection: close\r\n",
        req.method, path, host_header, USER_AGENT
    );
    for (k, v) in req.headers {
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    if !req.body.is_empty() || req.method == "POST" {
        head.push_str(&format!("Content-Length: {}\r\n", req.body.len()));
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes())?;
    stream.write_all(req.body)?;
    stream.flush()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;

    let split = find(&raw, b"\r\n\r\n").ok_or(HttpError::Invalid("No HTTP header terminator"))?;
    let header_text = String::from_utf8_lossy(&raw[..split]);
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().ok_or(HttpError::Invalid("No HTTP status"))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or(HttpError::Invalid("Bad HTTP status"))?;
    let mut chunked = false;
    let mut content_length: Option<usize> = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "transfer-encoding" if v.trim().eq_ignore_ascii_case("chunked") => chunked = true,
                "content-length" => content_length = v.trim().parse().ok(),
                _ => {}
            }
        }
    }
    let mut body = raw[split + 4..].to_vec();
    if status != 200 {
        return Err(HttpError::Status(status));
    }
    if chunked {
        body = decode_chunked(&body)?;
    } else if let Some(len) = content_length {
        body.truncate(len);
    }
    Ok(body)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn decode_chunked(input: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        let line_end = find(&input[pos..], b"\r\n")
            .map(|p| pos + p)
            .ok_or(HttpError::Invalid("Bad chunk header"))?;
        let size_text = String::from_utf8_lossy(&input[pos..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| HttpError::Invalid("Bad chunk size"))?;
        pos = line_end + 2;
        if size == 0 {
            break;
        }
        if pos + size > input.len() {
            return Err(HttpError::Invalid("Truncated chunk"));
        }
        out.extend_from_slice(&input[pos..pos + size]);
        pos += size + 2;
    }
    Ok(out)
}

pub fn http_get(uri: &str, timeout: Duration) -> Result<Vec<u8>, HttpError> {
    let headers = [];
    http_request(
        &HttpRequest {
            method: "GET",
            uri,
            headers: &headers,
            body: &[],
        },
        timeout,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uri() {
        let (host, port, path) = parse_uri("http://10.77.0.1:80/AVTransport/control").unwrap();
        assert_eq!(host, "10.77.0.1");
        assert_eq!(port, 80);
        assert_eq!(path, "/AVTransport/control");
    }

    #[test]
    fn decodes_chunked() {
        let raw = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(raw).unwrap(), b"hello world");
    }
}
