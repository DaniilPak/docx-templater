//! Port of docxtemplater's `render.js`, `scope-manager.js`, the render parts
//! of `modules/render.js`, `modules/loop.js`, `modules/rawxml.js`, and the
//! post-render steps (`space-preserve.js` postrender, `join-uncorrupt.js`).

use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;

use crate::error::TemplateError;
use crate::filetype::{FileType, FileTypeConfig, ct};
use crate::options::{NullContext, Options, ParserContext};
use crate::part::{
    Module, Part, Placeholder, Position, get_single_attribute, is_ending, is_starting, is_whitespace,
    set_single_attribute, utf8_to_word,
};

#[derive(Clone)]
pub(crate) struct Scope<'a> {
    pub list: Vec<&'a Value>,
    pub path: Vec<String>,
    pub items: Vec<usize>,
    pub lengths: Vec<usize>,
}

impl<'a> Scope<'a> {
    pub fn root(data: &'a Value) -> Self {
        Scope { list: vec![data], path: Vec::new(), items: Vec::new(), lengths: Vec::new() }
    }

    fn sub<'b>(&self, value: &'b Value, tag: &str, i: usize, len: usize) -> Scope<'b>
    where
        'a: 'b,
    {
        let mut list: Vec<&'b Value> = self.list.clone();
        list.push(value);
        let mut path = self.path.clone();
        path.push(tag.to_string());
        let mut items = self.items.clone();
        items.push(i);
        let mut lengths = self.lengths.clone();
        lengths.push(len);
        Scope { list, path, items, lengths }
    }
}

/// Default tag evaluator: `.`/`this` is the current scope, otherwise an exact
/// key lookup, falling back to a dotted path (`user.address.city`, `items.0`).
pub(crate) fn default_get<'v>(tag: &str, scope: &'v Value) -> Option<&'v Value> {
    let tag = tag.trim();
    if tag == "." || tag == "this" {
        return Some(scope);
    }
    let tag = tag.strip_prefix("this.").unwrap_or(tag);
    if let Value::Object(map) = scope
        && let Some(v) = map.get(tag)
    {
        return Some(v);
    }
    if !tag.contains('.') {
        return None;
    }
    let mut cur = scope;
    for seg in tag.split('.') {
        let seg = seg.trim();
        cur = match cur {
            Value::Object(map) => map.get(seg)?,
            Value::Array(arr) => arr.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// JavaScript truthiness of a JSON value (with empty arrays falsy, like
/// docxtemplater's `isValueFalsy`).
fn is_falsy(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f == 0.0 || f.is_nan()),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(_) => false,
    }
}

/// JavaScript's `String(value)` for JSON values.
pub(crate) fn js_to_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e21 { format!("{f:.0}") } else { f.to_string() }
            }
        }
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(js_to_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn is_corrupt_char(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}')
}

pub(crate) struct Renderer<'o> {
    pub opts: &'o Options,
    pub file_type: FileType,
    pub cfg: &'static FileTypeConfig,
    pub linebreaks: bool,
    pub sects: &'o [Vec<Part>],
    pub out: Vec<String>,
    pub errors: Vec<TemplateError>,
    recorded_run: String,
    record_run: bool,
    in_xfrm: bool,
    last_ext: Option<usize>,
    row_id_counts: HashMap<usize, i64>,
    row_id_override: HashMap<usize, String>,
}

impl<'o> Renderer<'o> {
    pub fn new(opts: &'o Options, file_type: FileType, content_type: &str, sects: &'o [Vec<Part>]) -> Self {
        Renderer {
            opts,
            file_type,
            cfg: file_type.config(),
            linebreaks: opts.linebreaks && !ct::NON_LINE_BREAKS.contains(&content_type),
            sects,
            out: Vec::new(),
            errors: Vec::new(),
            recorded_run: String::new(),
            record_run: false,
            in_xfrm: false,
            last_ext: None,
            row_id_counts: HashMap::new(),
            row_id_override: HashMap::new(),
        }
    }

