//! Port of docxtemplater's postparse modules: `loop.js` (paragraph loops,
//! section breaks), `space-preserve.js`, `expand-pair-trait.js` +
//! `traits.js` (pairing loop tags and expanding them to table rows /
//! paragraphs) and `rawxml.js`.

use std::collections::HashMap;

use crate::error::TemplateError;
use crate::filetype::{ExpandTag, FileType, FileTypeConfig};
use crate::options::Options;
use crate::part::{
    Chunk, ExpandTo, Location, LoopData, Module, Part, Placeholder, Position, chunk_by, get_left_or_null,
    get_right_or_null, is_whitespace,
};

pub(crate) struct PostCtx<'a> {
    pub cfg: &'static FileTypeConfig,
    pub file_type: FileType,
    pub opts: &'a Options,
    pub expand_tags: Vec<&'static ExpandTag>,
    pub sects: &'a [Vec<Part>],
    pub errors: Vec<TemplateError>,
}

impl<'a> PostCtx<'a> {
    pub fn new(file_type: FileType, opts: &'a Options, sects: &'a [Vec<Part>]) -> Self {
        let cfg = file_type.config();
        let mut expand_tags: Vec<&'static ExpandTag> = cfg.expand_tags.iter().collect();
        if opts.paragraph_loop {
            expand_tags.extend(cfg.on_paragraph_loop.iter());
        }
        PostCtx { cfg, file_type, opts, expand_tags, sects, errors: Vec::new() }
    }

    fn with_delims(&self, raw: &str) -> String {
        format!("{}{}{}", self.opts.delimiters.start, raw, self.opts.delimiters.end)
    }
}

pub(crate) fn postparse(parts: Vec<Part>, mut base: Option<&mut Placeholder>, ctx: &mut PostCtx<'_>) -> Vec<Part> {
    let parts = loop_postparse(parts, base.as_deref_mut(), ctx);
    let parts = if ctx.file_type == FileType::Docx {
        space_preserve(parts, base.as_ref().map(|b| b.end_l_index))
    } else {
        parts
    };
    let parts = expand_pair(parts, ctx);
    raw_xml_expand(parts, ctx)
}

// ---------------------------------------------------------------------------
// Section helpers (loop.js)

/// All `<w:sectPr>` blocks of a (lexed) main document.
pub(crate) fn get_sect_pr(parts: &[Part]) -> Vec<Vec<Part>> {
    let mut sects: Vec<Vec<Part>> = Vec::new();
    let mut collecting = false;
    for part in parts {
        if part.is_tag_start("w:sectPr") {
            sects.push(Vec::new());
            collecting = true;
        }
        if collecting && let Some(last) = sects.last_mut() {
            last.push(part.clone());
        }
        if part.is_tag_end("w:sectPr") {
            collecting = false;
        }
    }
    sects
}

fn sect_pr_header_footer_change_count(parts: &[Part]) -> usize {
    let mut collect = false;
    let mut count = 0;
    for part in parts {
        if part.is_tag_start("w:sectPr") {
            collect = true;
        }
        if collect && matches!(part.tag_name(), Some("w:headerReference" | "w:footerReference")) {
            count += 1;
            collect = false;
        }
        if part.is_tag_end("w:sectPr") {
            collect = false;
        }
    }
    count
}

fn is_continuous(parts: &[Part]) -> bool {
    parts.iter().any(|p| p.is_tag_start("w:type") && p.xml_value().contains("continuous"))
}

fn is_next_page(parts: &[Part]) -> bool {
    parts.iter().any(|p| p.is_tag_start("w:type") && p.xml_value().contains("w:val=\"nextPage\""))
}

fn get_last_sect_pr(parsed: &[Part]) -> String {
    let mut sect: Vec<&str> = Vec::new();
    let mut in_sect = false;
    for part in parsed.iter().rev() {
        if part.is_tag_end("w:sectPr") {
            in_sect = true;
        }
        if part.is_tag_start("w:sectPr") {
            sect.insert(0, part.xml_value());
            in_sect = false;
        }
        if in_sect {
            sect.insert(0, part.xml_value());
        }
        if part.is_paragraph_start() {
            if !sect.is_empty() {
                return sect.concat();
            }
            break;
        }
    }
    String::new()
}

