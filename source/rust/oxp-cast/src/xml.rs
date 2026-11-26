// SPDX-FileCopyrightText: 2026 OpenXPlay.org
// SPDX-License-Identifier: AGPL-3.0-only

//! Minimal XML reader for UPnP device descriptions and SOAP envelopes.
//! Matches elements by local name (namespace prefix stripped), which is what
//! `CastDiscovery` and `CastSoap` in the Kotlin implementation rely on.

#[derive(Clone, Debug, Default)]
pub struct XmlNode {
    /// Local name, with any `prefix:` removed.
    pub name: String,
    /// Direct character data (entities decoded, trimmed by callers).
    pub text: String,
    pub children: Vec<XmlNode>,
}

impl XmlNode {
    /// First direct child with this local name, ignoring namespace prefixes.
    pub fn child(&self, name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|n| n.name == name)
    }

    /// Trimmed text of the first direct child with this local name.
    pub fn value(&self, name: &str) -> String {
        self.child(name)
            .map(|n| n.text.trim().to_string())
            .unwrap_or_default()
    }

    /// All descendants (excluding self) with this local name, depth-first.
    pub fn descendants<'a>(&'a self, name: &'a str, out: &mut Vec<&'a XmlNode>) {
        for child in &self.children {
            if child.name == name {
                out.push(child);
            }
            child.descendants(name, out);
        }
    }
}

fn local_name(raw: &str) -> String {
    raw.rsplit(':').next().unwrap_or(raw).to_string()
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        if let Some(end) = after.find(';') {
            let entity = &after[..end];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                    } else if let Some(dec) = entity.strip_prefix('#') {
                        dec.parse::<u32>().ok().and_then(char::from_u32)
                    } else {
                        None
                    }
                }
            };
            if let Some(c) = decoded {
                out.push(c);
                rest = &after[end + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Parse an XML document into a tree. Tolerant of declarations, comments,
/// processing instructions, CDATA and self-closing tags.
pub fn parse(input: &str) -> Result<XmlNode, &'static str> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    // roots is a stack of (node, self_closing flag handled on close)
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root: Option<XmlNode> = None;

    while i < len {
        if bytes[i] != b'<' {
            let start = i;
            while i < len && bytes[i] != b'<' {
                i += 1;
            }
            let text = decode_entities(&input[start..i]);
            if let Some(top) = stack.last_mut() {
                if !text.trim().is_empty() || top.text.is_empty() {
                    top.text.push_str(&text);
                }
            }
            continue;
        }
        if input[i..].starts_with("<!--") {
            i = input[i + 4..]
                .find("-->")
                .map(|p| i + 4 + p + 3)
                .ok_or("Unterminated XML comment")?;
            continue;
        }
        if input[i..].starts_with("<![CDATA[") {
            let start = i + 9;
            let end = input[start..]
                .find("]]>")
                .map(|p| start + p)
                .ok_or("Unterminated CDATA")?;
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&input[start..end]);
            }
            i = end + 3;
            continue;
        }
        if input[i..].starts_with("<?") {
            i = input[i + 2..]
                .find("?>")
                .map(|p| i + 2 + p + 2)
                .ok_or("Unterminated processing instruction")?;
            continue;
        }
        if input[i..].starts_with("<!") {
            // DOCTYPE or other declaration: skip to next '>'.
            i = input[i..]
                .find('>')
                .map(|p| i + p + 1)
                .ok_or("Unterminated declaration")?;
            continue;
        }
        let end = input[i..]
            .find('>')
            .map(|p| i + p)
            .ok_or("Unterminated XML tag")?;
        let raw_tag = input[i + 1..end].trim();
        i = end + 1;

        if let Some(rest) = raw_tag.strip_prefix('/') {
            let closed = local_name(rest.trim());
            let node = stack.pop().ok_or("Unexpected closing tag")?;
            if node.name != closed {
                return Err("Mismatched closing tag");
            }
            if let Some(parent) = stack.last_mut() {
                parent.children.push(node);
            } else {
                root = Some(node);
            }
            continue;
        }

        let self_closing = raw_tag.ends_with('/');
        let name_part = if self_closing {
            raw_tag[..raw_tag.len() - 1].trim()
        } else {
            raw_tag
        };
        let name = local_name(
            name_part
                .split(|c: char| c.is_whitespace())
                .next()
                .unwrap_or(name_part),
        );
        let node = XmlNode {
            name,
            ..XmlNode::default()
        };
        if self_closing {
            if let Some(parent) = stack.last_mut() {
                parent.children.push(node);
            } else {
                root = Some(node);
            }
        } else {
            stack.push(node);
        }
    }
    if !stack.is_empty() {
        return Err("Unclosed XML element");
    }
    root.ok_or("Empty XML document")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_description() {
        let xml = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
<URLBase>http://10.0.0.1:80/</URLBase>
<device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
<UDN>uuid:car</UDN><friendlyName>Car TV</friendlyName>
<serviceList><service>
<serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
<controlURL>/AVTransport/control</controlURL>
</service></serviceList></device></root>"#;
        let root = parse(xml).unwrap();
        let device = root.child("device").unwrap();
        assert_eq!(device.value("UDN"), "uuid:car");
        assert_eq!(device.value("friendlyName"), "Car TV");
        let svc = device
            .child("serviceList")
            .unwrap()
            .child("service")
            .unwrap();
        assert_eq!(
            svc.value("controlURL"),
            "/AVTransport/control"
        );
    }

    #[test]
    fn parses_soap_response_and_entities() {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body>
<u:SetAVTransportURIResponse xmlns:u="urn:schemas-upnp-org:service:AVTransport:1"/>
</s:Body></s:Envelope>"#;
        let root = parse(xml).unwrap();
        let body = root.child("Body").unwrap();
        assert!(body.child("SetAVTransportURIResponse").is_some());
        assert_eq!(decode_entities("a&amp;b&lt;c"), "a&b<c");
    }
}