    fn get_value<'a>(&self, ph: &Placeholder, scope: &Scope<'a>) -> Result<Option<Cow<'a, Value>>, TemplateError> {
        for num in (0..scope.list.len()).rev() {
            let result: Option<Cow<'a, Value>> = match &self.opts.parser {
                Some(parser) => {
                    let ctx = ParserContext {
                        scope: scope.list[num],
                        scope_list: &scope.list,
                        num,
                        scope_path: &scope.path,
                        scope_path_item: &scope.items,
                        scope_path_length: &scope.lengths,
                    };
                    parser(&ph.value, &ctx)
                        .map_err(|e| {
                            TemplateError::new(
                                "scopeparser_execution_failed",
                                "Scope parser execution failed",
                                format!("The scope parser for the tag \"{}\" failed to execute: {e}", ph.value),
                            )
                            .xtag(ph.value.clone())
                            .offset(ph.offset)
                        })?
                        .map(Cow::Owned)
                }
                None => default_get(&ph.value, scope.list[num]).map(Cow::Borrowed),
            };
            match result {
                Some(v) if !v.is_null() => return Ok(Some(v)),
                _ if num > 0 => continue,
                _ => return Ok(None),
            }
        }
        Ok(None)
    }

    fn null_value(&self, ph: &Placeholder, scope: &Scope<'_>) -> Value {
        match &self.opts.null_getter {
            Some(f) => {
                f(&NullContext { tag: &ph.value, raw: &ph.raw, module: ph.module.name(), scope_path: &scope.path })
            }
            None => {
                if ph.module == Module::None {
                    Value::String("undefined".into())
                } else {
                    Value::String(String::new())
                }
            }
        }
    }

    fn record_runs(&mut self, part: &Part) {
        let run = if self.file_type == FileType::Docx { "w:r" } else { "a:r" };
        let rpr = if self.file_type == FileType::Docx { "w:rPr" } else { "a:rPr" };
        match part {
            Part::Tag(t) if t.tag == run => self.recorded_run.clear(),
            Part::Tag(t) if t.tag == rpr => {
                if t.position == Position::Start {
                    self.record_run = true;
                    self.recorded_run.push_str(&t.value);
                }
                if matches!(t.position, Position::End | Position::SelfClosing) {
                    self.recorded_run.push_str(&t.value);
                    self.record_run = false;
                }
            }
            p if self.record_run => self.recorded_run.push_str(p.xml_value()),
            _ => {}
        }
    }

    pub fn render(&mut self, parts: &[Part], scope: &Scope<'_>) {
        for part in parts {
            match part {
                Part::Tag(t) => {
                    if t.tag == "p:xfrm" {
                        self.in_xfrm = t.position == Position::Start;
                    }
                    if t.tag == "a:ext" && self.in_xfrm {
                        self.last_ext = Some(self.out.len());
                        self.out.push(t.value.clone());
                        continue;
                    }
                    if self.linebreaks {
                        self.record_runs(part);
                    }
                    match self.row_id_override.get(&t.l_index) {
                        Some(v) => self.out.push(v.clone()),
                        None => self.out.push(t.value.clone()),
                    }
                }
                Part::Content(c) => {
                    if self.linebreaks {
                        self.record_runs(part);
                    }
                    self.out.push(c.value.clone());
                }
                Part::Delimiter(_) => {}
                Part::Placeholder(ph) => match ph.module {
                    Module::None => self.render_simple(ph, scope),
                    Module::Loop => self.render_loop(ph, scope),
                    Module::RawXml => self.render_raw(ph, scope),
                },
            }
        }
    }

    fn render_simple(&mut self, ph: &Placeholder, scope: &Scope<'_>) {
        let value = match self.get_value(ph, scope) {
            Ok(Some(v)) => v.into_owned(),
            Ok(None) => self.null_value(ph, scope),
            Err(e) => {
                self.errors.push(e);
                return;
            }
        };
        if let Value::String(s) = &value {
            if self.opts.strip_invalid_xml_chars {
                let cleaned: String = s.chars().filter(|&c| !is_corrupt_char(c)).collect();
                return self.push_string(cleaned);
            }
            if s.chars().any(is_corrupt_char) {
                self.errors.push(
                    TemplateError::new(
                        "invalid_xml_characters",
                        "There are some XML corrupt characters",
                        format!("There are some corrupt characters for the field \"{}\"", ph.value),
                    )
                    .xtag(ph.value.clone())
                    .offset(ph.offset),
                );
                return;
            }
            let s = s.clone();
            return self.push_string(s);
        }
        self.out.push(utf8_to_word(&js_to_string(&value)));
    }

    fn push_string(&mut self, s: String) {
        if !self.linebreaks {
            self.out.push(utf8_to_word(&s));
            return;
        }
        let p = self.cfg.prefix;
        let br = if self.file_type == FileType::Docx { "<w:r><w:br/></w:r>" } else { "<a:br/>" };
        let preserve = if self.file_type == FileType::Docx { " xml:space=\"preserve\"" } else { "" };
        let lines: Vec<&str> = s.split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            self.out.push(utf8_to_word(line));
            if i + 1 < lines.len() {
                self.out.push(format!("</{p}:t></{p}:r>{br}<{p}:r>{}<{p}:t{preserve}>", self.recorded_run));
            }
        }
    }

    fn render_raw(&mut self, ph: &Placeholder, scope: &Scope<'_>) {
        let value = match self.get_value(ph, scope) {
            Ok(Some(v)) => v.into_owned(),
            Ok(None) => self.null_value(ph, scope),
            Err(e) => {
                self.errors.push(e);
                return;
            }
        };
        if is_falsy(&value) {
            return;
        }
        match value {
            Value::String(s) => self.out.push(s),
            _ => self.errors.push(
                TemplateError::new(
                    "invalid_raw_xml_value",
                    "Non string values are not allowed for rawXML tags",
                    format!(
                        "The value of the raw tag : \"{}{}{}\" is not a string",
                        self.opts.delimiters.start, ph.raw, self.opts.delimiters.end
                    ),
                )
                .xtag(ph.value.clone())
                .offset(ph.offset),
            ),
        }
    }

    fn render_loop(&mut self, ph: &Placeholder, scope: &Scope<'_>) {
        // Unpaired loop tags only survive with `allow_unbalanced_loops`.
        let Some(ld) = ph.loop_data.as_deref() else { return };
        let sub = &ld.subparsed;
        let tag_height: i64 = match sub.first() {
            Some(Part::Tag(t)) if t.tag == "a:tr" => {
                get_single_attribute(&t.value, "h").and_then(|h| h.parse().ok()).unwrap_or(0)
            }
            _ => 0,
        };
        let mut height_offset = -tag_height;
        let inside_paragraph_loop =
            sub.iter().find(|p| !p.is_any_content()).is_some_and(|p| p.tag_name() != Some("w:t"));

        let got = match self.get_value(ph, scope) {
            Ok(v) => v,
            Err(e) => {
                self.errors.push(e);
                return;
            }
        };
        let nulled;
        let value: &Value = match &got {
            Some(v) => v.as_ref(),
            None => {
                nulled = self.null_value(ph, scope);
                &nulled
            }
        };
        let current = *scope.list.last().expect("root scope");
        let mut iterations: Vec<(&Value, usize, usize)> = Vec::new();
        let result = if is_falsy(value) {
            if ph.inverted {
                iterations.push((current, 0, 1));
            }
            ph.inverted
        } else if let Value::Array(arr) = value {
            if !ph.inverted {
                iterations.extend(arr.iter().enumerate().map(|(i, v)| (v, i, arr.len())));
            }
            true
        } else if value.is_object() {
            if !ph.inverted {
                iterations.push((value, 0, 1));
            }
            !ph.inverted
        } else {
            if !ph.inverted {
                iterations.push((current, 0, 1));
            }
            !ph.inverted
        };

        for (item, i, len) in iterations {
            height_offset += tag_height;
            let sub_scope = scope.sub(item, &ph.value, i, len);
            for pp in sub {
                if pp.is_tag_start("a16:rowId")
                    && let Part::Tag(t) = pp
                {
                    let count = self.row_id_counts.entry(t.l_index).or_insert(0);
                    let base: i64 = get_single_attribute(&t.value, "val").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let new_value = set_single_attribute(&t.value, "val", &(base + *count).to_string());
                    *count += 1;
                    self.row_id_override.insert(t.l_index, new_value);
                }
            }
            let start = self.out.len();
            self.render(sub, &sub_scope);
            if self.file_type != FileType::Docx {
                continue;
            }
            let mut rendered = self.out.split_off(start);
            if ld.has_page_break && i + 1 == len && inside_paragraph_loop {
                add_page_break_at_end(&mut rendered);
            }
            let is_not_first = sub_scope.items.iter().any(|&x| x != 0);
            if is_not_first {
                if ld.sect_pr_count == 1 {
                    rendered.retain(|p| !p.starts_with("<w:headerReference") && !p.starts_with("<w:footerReference"));
                }
                if ld.add_continuous_type {
                    add_continuous_type(&mut rendered);
                }
            } else if let Some(idx) = ld.add_next_page
                && let Some(sect) = self.sects.get(idx)
            {
                let s: String = sect.iter().map(Part::xml_value).collect();
                rendered.insert(0, format!("<w:p><w:pPr>{s}</w:pPr></w:p>"));
            }
            if ld.add_next_page.is_some() {
                add_page_break_at_end(&mut rendered);
            }
            if ld.has_page_break_beginning && inside_paragraph_loop {
                rendered.insert(0, "<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>".to_string());
            }
            self.out.append(&mut rendered);
        }

        if !result {
            if !ld.last_paragraph_sect_pr.is_empty() {
                let s = &ld.last_paragraph_sect_pr;
                if ld.paragraph_loop {
                    self.out.push(format!("<w:p><w:pPr>{s}</w:pPr></w:p>"));
                } else {
                    self.out.push(format!("</w:t></w:r></w:p><w:p><w:pPr>{s}</w:pPr><w:r><w:t>"));
                }
                return;
            }
            if ld.has_page_break && inside_paragraph_loop {
                self.out.push("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>".to_string());
            }
            return;
        }
        if height_offset != 0
            && let Some(idx) = self.last_ext
            && let Some(ext) = self.out.get(idx)
        {
            let cy: i64 = get_single_attribute(ext, "cy").and_then(|v| v.parse().ok()).unwrap_or(0);
            self.out[idx] = set_single_attribute(ext, "cy", &(cy + height_offset).to_string());
        }
    }
}