fn has_page_break(parts: &[Part]) -> bool {
    parts.iter().any(|p| p.tag_name() == Some("w:br") && p.xml_value().contains("w:type=\"page\""))
}

fn has_image(parts: &[Part]) -> bool {
    parts.iter().any(|p| p.tag_name() == Some("w:drawing"))
}

fn loop_postparse(parsed: Vec<Part>, base: Option<&mut Placeholder>, ctx: &PostCtx<'_>) -> Vec<Part> {
    let Some(base) = base else { return parsed };
    if ctx.file_type == FileType::Docx && !parsed.is_empty() {
        let base_l_index = base.l_index;
        let ld = base.loop_data.get_or_insert_with(Box::default);
        ld.sect_pr_count = sect_pr_header_footer_change_count(&parsed);
        for (index, sect) in ctx.sects.iter().enumerate() {
            let Some(first) = sect.first() else { continue };
            let s0 = first.l_index();
            if base_l_index < s0 {
                if index + 1 < ctx.sects.len() && is_continuous(&ctx.sects[index + 1]) {
                    ld.add_continuous_type = true;
                }
                break;
            }
            if parsed[0].l_index() < s0 && s0 < base_l_index {
                if is_next_page(sect) {
                    ld.add_next_page = Some(index);
                }
                break;
            }
        }
        ld.last_paragraph_sect_pr = get_last_sect_pr(&parsed);
    }
    let enclosed =
        !parsed.is_empty() && parsed[0].is_paragraph_start() && parsed.last().is_some_and(|p| p.is_paragraph_end());
    if base.expand_to != Some(ExpandTo::Auto) || base.module != Module::Loop || !enclosed {
        return parsed;
    }
    let ld = base.loop_data.get_or_insert_with(Box::default);
    ld.paragraph_loop = true;

    let mut level = 0i64;
    let chunks = chunk_by(&parsed, |p| {
        if p.is_paragraph_start() {
            level += 1;
            if level == 1 {
                return Chunk::Start;
            }
        }
        if p.is_paragraph_end() {
            level -= 1;
            if level == 0 {
                return Chunk::End;
            }
        }
        Chunk::None
    });
    let (Some(first), Some(last)) = (chunks.first().cloned(), chunks.last().cloned()) else { return parsed };
    let offset_of =
        |r: &std::ops::Range<usize>| if parsed[r.clone()].iter().any(Part::is_content) { 0 } else { r.len() };
    let mut first_offset = offset_of(&first);
    let mut last_offset = offset_of(&last);
    let ws = |p: &Part| matches!(p, Part::Content(c) if is_whitespace(&c.value));
    if first_offset > 0 && chunks.len() > 1 && ws(&parsed[chunks[1].start]) {
        first_offset += 1;
    }
    if last_offset > 0 && chunks.len() > 1 && ws(&parsed[chunks[chunks.len() - 2].end - 1]) {
        last_offset += 1;
    }
    ld.has_page_break_beginning = has_page_break(&parsed[first.clone()]);
    ld.has_page_break = has_page_break(&parsed[last.clone()]);
    if has_image(&parsed[first]) {
        first_offset = 0;
    }
    if has_image(&parsed[last]) {
        last_offset = 0;
    }
    let end = parsed.len().saturating_sub(last_offset);
    if first_offset >= end {
        return Vec::new();
    }
    let mut parsed = parsed;
    parsed.truncate(end);
    parsed.drain(..first_offset);
    parsed
}

// ---------------------------------------------------------------------------
// space-preserve.js

fn add_xml_preserve(chunk: &mut [Part], index: usize) {
    let next_is_end = chunk.get(index + 1).is_some_and(|p| p.xml_value() == "</w:t>");
    if next_is_end {
        return;
    }
    if let Some(Part::Tag(t)) = chunk.get_mut(index)
        && !t.value.contains("xml:space=\"preserve\"")
        && t.value.ends_with('>')
    {
        t.value.pop();
        t.value.push_str(" xml:space=\"preserve\">");
    }
}

