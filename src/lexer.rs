//! Port of docxtemplater's `lexer.js`.
//!
//! The XML is first split into "interesting" tags and opaque content
//! (`xmlparse`). Then the text of every text tag (`<w:t>`, `<a:t>`...) is
//! concatenated and the delimiters are searched in that concatenation, which
//! is what makes tags split across several runs work.

use std::collections::HashMap;

use crate::error::{self, TemplateError};
use crate::filetype::FileTypeConfig;
use crate::options::{Delimiters, Syntax};
use crate::part::{Content, Delimiter, Part, Position, Tag, find_from, safe_slice, utf8_to_word, word_to_utf8};

fn get_tag(tag: &str) -> (&str, Position) {
    let b = tag.as_bytes();
    let n = b.len();
    let space = tag.find(' ');
    let (start, end, position) = if n >= 2 && b[n - 2] == b'/' {
        (1, space.unwrap_or(n - 2), Position::SelfClosing)
    } else if n >= 2 && b[1] == b'/' {
        (2, space.unwrap_or(n - 1), Position::End)
    } else {
        (1, space.unwrap_or(n.saturating_sub(1)), Position::Start)
    };
    (tag.get(start..end).unwrap_or(""), position)
}

/// Split `content` into tags of interest and opaque content.
pub(crate) fn xmlparse(content: &str, text_tags: &[&str], other_tags: &[&str]) -> Result<Vec<Part>, TemplateError> {
    let mut wanted: HashMap<&str, bool> = HashMap::new();
    for t in text_tags {
        wanted.insert(t, true);
    }
    for t in other_tags {
        wanted.insert(t, false);
    }
    let bytes = content.as_bytes();
    let find = |b: u8, from: usize| bytes.get(from..).and_then(|s| s.iter().position(|&x| x == b)).map(|i| i + from);

    let mut parsed = Vec::new();
    let mut cursor = 0;
    let mut last_end = 0;
    while cursor < bytes.len() {
        let Some(offset) = find(b'<', cursor) else { break };
        let next_opening = find(b'<', offset + 1);
        let close = match find(b'>', offset) {
            Some(c) if next_opening.is_none_or(|n| c < n) => c,
            _ => return Err(error::invalid_xml(offset)),
        };
        cursor = close;
        let tag_text = &content[offset..=close];
        let (name, position) = get_tag(tag_text);
        let Some(&text) = wanted.get(name) else { continue };
        if offset > last_end {
            parsed.push(Part::Content(Content {
                value: content[last_end..offset].to_string(),
                inside: false,
                l_index: 0,
            }));
        }
        parsed.push(Part::Tag(Tag { tag: name.to_string(), position, text, value: tag_text.to_string(), l_index: 0 }));
        last_end = close + 1;
    }
    if content.len() > last_end {
        parsed.push(Part::Content(Content { value: content[last_end..].to_string(), inside: false, l_index: 0 }));
    }
    Ok(parsed)
}