fn add_page_break_at_end(parts: &mut Vec<String>) {
    if parts.last().is_some_and(|p| p == "</w:p>") {
        let j = parts.len() - 1;
        parts.insert(j, "<w:r><w:br w:type=\"page\"/></w:r>".to_string());
    } else {
        parts.push("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>".to_string());
    }
}

fn add_continuous_type(parts: &mut Vec<String>) {
    let mut stop = false;
    let mut in_sect = false;
    let mut i = 0;
    while i < parts.len() {
        if !stop && parts[i].starts_with("<w:sectPr") {
            in_sect = true;
        }
        if in_sect {
            if parts[i].starts_with("<w:type") {
                stop = true;
            }
            if !stop && parts[i].starts_with("</w:sectPr") {
                parts.insert(i, "<w:type w:val=\"continuous\"/>".to_string());
                i += 1;
            }
        }
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// Post-render

const WT_PRESERVE: &str = "<w:t xml:space=\"preserve\">";

/// `space-preserve.js` postrender: `<w:t xml:space="preserve"></w:t>` → `<w:t/>`.
pub(crate) fn space_preserve_postrender(parts: &mut [String]) {
    let mut last_non_empty = 0usize;
    let mut ends_with_preserve = false;
    for i in 0..parts.len() {
        if parts[i].is_empty() {
            continue;
        }
        if ends_with_preserve && parts[i].starts_with("</w:t>") {
            let prev = &mut parts[last_non_empty];
            prev.truncate(prev.len() - WT_PRESERVE.len());
            prev.push_str("<w:t/>");
            parts[i].drain(..6);
        }
        ends_with_preserve = parts[i].ends_with(WT_PRESERVE);
        last_non_empty = i;
    }
}

fn add_empty_paragraph_after_table(parts: &mut [String]) {
    let mut last_non_empty_ends_tbl = false;
    for p in parts.iter_mut() {
        if is_whitespace(p) || p.starts_with("<w:bookmarkEnd") {
            continue;
        }
        if last_non_empty_ends_tbl
            && !p.starts_with("<w:p")
            && !p.starts_with("<w:tbl")
            && !p.starts_with("<w:sectPr")
            && !p.starts_with("</w:ftr>")
            && !p.starts_with("</w:hdr>")
        {
            p.insert_str(0, "<w:p/>");
        }
        last_non_empty_ends_tbl = p.ends_with("</w:tbl>");
    }
}

/// Port of `join-uncorrupt.js`: fixes cells without paragraphs, rows without
/// cells, tables without rows... that loops can produce.
pub(crate) fn join_uncorrupt(parts: &mut [String], cfg: &FileTypeConfig, content_type: &str) {
    if ct::DOCX_PARTS.contains(&content_type) {
        add_empty_paragraph_after_table(parts);
    }
    let mut currently_collecting: Option<usize> = None;
    let mut start_index = 0usize;
    for (j, rule) in cfg.tag_should_contain.iter().enumerate() {
        let mut collected: Option<Vec<String>> = None;
        for i in 0..parts.len() {
            if currently_collecting == Some(j) {
                if is_ending(&parts[i], rule.tag) {
                    currently_collecting = None;
                    if let Some(dp) = rule.drop_parent {
                        if let Some(start) = (1..=start_index).rev().find(|&k| is_starting(&parts[k], dp)) {
                            for part in &mut parts[start..] {
                                let end = is_ending(part, dp);
                                part.clear();
                                if end {
                                    break;
                                }
                            }
                        }
                    } else {
                        let ending = parts[i].clone();
                        for p in &mut parts[start_index..=i] {
                            p.clear();
                        }
                        if !rule.drop {
                            let mut s = collected.take().unwrap_or_default().concat();
                            s.push_str(rule.value);
                            s.push_str(&ending);
                            parts[i] = s;
                        }
                    }
                    collected = None;
                    continue;
                } else {
                    if let Some(c) = collected.as_mut() {
                        c.push(parts[i].clone());
                    }
                    if rule.should_contain.iter().any(|sc| is_starting(&parts[i], sc)) {
                        currently_collecting = None;
                        collected = None;
                    }
                }
            }
            let part = &parts[i];
            if currently_collecting.is_none() && is_starting(part, rule.tag) && !part[1..].contains('<') {
                if part.as_bytes().get(part.len().wrapping_sub(2)) == Some(&b'/') {
                    parts[i].clear();
                } else {
                    start_index = i;
                    currently_collecting = Some(j);
                    collected = if rule.drop || rule.drop_parent.is_some() { None } else { Some(vec![part.clone()]) };
                }
            }
        }
    }
}

/// Renumber `<wp:docPr id="...">` sequentially (fix-doc-pr-corruption.js).
pub(crate) fn fix_doc_pr(content: &str, next_id: &mut u64) -> Option<String> {
    let parts = crate::lexer::xmlparse(content, &[], &["wp:docPr"]).ok()?;
    if parts.len() <= 1 {
        return None;
    }
    let mut out = String::with_capacity(content.len());
    for p in &parts {
        if p.is_tag_start("wp:docPr") {
            out.push_str(&set_single_attribute(p.xml_value(), "id", &next_id.to_string()));
            *next_id += 1;
        } else {
            out.push_str(p.xml_value());
        }
    }
    Some(out)
}
