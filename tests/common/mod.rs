#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use zip::ZipArchive;
use zip::write::{SimpleFileOptions, ZipWriter};

/// All entries of a zip archive.
pub fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid zip");
    let mut out = BTreeMap::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        if f.is_dir() {
            continue;
        }
        let name = f.name().unwrap().to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        out.insert(name, buf);
    }
    out
}

pub fn xml_of(bytes: &[u8], name: &str) -> String {
    String::from_utf8(unzip(bytes).remove(name).unwrap_or_else(|| panic!("no {name} in zip"))).unwrap()
}

fn normalize_tag(tag: &str) -> String {
    if tag.starts_with("<?") || tag.starts_with("<!") || tag.starts_with("</") {
        return tag.to_string();
    }
    let self_closing = tag.ends_with("/>");
    let inner = tag.trim_start_matches('<').trim_end_matches('>').trim_end_matches('/');
    let name_end = inner.find(char::is_whitespace).unwrap_or(inner.len());
    let name = &inner[..name_end];
    let mut rest = &inner[name_end..];
    let mut attrs = Vec::new();
    loop {
        rest = rest.trim_start();
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next() else { break };
        let after = &after[1..];
        let Some(end) = after.find(q) else { break };
        let mut value = after[..end].to_string();
        // normalize decimal character references
        while let Some(i) = value.find("&#") {
            if value[i + 2..].starts_with('x') {
                break;
            }
            let Some(semi) = value[i..].find(';') else { break };
            let num: u32 = value[i + 2..i + semi].parse().unwrap_or(0);
            value.replace_range(i..i + semi + 1, &format!("&#x{num:X};"));
        }
        attrs.push((key, value));
        rest = &after[end + 1..];
    }
    attrs.sort();
    let mut s = format!("<{name}");
    for (k, v) in attrs {
        s.push_str(&format!(" {k}=\"{v}\""));
    }
    s.push_str(if self_closing { "/>" } else { ">" });
    s
}

/// Tokenize XML into normalized tags and texts so that two documents can be
/// compared modulo attribute order, `<a></a>` vs `<a/>` and whitespace-only
/// text nodes.
pub fn normalize_xml(xml: &str) -> Vec<String> {
    let xml = xml.trim_start_matches('\u{feff}');
    let mut tokens: Vec<String> = Vec::new();
    let mut i = 0;
    let b = xml.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            let end = xml[i..].find('>').map(|e| i + e).unwrap_or(b.len() - 1);
            let tag = normalize_tag(&xml[i..=end]);
            if tag.starts_with("</")
                && let Some(last) = tokens.last_mut()
                && last.starts_with('<')
                && !last.starts_with("</")
                && !last.ends_with("/>")
                && {
                    let name = &tag[2..tag.len() - 1];
                    let lname = last[1..].split([' ', '>']).next().unwrap_or("");
                    lname == name
                }
            {
                last.pop();
                last.push_str("/>");
            } else {
                tokens.push(tag);
            }
            i = end + 1;
        } else {
            let end = xml[i..].find('<').map(|e| i + e).unwrap_or(b.len());
            let text = &xml[i..end];
            if !text.trim().is_empty() {
                tokens.push(text.to_string());
            }
            i = end;
        }
    }
    tokens
}

/// Returns a description of the first difference between two xml documents.
pub fn xml_diff(actual: &str, expected: &str) -> Option<String> {
    let a = normalize_xml(actual);
    let e = normalize_xml(expected);
    if a == e {
        return None;
    }
    let first = a.iter().zip(e.iter()).position(|(x, y)| x != y).unwrap_or(a.len().min(e.len()));
    let ctx = |v: &[String]| v[first.saturating_sub(6)..(first + 6).min(v.len())].join("");
    Some(format!(
        "first difference at token {first} (actual {} tokens, expected {} tokens)\n  actual:   ...{}...\n  expected: ...{}...",
        a.len(),
        e.len(),
        ctx(&a),
        ctx(&e)
    ))
}

/// Compare two packages; returns the list of differing entries.
pub fn zip_diff(actual: &[u8], expected: &[u8]) -> Vec<String> {
    let a = unzip(actual);
    let e = unzip(expected);
    let mut diffs = Vec::new();
    for (name, ev) in &e {
        let Some(av) = a.get(name) else {
            diffs.push(format!("{name}: missing in actual"));
            continue;
        };
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let (Ok(at), Ok(et)) = (std::str::from_utf8(av), std::str::from_utf8(ev)) else { continue };
            if let Some(d) = xml_diff(at, et) {
                diffs.push(format!("{name}: {d}"));
            }
        } else if av != ev {
            diffs.push(format!("{name}: binary content differs"));
        }
    }
    for name in a.keys() {
        if !e.contains_key(name) {
            diffs.push(format!("{name}: unexpected entry"));
        }
    }
    diffs
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/><Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/></Types>"#;

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

pub const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

/// Build a minimal .docx whose body is `body` (OOXML that goes inside `<w:body>`).
pub fn docx(body: &str) -> Vec<u8> {
    docx_with_parts(body, None, None, None)
}

pub fn docx_with_parts(body: &str, header: Option<&str>, footer: Option<&str>, footnotes: Option<&str>) -> Vec<u8> {
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    let o = SimpleFileOptions::default();
    w.start_file("[Content_Types].xml", o).unwrap();
    w.write_all(CONTENT_TYPES.as_bytes()).unwrap();
    w.start_file("_rels/.rels", o).unwrap();
    w.write_all(RELS.as_bytes()).unwrap();
    w.start_file("word/document.xml", o).unwrap();
    write!(w, r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W_NS}><w:body>{body}<w:sectPr/></w:body></w:document>"#)
        .unwrap();
    if let Some(h) = header {
        w.start_file("word/header1.xml", o).unwrap();
        write!(w, r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr {W_NS}>{h}</w:hdr>"#).unwrap();
    }
    if let Some(f) = footer {
        w.start_file("word/footer1.xml", o).unwrap();
        write!(w, r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr {W_NS}>{f}</w:ftr>"#).unwrap();
    }
    if let Some(f) = footnotes {
        w.start_file("word/footnotes.xml", o).unwrap();
        write!(w, r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:footnotes {W_NS}>{f}</w:footnotes>"#)
            .unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// A paragraph with one run per text fragment (simulates Word splitting text
/// into several runs).
pub fn p(runs: &[&str]) -> String {
    let mut s = String::from("<w:p>");
    for r in runs {
        s.push_str(&format!(r#"<w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">{r}</w:t></w:r>"#));
    }
    s.push_str("</w:p>");
    s
}

/// Body of the rendered word/document.xml (between `<w:body>` and `<w:sectPr/>`).
pub fn body_of(bytes: &[u8]) -> String {
    let xml = xml_of(bytes, "word/document.xml");
    let start = xml.find("<w:body>").unwrap() + "<w:body>".len();
    let end = xml.rfind("<w:sectPr/>").unwrap();
    xml[start..end].to_string()
}