fn decode_content_parts(parts: &mut [Part]) -> Result<(), TemplateError> {
    let mut in_text = false;
    for part in parts.iter_mut() {
        if part.is_text_start() {
            if in_text {
                return Err(error::malformed_xml());
            }
            in_text = true;
        } else if part.is_text_end() {
            if !in_text {
                return Err(error::malformed_xml());
            }
            in_text = false;
        }
        if let Part::Content(c) = part {
            c.inside = in_text;
            if in_text && c.value.contains('>') {
                c.value = c.value.replace('>', "&gt;");
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct DelimMatch {
    offset: usize,
    position: Position,
    length: usize,
    change: bool,
}

#[derive(PartialEq, Eq)]
enum Cmp {
    None,
    Equal,
    Start,
    End,
}

fn compare_offsets(start: Option<usize>, end: Option<usize>) -> Cmp {
    match (start, end) {
        (None, None) => Cmp::None,
        (Some(s), Some(e)) if s == e => Cmp::Equal,
        (Some(_), None) => Cmp::Start,
        (None, Some(_)) => Cmp::End,
        (Some(s), Some(e)) => {
            if s < e {
                Cmp::Start
            } else {
                Cmp::End
            }
        }
    }
}

fn split_delimiters(inside: &str) -> Result<(String, String), TemplateError> {
    let parts: Vec<&str> = inside.split(' ').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(error::change_delimiters_invalid());
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

fn get_all_delimiter_indexes(
    full: &str,
    delimiters: &Delimiters,
    syntax: &Syntax,
    errors: &mut Vec<TemplateError>,
) -> Vec<DelimMatch> {
    let mut indexes = Vec::new();
    let mut start = utf8_to_word(&delimiters.start);
    let mut end = utf8_to_word(&delimiters.end);
    if start.is_empty() || end.is_empty() {
        return indexes;
    }
    let mut offset: Option<usize> = None;
    let mut inside_tag = false;
    loop {
        let from = offset.map_or(0, |o| o + 1);
        let so = find_from(full, &start, from);
        let eo = find_from(full, &end, from);
        let mut cmp = compare_offsets(so, eo);
        if cmp == Cmp::Equal {
            cmp = if inside_tag { Cmp::End } else { Cmp::Start };
        }
        let (pos, len, at) = match cmp {
            Cmp::None | Cmp::Equal => return indexes,
            Cmp::End => {
                inside_tag = false;
                (Position::End, end.len(), eo.expect("end found"))
            }
            Cmp::Start => {
                inside_tag = true;
                (Position::Start, start.len(), so.expect("start found"))
            }
        };
        offset = Some(at);
        if let Some(prefix) = syntax.change_delimiter_prefix.as_deref().filter(|p| !p.is_empty())
            && pos == Position::Start
            && full[at + start.len()..].starts_with(prefix)
        {
            indexes.push(DelimMatch { offset: at, position: Position::Start, length: start.len(), change: true });
            let Some(next_equal) = find_from(full, prefix, at + start.len() + prefix.len()) else {
                errors.push(error::change_delimiters_invalid().offset(at));
                indexes.pop();
                return indexes;
            };
            let Some(next_end) = find_from(full, &end, next_equal + prefix.len()) else {
                errors.push(error::change_delimiters_invalid().offset(at));
                indexes.pop();
                return indexes;
            };
            indexes.push(DelimMatch { offset: next_end, position: Position::End, length: end.len(), change: true });
            let inside = safe_slice(full, at + start.len() + prefix.len(), next_equal);
            match split_delimiters(inside) {
                Ok((s, e)) => {
                    start = s;
                    end = e;
                }
                Err(e) => {
                    errors.push(e.offset(at));
                    return indexes;
                }
            }
            offset = Some(next_end);
            continue;
        }
        indexes.push(DelimMatch { offset: at, position: pos, length: len, change: false });
    }
}

fn get_delimiter_errors(
    matches: Vec<DelimMatch>,
    full: &str,
    syntax: &Syntax,
    errors: &mut Vec<TemplateError>,
) -> Vec<DelimMatch> {
    let mut in_delimiter = false;
    let mut last_offset: usize = 0;
    let mut last_length: Option<usize> = None;
    let mut acc: Vec<DelimMatch> = Vec::new();
    let substr = |start: usize, len: usize| safe_slice(full, start, start.saturating_add(len)).to_string();

    for curr in matches {
        let mut xtag = substr(last_offset, curr.offset.saturating_sub(last_offset));
        let adjacent = last_length.is_some_and(|l| last_offset + l == curr.offset);
        if in_delimiter && curr.position == Position::Start {
            if adjacent {
                let l = last_length.unwrap_or(0);
                xtag = substr(last_offset, curr.offset - last_offset + l + 4);
                if !syntax.allow_unclosed_tag {
                    errors.push(error::duplicate_open_tag(&xtag, last_offset));
                    last_offset = curr.offset;
                    last_length = Some(curr.length);
                    acc.push(curr);
                    continue;
                }
            }
            if !syntax.allow_unclosed_tag {
                errors.push(error::unclosed_tag(&word_to_utf8(&xtag), last_offset));
            }
            acc.pop();
        }
        if !in_delimiter && curr.position == Position::End {
            if syntax.allow_unopened_tag {
                continue;
            }
            if adjacent {
                let l = last_length.unwrap_or(0);
                let s = last_offset.saturating_sub(4);
                xtag = substr(s, curr.offset - last_offset + l + 4);
                errors.push(error::duplicate_close_tag(&xtag, last_offset));
            } else {
                errors.push(error::unopened_tag(&xtag, curr.offset));
            }
            last_offset = curr.offset;
            last_length = Some(curr.length);
            acc.push(curr);
            continue;
        }
        in_delimiter = curr.position == Position::Start;
        last_offset = curr.offset;
        last_length = Some(curr.length);
        acc.push(curr);
    }
    if in_delimiter {
        let xtag = safe_slice(full, last_offset, full.len()).to_string();
        if !syntax.allow_unclosed_tag {
            errors.push(error::unclosed_tag(&word_to_utf8(&xtag), last_offset));
        }
        acc.pop();
    }
    acc
}

enum Sub {
    Content(String),
    Delimiter(Position, usize),
}

fn parse_delimiters(
    contents: &[&str],
    delimiters: &Delimiters,
    syntax: &Syntax,
    errors: &mut Vec<TemplateError>,
) -> Vec<Vec<Sub>> {
    let full: String = contents.concat();
    let matches = get_all_delimiter_indexes(&full, delimiters, syntax, errors);
    let delims = get_delimiter_errors(matches, &full, syntax, errors);

    let mut result = Vec::with_capacity(contents.len());
    let mut offset = 0usize;
    let mut cut_next = 0usize;
    let mut di = 0usize;
    let mut skipping = false;
    for content in contents {
        let range_end = offset + content.len();
        let mut parts = Vec::new();
        let mut cursor = 0usize;
        if cut_next > 0 {
            cursor = cut_next;
            cut_next = 0;
        }
        while di < delims.len() && offset <= delims[di].offset && delims[di].offset < range_end {
            let d = delims[di];
            di += 1;
            let rel = d.offset - offset;
            let value = if rel > cursor { safe_slice(content, cursor, rel) } else { "" };
            if d.change {
                if d.position == Position::Start {
                    if !value.is_empty() {
                        parts.push(Sub::Content(value.to_string()));
                    }
                    cursor = rel;
                    skipping = true;
                } else {
                    cursor = rel + d.length;
                    skipping = false;
                }
                continue;
            }
            if !value.is_empty() {
                parts.push(Sub::Content(value.to_string()));
                cursor += value.len();
            }
            parts.push(Sub::Delimiter(d.position, cursor + offset));
            cursor = rel + d.length;
        }
        if cursor > content.len() {
            cut_next = cursor - content.len();
        }
        if !skipping && cursor < content.len() {
            parts.push(Sub::Content(safe_slice(content, cursor, content.len()).to_string()));
        }
        result.push(parts);
        offset = range_end;
    }
    result
}

/// Lex an XML file: returns the token stream (tags, contents, delimiters)
/// and the delimiter errors.
pub(crate) fn lex(
    content: &str,
    cfg: &FileTypeConfig,
    delimiters: &Delimiters,
    syntax: &Syntax,
) -> Result<(Vec<Part>, Vec<TemplateError>), TemplateError> {
    let mut xmllexed = xmlparse(content, cfg.text_tags, cfg.lexed_tags)?;
    decode_content_parts(&mut xmllexed)?;
    let mut errors = Vec::new();
    let inner: Vec<&str> = xmllexed
        .iter()
        .filter_map(|p| match p {
            Part::Content(c) if c.inside => Some(c.value.as_str()),
            _ => None,
        })
        .collect();
    let mut subs = parse_delimiters(&inner, delimiters, syntax, &mut errors).into_iter();

    let mut lexed = Vec::with_capacity(xmllexed.len());
    let mut l_index = 0usize;
    for part in xmllexed {
        match part {
            Part::Content(c) if c.inside => {
                for sub in subs.next().unwrap_or_default() {
                    let p = match sub {
                        Sub::Content(value) => Part::Content(Content { value, inside: true, l_index }),
                        Sub::Delimiter(position, offset) => Part::Delimiter(Delimiter { position, offset, l_index }),
                    };
                    lexed.push(p);
                    l_index += 1;
                }
            }
            mut p => {
                match &mut p {
                    Part::Tag(t) => t.l_index = l_index,
                    Part::Content(c) => c.l_index = l_index,
                    _ => {}
                }
                lexed.push(p);
                l_index += 1;
            }
        }
    }
    Ok((lexed, errors))
}

/// Concatenated document text of a file (docxtemplater's `getFullText`).
pub(crate) fn full_text(content: &str, cfg: &FileTypeConfig) -> String {
    let Ok(parts) = xmlparse(content, cfg.text_tags, &[]) else { return String::new() };
    let mut out = String::new();
    let mut in_text = false;
    for p in &parts {
        if p.is_text_start() {
            in_text = true;
        } else if p.is_text_end() {
            in_text = false;
        } else if in_text && let Part::Content(c) = p {
            out.push_str(&c.value);
        }
    }
    word_to_utf8(&out.replace('\u{a0}', " "))
}