fn space_preserve(parsed: Vec<Part>, base_end_l_index: Option<usize>) -> Vec<Part> {
    let mut out = Vec::with_capacity(parsed.len());
    let mut chunk: Vec<Part> = Vec::new();
    let mut in_text = false;
    let mut end_l_index = 0usize;
    let mut last_text_tag = 0usize;
    for part in parsed {
        if part.is_text_start() && part.tag_name() == Some("w:t") {
            in_text = true;
            last_text_tag = chunk.len();
        }
        if !in_text {
            out.push(part);
            continue;
        }
        chunk.push(part);
        if let Some(bel) = base_end_l_index
            && chunk.len() > 1
        {
            end_l_index = bel;
            add_xml_preserve(&mut chunk, 0);
        }
        let last = chunk.last().expect("just pushed");
        if let Part::Placeholder(ph) = last
            && chunk.len() > 1
        {
            end_l_index = ph.end_l_index;
            add_xml_preserve(&mut chunk, last_text_tag);
        }
        let last = chunk.last().expect("just pushed");
        if last.is_text_end() && last.l_index() > end_l_index {
            if end_l_index != 0 {
                add_xml_preserve(&mut chunk, last_text_tag);
            }
            out.append(&mut chunk);
            in_text = false;
            end_l_index = 0;
            last_text_tag = 0;
        }
    }
    out.append(&mut chunk);
    out
}

// ---------------------------------------------------------------------------
// expand-pair-trait.js / traits.js

#[derive(Clone, Debug)]
struct Trait {
    offset: usize,
    location: Location,
    value: String,
    tag_offset: usize,
    l_index: usize,
}

fn trait_match(start: Option<&Trait>, end: Option<&Trait>) -> bool {
    match (start, end) {
        (Some(s), Some(e)) => {
            (s.location == Location::Start && e.location == Location::End && s.value.trim() == e.value.trim())
                || e.value.trim().is_empty()
        }
        _ => false,
    }
}

fn unmatched_loop(t: &Trait) -> TemplateError {
    let (id, msg, word) = match t.location {
        Location::Start => ("unclosed_loop", "Unclosed loop", "unclosed"),
        Location::End => ("unopened_loop", "Unopened loop", "unopened"),
    };
    TemplateError::new(id, msg, format!("The loop with tag \"{}\" is {}", t.value, word))
        .xtag(t.value.clone())
        .offset(t.tag_offset)
}

fn closing_not_matching(open: &Trait, close: &Trait) -> TemplateError {
    TemplateError::new(
        "closing_tag_does_not_match_opening_tag",
        "Closing tag does not match opening tag",
        format!("The tag \"{}\" is closed by the tag \"{}\"", open.value, close.value),
    )
    .xtag(open.value.clone())
    .context(format!("{{#{}}}...{{/{}}}", open.value, close.value))
    .offset(open.tag_offset)
}

fn transformer(t: &mut Vec<Trait>, errors: &mut Vec<TemplateError>) {
    let mut i = 0;
    while i < t.len() {
        if t[i].location == Location::End {
            if i == 0 {
                errors.push(unmatched_loop(&t[0]));
                t.remove(0);
                return;
            }
            let end_i = i;
            let start_i = i - 1;
            if trait_match(t.get(start_i), t.get(end_i)) {
                t.remove(end_i);
                t.remove(start_i);
                return;
            }
            for off in 1..50 {
                let start_candidate = start_i.checked_sub(off).and_then(|k| t.get(k));
                if trait_match(start_candidate, t.get(end_i)) {
                    t.remove(end_i);
                    t.remove(start_i - off);
                    return;
                }
                if trait_match(t.get(start_i), t.get(end_i + off)) {
                    t.remove(end_i + off);
                    t.remove(start_i);
                    return;
                }
            }
            errors.push(closing_not_matching(&t[start_i], &t[end_i]));
            t.remove(end_i);
            t.remove(start_i);
            return;
        }
        i += 1;
    }
    for tr in t.iter() {
        errors.push(unmatched_loop(tr));
    }
    t.clear();
}

