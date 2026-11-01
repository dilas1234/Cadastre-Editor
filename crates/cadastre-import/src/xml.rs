//! A small, allocation-light XML reader for LandXML and similar survey exchange files.
//!
//! This is not a validating XML processor. It understands elements, attributes, text, CDATA,
//! comments, processing instructions and the predefined entities — enough for LandXML, which is
//! simple and machine-generated. Namespace prefixes are stripped so `<lx:CgPoint>` and
//! `<CgPoint>` are the same element.

use std::collections::BTreeMap;
use std::fmt;

/// One element in the parsed tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    /// Local element name (namespace prefix removed).
    pub name: String,
    /// Attributes by local name.
    pub attrs: BTreeMap<String, String>,
    /// Concatenated character data directly inside the element.
    pub text: String,
    /// Child elements.
    pub children: Vec<Node>,
}

impl Node {
    /// An attribute value by local name.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }

    /// The first child with the given local name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// All children with the given local name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Depth-first search for the first descendant with the given name, excluding self.
    pub fn find(&self, name: &str) -> Option<&Node> {
        for child in &self.children {
            if child.name == name {
                return Some(child);
            }
            if let Some(found) = child.find(name) {
                return Some(found);
            }
        }
        None
    }

    /// All descendants (any depth) with the given local name.
    pub fn find_all<'a>(&'a self, name: &'a str, out: &mut Vec<&'a Node>) {
        for child in &self.children {
            if child.name == name {
                out.push(child);
            }
            child.find_all(name, out);
        }
    }
}

