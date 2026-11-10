//! Port of docxtemplater's `parser.js` `parse` step: turns delimiter tokens
//! into placeholders and moves the XML that was inside a tag (run
//! boundaries of a tag split across runs) right after the placeholder.

use crate::filetype::FileTypeConfig;
use crate::options::Syntax;
use crate::part::{ExpandTo, Location, Module, Part, Placeholder, Position, word_to_utf8};

fn module_parse(content: String, offset: usize, l_index: usize) -> Placeholder {
    let normalized = content.replace('\u{a0}', " ");
    let mut ph = Placeholder {
        value: content.clone(),
        raw: content,
        module: Module::None,
        location: None,
        inverted: false,
        expand_to: None,
        offset,
        l_index,
        end_l_index: l_index,
        loop_data: None,
        expanded: false,
    };
    if let Some(rest) = normalized.strip_prefix('#') {
        ph.module = Module::Loop;
        ph.location = Some(Location::Start);
        ph.expand_to = Some(ExpandTo::Auto);
        ph.value = rest.to_string();
    } else if let Some(rest) = normalized.strip_prefix('^') {
        ph.module = Module::Loop;
        ph.location = Some(Location::Start);
        ph.inverted = true;
        ph.expand_to = Some(ExpandTo::Auto);
        ph.value = rest.to_string();
    } else if let Some(rest) = normalized.strip_prefix('/') {
        ph.module = Module::Loop;
        ph.location = Some(Location::End);
        ph.value = rest.to_string();
    } else if let Some(rest) = normalized.strip_prefix('@') {
        ph.module = Module::RawXml;
        ph.value = rest.to_string();
    } else if let Some((expand, value)) = parse_dash(&normalized) {
        ph.module = Module::Loop;
        ph.location = Some(Location::Start);
        ph.expand_to = Some(ExpandTo::Tag(expand.to_string()));
        ph.value = value.to_string();
    }
    ph
}

/// `/^-([^\s]+)\s(.+)/`
fn parse_dash(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix('-')?;
    let end = rest.find(char::is_whitespace)?;
    if end == 0 {
        return None;
    }
    let expand = &rest[..end];
    let ws_len = rest[end..].chars().next()?.len_utf8();
    let value = &rest[end + ws_len..];
    let value = value.split('\n').next().unwrap_or("");
    if value.is_empty() {
        return None;
    }
    Some((expand, value))
}

pub(crate) fn parse(lexed: Vec<Part>, cfg: &FileTypeConfig, syntax: &Syntax) -> Vec<Part> {
    let mut parsed = Vec::with_capacity(lexed.len());
    let mut in_placeholder = false;
    let mut content = String::new();
    let mut start_offset = 0usize;
    let mut tail: Vec<Part> = Vec::new();
    for token in lexed {
        match token {
            Part::Delimiter(d) => {
                in_placeholder = d.position == Position::Start;
                if d.position == Position::End {
                    let ph = module_parse(word_to_utf8(&content), start_offset, d.l_index);
                    parsed.push(Part::Placeholder(Box::new(ph)));
                    parsed.append(&mut tail);
                } else {
                    tail.clear();
                    start_offset = d.offset;
                }
                content.clear();
            }
            token if !in_placeholder => parsed.push(token),
            Part::Content(c) if c.inside => content.push_str(&c.value),
            token => {
                if syntax.preserve_newlines_in_tags && (token.is_break_tag() || token.is_paragraph_start()) {
                    content.push('\n');
                }
                if let Some(tag) = token.tag_name()
                    && cfg.dropped_tags_inside_placeholder.contains(&tag)
                {
                    continue;
                }
                tail.push(token);
            }
        }
    }
    parsed
}