fn get_pairs(traits: &[Trait], errors: &mut Vec<TemplateError>) -> Vec<(usize, usize)> {
    let mut transformed = traits.to_vec();
    let before = errors.len();
    while !transformed.is_empty() {
        transformer(&mut transformed, errors);
    }
    if errors.len() > before {
        return Vec::new();
    }
    let mut pairs = Vec::new();
    let mut level_traits: HashMap<i64, usize> = HashMap::new();
    let mut count_open: i64 = 0;
    for (idx, tr) in traits.iter().enumerate() {
        let change = if tr.location == Location::Start { 1 } else { -1 };
        count_open += change;
        if change == 1 {
            level_traits.insert(count_open, idx);
        } else if count_open == 0
            && let Some(&start) = level_traits.get(&(count_open + 1))
        {
            pairs.push((start, idx));
        }
        if count_open < 0 {
            count_open = 0;
        }
    }
    pairs
}

fn get_list_xml_elements(parts: &[Part]) -> Vec<&str> {
    let mut result: Vec<&str> = Vec::new();
    for part in parts {
        let Part::Tag(t) = part else { continue };
        match t.position {
            Position::End => {
                let last_is_open = result.last().is_some_and(|l| l.get(1..).is_some_and(|s| s.starts_with(&*t.tag)));
                if last_is_open {
                    result.pop();
                } else {
                    result.push(&t.value);
                }
            }
            Position::Start => result.push(&t.value),
            Position::SelfClosing => {}
        }
    }
    result
}

fn xml_tag_name(el: &str) -> &str {
    let s = el.trim_start();
    let s = s.strip_prefix('<').unwrap_or(s);
    let s = s.strip_prefix('/').unwrap_or(s);
    let end = s.find(|c: char| !(c.is_ascii_alphabetic() || c == ':')).unwrap_or(s.len());
    if end == 0 { el } else { &s[..end] }
}

fn check_start_end(els: &[&str]) -> bool {
    if els.len() % 2 == 1 {
        return false;
    }
    (0..els.len() / 2).all(|i| xml_tag_name(els[i]) == xml_tag_name(els[els.len() - i - 1]))
}

fn loop_position_invalid(tag: &str, offset: usize) -> TemplateError {
    TemplateError::new(
        "loop_position_invalid",
        format!("The position of the loop tags \"{tag}\" would produce invalid XML"),
        format!(
            "The tags \"{tag}\" are misplaced in the document, for example one of them is in a table and the other one outside the table"
        ),
    )
    .xtag(tag)
    .offset(offset)
}

fn get_expand_to_default(
    parsed: &[Part],
    open: &Trait,
    close: &Trait,
    expand_tags: &[&'static ExpandTag],
) -> Result<Option<&'static str>, TemplateError> {
    let els = get_list_xml_elements(&parsed[open.offset..close.offset]);
    for et in expand_tags {
        if !els.iter().any(|e| e.starts_with('<') && e[1..].starts_with(et.contains)) {
            continue;
        }
        if et.only_text_in_tag {
            let left = get_left_or_null(parsed, et.contains, open.offset);
            let right = get_right_or_null(parsed, et.contains, close.offset);
            let (Some(left), Some(right)) = (left, right) else { continue };
            let sub = &parsed[left..right];
            let chunks = chunk_by(sub, |p| {
                if p.is_tag_start(et.contains) {
                    Chunk::Start
                } else if p.is_tag_end(et.contains) {
                    Chunk::End
                } else {
                    Chunk::None
                }
            });
            let (Some(first), Some(last)) = (chunks.first(), chunks.last()) else { continue };
            let first_content = sub[first.clone()].iter().filter(|p| p.is_content()).count();
            let last_content = sub[last.clone()].iter().filter(|p| p.is_content()).count();
            if first_content != 1 || last_content != 1 {
                continue;
            }
        }
        let mut open_count = 0i64;
        for el in &els {
            if xml_tag_name(el) == et.expand {
                if el.trim_start().starts_with("</") {
                    open_count -= 1;
                } else {
                    open_count += 1;
                }
            }
        }
        if open_count != 0 {
            return Err(loop_position_invalid(&open.value, open.tag_offset));
        }
        return Ok(Some(et.expand));
    }
    if !check_start_end(&els) {
        return Err(loop_position_invalid(&open.value, open.tag_offset));
    }
    Ok(None)
}