/// Why XML parsing failed.
#[derive(Clone, Debug, PartialEq)]
pub struct XmlError {
    /// Message describing the problem.
    pub message: String,
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for XmlError {}

fn error<T>(message: impl Into<String>) -> Result<T, XmlError> {
    Err(XmlError {
        message: message.into(),
    })
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..end];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match replacement {
            Some(c) => out.push(c),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn parse_attrs(input: &str) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        if start == i {
            break;
        }
        let name = &input[start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            // Attribute without a value: keep it with an empty value.
            attrs.insert(local_name(name).to_string(), String::new());
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        if quote == b'"' || quote == b'\'' {
            i += 1;
            let value_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let value = unescape(&input[value_start..i.min(input.len())]);
            attrs.insert(local_name(name).to_string(), value);
            if i < bytes.len() {
                i += 1;
            }
        } else {
            let value_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            attrs.insert(
                local_name(name).to_string(),
                unescape(&input[value_start..i]),
            );
        }
    }
    attrs
}

/// Parse an XML document and return its root element.
pub fn parse(contents: &str) -> Result<Node, XmlError> {
    let text = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let mut i = 0;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        if bytes[i] != b'<' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'<' {
                i += 1;
            }
            let chunk = unescape(&text[start..i]);
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&chunk);
            }
            continue;
        }

        // Comments, declarations, CDATA and doctype.
        if text[i..].starts_with("<!--") {
            match text[i + 4..].find("-->") {
                Some(end) => i += 4 + end + 3,
                None => return error("unterminated XML comment"),
            }
            continue;
        }
        if text[i..].starts_with("<![CDATA[") {
            let start = i + 9;
            match text[start..].find("]]>") {
                Some(end) => {
                    if let Some(top) = stack.last_mut() {
                        top.text.push_str(&text[start..start + end]);
                    }
                    i = start + end + 3;
                }
                None => return error("unterminated CDATA section"),
            }
            continue;
        }
        if text[i..].starts_with("<?") {
            match text[i + 2..].find("?>") {
                Some(end) => i += 2 + end + 2,
                None => return error("unterminated processing instruction"),
            }
            continue;
        }
        if text[i..].starts_with("<!") {
            // Doctype, possibly with an internal subset.
            let mut depth = 0usize;
            let mut j = i + 2;
            while j < bytes.len() {
                match bytes[j] {
                    b'[' => depth += 1,
                    b']' => depth = depth.saturating_sub(1),
                    b'>' if depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            i = (j + 1).min(bytes.len());
            continue;
        }

        // End tag.
        if text[i..].starts_with("</") {
            let start = i + 2;
            let Some(end) = text[start..].find('>') else {
                return error("unterminated end tag");
            };
            let name = local_name(text[start..start + end].trim()).to_string();
            let Some(node) = stack.pop() else {
                return error(format!("unexpected end tag </{name}>"));
            };
            if node.name != name {
                return error(format!("end tag </{name}> does not close <{}>", node.name));
            }
            if let Some(parent) = stack.last_mut() {
                parent.children.push(node);
            } else if root.is_none() {
                root = Some(node);
            } else {
                return error("multiple root elements");
            }
            i = start + end + 1;
            continue;
        }

        // Start tag.
        let start = i + 1;
        let mut j = start;
        let mut quote: Option<u8> = None;
        while j < bytes.len() {
            match bytes[j] {
                b'"' | b'\'' if quote.is_none() => quote = Some(bytes[j]),
                b if Some(b) == quote => quote = None,
                b'>' if quote.is_none() => break,
                _ => {}
            }
            j += 1;
        }
        if j >= bytes.len() {
            return error("unterminated start tag");
        }
        let inner = text[start..j].trim_end();
        let self_closing = inner.ends_with('/');
        let inner = inner.trim_end_matches('/');
        let mut split = inner.splitn(2, char::is_whitespace);
        let name = split.next().unwrap_or("").to_string();
        if name.is_empty() {
            return error("empty element name");
        }
        let attrs = split.next().map(parse_attrs).unwrap_or_default();
        let node = Node {
            name: local_name(&name).to_string(),
            attrs,
            text: String::new(),
            children: Vec::new(),
        };
        if self_closing {
            if let Some(parent) = stack.last_mut() {
                parent.children.push(node);
            } else if root.is_none() {
                root = Some(node);
            } else {
                return error("multiple root elements");
            }
        } else {
            stack.push(node);
        }
        i = j + 1;
    }

    match root {
        Some(root) if stack.is_empty() => Ok(root),
        Some(_) => error("unclosed elements at end of document"),
        None if stack.is_empty() => error("document has no root element"),
        None => error("document has no root element"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_elements_attributes_and_text() {
        let xml = r#"<?xml version="1.0"?>
            <LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2">
              <CgPoints>
                <CgPoint name="1001" code="BC">1233094.048 352440.677 512.3</CgPoint>
              </CgPoints>
            </LandXML>"#;
        let root = parse(xml).unwrap();
        assert_eq!(root.name, "LandXML");
        let point = root.find("CgPoint").unwrap();
        assert_eq!(point.attr("name"), Some("1001"));
        assert_eq!(point.text.trim(), "1233094.048 352440.677 512.3");
    }

    #[test]
    fn strips_namespace_prefixes_and_decodes_entities() {
        let root = parse(r#"<lx:Root a="1"><lx:Child>x &amp; y &lt;z&gt;</lx:Child></lx:Root>"#)
            .unwrap();
        assert_eq!(root.name, "Root");
        assert_eq!(root.children[0].name, "Child");
        assert_eq!(root.children[0].text, "x & y <z>");
    }

    #[test]
    fn handles_self_closing_cdata_and_comments() {
        let root = parse(
            r#"<Root><!-- note --><Empty a="1"/><Data><![CDATA[a < b & c]]></Data></Root>"#,
        )
        .unwrap();
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.child("Empty").unwrap().attr("a"), Some("1"));
        assert_eq!(root.child("Data").unwrap().text, "a < b & c");
    }

    #[test]
    fn rejects_malformed_documents() {
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a>").is_err());
        assert!(parse("").is_err());
        assert!(parse("<!-- only a comment -->").is_err());
    }

    #[test]
    fn parses_single_and_double_quoted_attributes() {
        let root = parse(r#"<a x="1" y='2' z=3></a>"#).unwrap();
        assert_eq!(root.attr("x"), Some("1"));
        assert_eq!(root.attr("y"), Some("2"));
        assert_eq!(root.attr("z"), Some("3"));
    }
}
