//! Zip / OPC package handling: content types, which parts are templated, and
//! writing the result back.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use zip::ZipArchive;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::error::{Error, TemplateError};
use crate::filetype::{FileType, ct};

pub(crate) struct Package {
    pub bytes: Vec<u8>,
    /// Entry names in archive order.
    pub names: Vec<String>,
    /// Content type of every entry ("" when unknown).
    pub content_types: HashMap<String, String>,
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut search = 0;
    while let Some(i) = tag[search..].find(name) {
        let i = search + i;
        let before_ok = i > 0 && tag.as_bytes()[i - 1].is_ascii_whitespace();
        let rest = &tag[i + name.len()..];
        let rest_trim = rest.trim_start();
        if before_ok && let Some(after_eq) = rest_trim.strip_prefix('=') {
            let after_eq = after_eq.trim_start();
            let quote = after_eq.chars().next()?;
            if quote == '"' || quote == '\'' {
                let inner = &after_eq[1..];
                let end = inner.find(quote)?;
                return Some(&inner[..end]);
            }
        }
        search = i + name.len();
    }
    None
}

fn elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = xml[from..].find(&open) {
        let start = from + i;
        let after = xml.as_bytes().get(start + open.len()).copied();
        let Some(end) = xml[start..].find('>') else { break };
        if matches!(after, Some(b' ' | b'/' | b'>' | b'\t' | b'\n' | b'\r')) {
            out.push(&xml[start..=start + end]);
        }
        from = start + end + 1;
    }
    out
}

pub(crate) fn read_entry(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, Error> {
    let mut f = archive.by_name(name)?;
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

pub(crate) fn decode_xml(bytes: Vec<u8>) -> String {
    let s = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    match s.strip_prefix('\u{feff}') {
        Some(stripped) => stripped.to_string(),
        None => s,
    }
}

impl Package {
    pub fn open(bytes: Vec<u8>) -> Result<Self, Error> {
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice()))?;
        let names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.name_for_index(i).and_then(|r| r.ok()).map(|n| n.to_string()))
            .filter(|n| !n.ends_with('/'))
            .collect();
        let mut content_types = HashMap::new();
        if names.iter().any(|n| n == "[Content_Types].xml") {
            let xml = decode_xml(read_entry(&mut archive, "[Content_Types].xml")?);
            let mut overrides = HashMap::new();
            for el in elements(&xml, "Override") {
                if let (Some(part), Some(ctype)) = (attr(el, "PartName"), attr(el, "ContentType")) {
                    overrides.insert(part.trim_start_matches('/').to_string(), ctype.to_string());
                }
            }
            let defaults: Vec<(String, String)> = elements(&xml, "Default")
                .into_iter()
                .filter_map(|el| Some((attr(el, "Extension")?.to_string(), attr(el, "ContentType")?.to_string())))
                .collect();
            for name in &names {
                let mut ctype = overrides.get(name).cloned();
                if ctype.is_none() && name != "[Content_Types].xml" {
                    ctype = defaults
                        .iter()
                        .find(|(ext, _)| name.to_ascii_lowercase().ends_with(&ext.to_ascii_lowercase()))
                        .map(|(_, c)| c.clone());
                }
                content_types.insert(name.clone(), ctype.unwrap_or_default());
            }
        }
        drop(archive);
        Ok(Package { bytes, names, content_types })
    }

    pub fn content_type(&self, name: &str) -> &str {
        self.content_types.get(name).map(String::as_str).unwrap_or("")
    }

    pub fn file_type(&self) -> Result<FileType, Error> {
        let has = |list: &[&str]| self.names.iter().any(|n| list.contains(&self.content_type(n)));
        if has(ct::DOCX_MAINS) {
            Ok(FileType::Docx)
        } else if has(&[ct::PPTX_SLIDE, ct::PPTX_PRESENTATION, ct::PPTM_PRESENTATION]) {
            Ok(FileType::Pptx)
        } else if has(ct::XLSX_PARTS) {
            Err(Error::single(TemplateError::new(
                "filetype_not_handled",
                "The filetype \"xlsx\" is not handled by docx-templater",
                "The file you are trying to generate is of type \"xlsx\", but only docx and pptx formats are handled",
            )))
        } else {
            Err(Error::single(TemplateError::new(
                "filetype_not_identified",
                "The filetype for this file could not be identified, is this file corrupted ?",
                "The filetype for this file could not be identified, is this file corrupted ? Zip file contains no docx or pptx main document",
            )))
        }
    }

    /// Names of the parts that get templated, in archive order.
    pub fn templated_parts(&self, file_type: FileType) -> Vec<String> {
        let list = match file_type {
            FileType::Docx => ct::DOCX_PARTS,
            FileType::Pptx => ct::PPTX_PARTS,
        };
        self.names
            .iter()
            .filter(|n| {
                let c = self.content_type(n);
                list.contains(&c) || ct::COMMON.contains(&c)
            })
            .cloned()
            .collect()
    }

    /// Write a new archive where the entries in `replaced` get new content.
    pub fn write(&self, replaced: &HashMap<String, String>) -> Result<Vec<u8>, Error> {
        let mut archive = ZipArchive::new(Cursor::new(self.bytes.as_slice()))?;
        let mut writer = ZipWriter::new(Cursor::new(Vec::with_capacity(self.bytes.len())));
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for i in 0..archive.len() {
            let file = archive.by_index_raw(i)?;
            let name = file.name()?.to_string();
            match replaced.get(&name) {
                Some(content) => {
                    drop(file);
                    writer.start_file(name, options)?;
                    writer.write_all(content.as_bytes())?;
                }
                None => writer.raw_copy_file(file)?,
            }
        }
        Ok(writer.finish()?.into_inner())
    }
}