fn xml_tag_not_found(element: &str, position: &str, offset: usize) -> TemplateError {
    TemplateError::new(
        &format!("no_xml_tag_found_at_{position}"),
        format!("No tag \"{element}\" was found at the {position}"),
        format!("No tag \"{element}\" was found at the {position}"),
    )
    .offset(offset)
}

fn unbalanced(pair: (&Trait, &Trait), last: (&Trait, &Trait)) -> TemplateError {
    TemplateError::new(
        "unbalanced_loop_tags",
        "Unbalanced loop tag",
        format!(
            "Unbalanced loop tags {{#{}}}{{/{}}}{{#{}}}{{/{}}}",
            last.0.value, last.1.value, pair.0.value, pair.1.value
        ),
    )
    .xtag(pair.0.value.clone())
    .offset(last.0.tag_offset)
}

fn expand_pair(parsed: Vec<Part>, ctx: &mut PostCtx<'_>) -> Vec<Part> {
    let traits: Vec<Trait> = parsed
        .iter()
        .enumerate()
        .filter_map(|(i, p)| match p {
            Part::Placeholder(ph) if ph.module == Module::Loop && ph.loop_data.is_none() => {
                ph.location.map(|location| Trait {
                    offset: i,
                    location,
                    value: ph.value.clone(),
                    tag_offset: ph.offset,
                    l_index: ph.l_index,
                })
            }
            _ => None,
        })
        .collect();
    if traits.is_empty() {
        return parsed;
    }
    let mut errors = Vec::new();
    let pairs = get_pairs(&traits, &mut errors);
    let allow_unbalanced = ctx.opts.syntax.allow_unbalanced_loops;
    let mut last_right: Option<usize> = Some(0);
    let mut last_pair: Option<(usize, usize)> = None;
    let mut expanded: Vec<(usize, usize)> = Vec::with_capacity(pairs.len());
    for &(a, b) in &pairs {
        let (open, close) = (&traits[a], &traits[b]);
        let ph = parsed[open.offset].as_placeholder().expect("trait points to a placeholder");
        let expand_to: Option<String> = match &ph.expand_to {
            Some(ExpandTo::Auto) => match get_expand_to_default(&parsed, open, close, &ctx.expand_tags) {
                Ok(v) => v.map(str::to_string),
                Err(e) => {
                    errors.push(e);
                    None
                }
            },
            Some(ExpandTo::Tag(t)) => Some(t.clone()),
            None => None,
        };
        let (left, right) = match expand_to {
            None => (Some(open.offset), Some(close.offset)),
            Some(el) => {
                let left = get_left_or_null(&parsed, &el, open.offset);
                if left.is_none() {
                    errors.push(xml_tag_not_found(&el, "left", open.tag_offset));
                }
                let right = get_right_or_null(&parsed, &el, close.offset);
                if right.is_none() {
                    errors.push(xml_tag_not_found(&el, "right", close.tag_offset));
                }
                (left, right)
            }
        };
        if let (Some(l), Some(lr), Some((la, lb))) = (left, last_right, last_pair)
            && l < lr
            && !allow_unbalanced
        {
            errors.push(unbalanced((open, close), (&traits[la], &traits[lb])));
        }
        last_right = right;
        last_pair = Some((a, b));
        expanded.push((left.unwrap_or(open.offset), right.unwrap_or(close.offset)));
    }
    if !errors.is_empty() {
        ctx.errors.extend(errors);
        return parsed;
    }

    let mut slots: Vec<Option<Part>> = parsed.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(slots.len());
    let mut cur = 0usize;
    let mut inner: Vec<Part> = Vec::new();
    let mut base: Option<Box<Placeholder>> = None;
    for (i, slot) in slots.iter_mut().enumerate() {
        let part = slot.take().expect("each part is visited once");
        let in_pair = cur < pairs.len() && expanded[cur].0 <= i && i <= expanded[cur].1;
        if !in_pair {
            out.push(part);
            continue;
        }
        let open = &traits[pairs[cur].0];
        let close = &traits[pairs[cur].1];
        if expanded[cur].0 == i {
            inner = Vec::new();
        }
        if i == open.offset {
            if let Part::Placeholder(ph) = part {
                base = Some(ph);
            }
        } else if i != close.offset {
            inner.push(part);
        }
        if expanded[cur].1 == i {
            if let Some(mut b) = base.take() {
                b.loop_data = Some(Box::<LoopData>::default());
                let sub = postparse(std::mem::take(&mut inner), Some(&mut b), ctx);
                if let Some(ld) = b.loop_data.as_mut() {
                    ld.subparsed = sub;
                }
                b.end_l_index = close.l_index;
                b.location = None;
                b.expand_to = None;
                out.push(Part::Placeholder(b));
            }
            cur += 1;
            while cur < pairs.len() && expanded[cur].0 < i {
                cur += 1;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// rawxml.js (expandToOne)

fn raw_xml_expand(parsed: Vec<Part>, ctx: &mut PostCtx<'_>) -> Vec<Part> {
    let element = ctx.cfg.tag_raw_xml;
    struct Limit {
        left: usize,
        right: usize,
        index: usize,
        l_index: usize,
    }
    let mut limits = Vec::new();
    for (i, part) in parsed.iter().enumerate() {
        let Part::Placeholder(ph) = part else { continue };
        if ph.module != Module::RawXml || ph.expanded {
            continue;
        }
        let left = get_left_or_null(&parsed, element, i);
        let right = get_right_or_null(&parsed, element, i);
        match (left, right) {
            (Some(left), Some(right)) => limits.push(Limit { left, right, index: i, l_index: ph.l_index }),
            _ => ctx.errors.push(
                TemplateError::new(
                    "raw_tag_outerxml_invalid",
                    "Raw tag not in paragraph",
                    format!(
                        "The tag \"{}\" is not inside a paragraph, putting raw tags inside an inline loop is disallowed.",
                        ctx.with_delims(&ph.raw)
                    ),
                )
                .xtag(ph.value.clone())
                .offset(ph.offset),
            ),
        }
    }
    if limits.is_empty() {
        return parsed;
    }
    limits.sort_by(|a, b| a.left.cmp(&b.left).then(a.l_index.cmp(&b.l_index)));
    let mut parsed = parsed;
    let mut max_right: i64 = -1;
    let mut offset: i64 = 0;
    for k in 0..limits.len() {
        let lim = &limits[k];
        max_right = max_right.max(if k > 0 { limits[k - 1].right as i64 } else { 0 });
        if (lim.left as i64) < max_right {
            continue;
        }
        let left = (lim.left as i64 + offset) as usize;
        let right = (lim.right as i64 + offset) as usize;
        let index = (lim.index as i64 + offset) as usize;
        let has_other_content = (left + 1..right).any(|j| j != index && parsed[j].is_content());
        if has_other_content {
            let ph = parsed[index].as_placeholder().expect("raw placeholder");
            ctx.errors.push(
                TemplateError::new(
                    "raw_xml_tag_should_be_only_text_in_paragraph",
                    "Raw tag should be the only text in paragraph",
                    format!(
                        "The raw tag \"{}\" should be the only text in this paragraph. This means that this tag should not be surrounded by any text or spaces.",
                        ph.value
                    ),
                )
                .xtag(ph.value.clone())
                .offset(ph.offset),
            );
            continue;
        }
        let mut part = std::mem::replace(
            &mut parsed[index],
            Part::Content(crate::part::Content { value: String::new(), inside: false, l_index: 0 }),
        );
        if let Part::Placeholder(ph) = &mut part {
            ph.expanded = true;
        }
        parsed.splice(left..=right, std::iter::once(part));
        offset += 1 - (right as i64 + 1 - left as i64);
    }
    parsed
}
