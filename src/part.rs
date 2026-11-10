//! Token types produced by the lexer/parser and small helpers ported from
//! docxtemplater's `doc-utils.js`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Position {
    Start,
    End,
    SelfClosing,
}

/// An XML tag that the lexer cares about (text tags such as `<w:t>` and
/// structural tags such as `<w:p>`, `<w:tr>`...).
#[derive(Clone, Debug)]
pub(crate) struct Tag {
    pub tag: String,
    pub position: Position,
    /// Whether this is a text tag (`w:t`, `a:t`...).
    pub text: bool,
    pub value: String,
    pub l_index: usize,
}

/// Raw XML / text that sits between interesting tags.
#[derive(Clone, Debug)]
pub(crate) struct Content {
    pub value: String,
    /// `true` when the content is inside a text tag (it is document text).
    pub inside: bool,
    pub l_index: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct Delimiter {
    pub position: Position,
    pub offset: usize,
    pub l_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Module {
    /// A plain `{value}` placeholder.
    None,
    /// `{#loop}`, `{^inverted}`, `{/loop}`, `{-w:p loop}`.
    Loop,
    /// `{@raw}`.
    RawXml,
}

impl Module {
    pub fn name(self) -> Option<&'static str> {
        match self {
            Module::None => None,
            Module::Loop => Some("loop"),
            Module::RawXml => Some("rawxml"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Location {
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExpandTo {
    Auto,
    Tag(String),
}

/// Data attached to a loop placeholder once its opening and closing tags were
/// paired.
#[derive(Clone, Debug, Default)]
pub(crate) struct LoopData {
    pub subparsed: Vec<Part>,
    pub paragraph_loop: bool,
    pub has_page_break: bool,
    pub has_page_break_beginning: bool,
    pub sect_pr_count: usize,
    pub add_continuous_type: bool,
    pub add_next_page: Option<usize>,
    pub last_paragraph_sect_pr: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Placeholder {
    /// The tag expression (without prefix), e.g. `users` for `{#users}`.
    pub value: String,
    /// The full text between the delimiters, e.g. `#users`.
    pub raw: String,
    pub module: Module,
    pub location: Option<Location>,
    pub inverted: bool,
    pub expand_to: Option<ExpandTo>,
    pub offset: usize,
    pub l_index: usize,
    pub end_l_index: usize,
    pub loop_data: Option<Box<LoopData>>,
    pub expanded: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum Part {
    Tag(Tag),
    Content(Content),
    Delimiter(Delimiter),
    Placeholder(Box<Placeholder>),
}

impl Part {
    pub fn l_index(&self) -> usize {
        match self {
            Part::Tag(t) => t.l_index,
            Part::Content(c) => c.l_index,
            Part::Delimiter(d) => d.l_index,
            Part::Placeholder(p) => p.l_index,
        }
    }

    /// The XML text of this part (empty for placeholders/delimiters).
    pub fn xml_value(&self) -> &str {
        match self {
            Part::Tag(t) => &t.value,
            Part::Content(c) => &c.value,
            _ => "",
        }
    }

    pub fn as_tag(&self) -> Option<&Tag> {
        match self {
            Part::Tag(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_placeholder(&self) -> Option<&Placeholder> {
        match self {
            Part::Placeholder(p) => Some(p),
            _ => None,
        }
    }

    pub fn tag_name(&self) -> Option<&str> {
        self.as_tag().map(|t| t.tag.as_str())
    }

    /// Start or self-closing tag named `name`.
    pub fn is_tag_start(&self, name: &str) -> bool {
        matches!(self, Part::Tag(t) if t.tag == name && matches!(t.position, Position::Start | Position::SelfClosing))
    }

    pub fn is_tag_end(&self, name: &str) -> bool {
        matches!(self, Part::Tag(t) if t.tag == name && t.position == Position::End)
    }

    pub fn is_paragraph_start(&self) -> bool {
        matches!(self, Part::Tag(t) if is_paragraph_tag(&t.tag) && t.position == Position::Start)
    }

    pub fn is_paragraph_end(&self) -> bool {
        matches!(self, Part::Tag(t) if is_paragraph_tag(&t.tag) && t.position == Position::End)
    }

    pub fn is_break_tag(&self) -> bool {
        matches!(self, Part::Tag(t) if (t.tag == "w:br" || t.tag == "a:br") && matches!(t.position, Position::Start | Position::SelfClosing))
    }

    pub fn is_text_start(&self) -> bool {
        matches!(self, Part::Tag(t) if t.text && t.position == Position::Start)
    }

    pub fn is_text_end(&self) -> bool {
        matches!(self, Part::Tag(t) if t.text && t.position == Position::End)
    }

    /// Placeholder or document text.
    pub fn is_content(&self) -> bool {
        match self {
            Part::Placeholder(_) => true,
            Part::Content(c) => c.inside,
            _ => false,
        }
    }

    pub fn is_any_content(&self) -> bool {
        matches!(self, Part::Content(_))
    }
}

fn is_paragraph_tag(tag: &str) -> bool {
    matches!(tag, "w:p" | "a:p" | "text:p")
}

/// `<element>`, `<element ...>` or `<element/>`.
pub(crate) fn is_starting(value: &str, element: &str) -> bool {
    let b = value.as_bytes();
    b.first() == Some(&b'<')
        && value[1..].starts_with(element)
        && matches!(b.get(element.len() + 1), Some(b'>' | b' ' | b'/'))
}

/// Exactly `</element>`.
pub(crate) fn is_ending(value: &str, element: &str) -> bool {
    value.len() == element.len() + 3
        && value.starts_with("</")
        && value.ends_with('>')
        && &value[2..value.len() - 1] == element
}

pub(crate) fn get_right_or_null(parsed: &[Part], element: &str, index: usize) -> Option<usize> {
    let mut level: i64 = 1;
    for (i, part) in parsed.iter().enumerate().skip(index) {
        let v = part.xml_value();
        if is_ending(v, element) {
            level -= 1;
        }
        if is_starting(v, element) {
            level += 1;
        }
        if level == 0 {
            return Some(i);
        }
    }
    None
}

pub(crate) fn get_left_or_null(parsed: &[Part], element: &str, index: usize) -> Option<usize> {
    let mut level: i64 = 1;
    let start = index.min(parsed.len().saturating_sub(1));
    if parsed.is_empty() {
        return None;
    }
    for i in (0..=start).rev() {
        let v = parsed[i].xml_value();
        if is_starting(v, element) {
            level -= 1;
        }
        if is_ending(v, element) {
            level += 1;
        }
        if level == 0 {
            return Some(i);
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chunk {
    Start,
    End,
    None,
}

/// Port of `chunkBy`: returns index ranges into `parts`.
pub(crate) fn chunk_by(parts: &[Part], mut f: impl FnMut(&Part) -> Chunk) -> Vec<std::ops::Range<usize>> {
    let mut chunks: Vec<std::ops::Range<usize>> = Vec::new();
    chunks.push(0..0);
    for (i, p) in parts.iter().enumerate() {
        match f(p) {
            Chunk::Start => chunks.push(i..i + 1),
            Chunk::End => {
                chunks.last_mut().expect("non empty").end = i + 1;
                chunks.push(i + 1..i + 1);
            }
            Chunk::None => chunks.last_mut().expect("non empty").end = i + 1,
        }
    }
    chunks.into_iter().filter(|r| !r.is_empty()).collect()
}

/// `/^[ \n\r\t]+$/`
pub(crate) fn is_whitespace(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
}

/// Decode the five predefined XML entities (docxtemplater's `wordToUtf8`).
pub(crate) fn word_to_utf8(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&apos;", "'").replace("&quot;", "\"").replace("&gt;", ">").replace("&lt;", "<").replace("&amp;", "&")
}

/// Escape the five predefined XML entities (docxtemplater's `utf8ToWord`).
pub(crate) fn utf8_to_word(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Port of `getSingleAttribute`.
pub(crate) fn get_single_attribute(value: &str, name: &str) -> Option<String> {
    let needle = format!(" {name}=");
    let idx = value.find(&needle)?;
    let rest = &value[idx + needle.len()..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let inner = &rest[1..];
    let end = inner.find(['"', '\''])?;
    Some(inner[..end].to_string())
}

/// Port of `setSingleAttribute`.
pub(crate) fn set_single_attribute(value: &str, name: &str, attr_value: &str) -> String {
    let needle = format!(" {name}=\"");
    if let Some(idx) = value.rfind(&needle) {
        let start = idx + needle.len();
        if let Some(end) = value[start..].find('"') {
            return format!("{}{}{}", &value[..start], attr_value, &value[start + end..]);
        }
    }
    let end = value.rfind("/>").or_else(|| value.rfind('>')).unwrap_or(value.len());
    format!("{} {}=\"{}\"{}", &value[..end], name, attr_value, &value[end..])
}

/// Find `needle` in `hay` starting at byte `from` (rounded up to a char
/// boundary).
pub(crate) fn find_from(hay: &str, needle: &str, from: usize) -> Option<usize> {
    if from > hay.len() {
        return None;
    }
    let mut from = from;
    while !hay.is_char_boundary(from) {
        from += 1;
    }
    hay[from..].find(needle).map(|i| i + from)
}

/// Char-boundary-safe substring (`start..end` in bytes, clamped).
pub(crate) fn safe_slice(s: &str, start: usize, end: usize) -> &str {
    let mut start = start.min(s.len());
    let mut end = end.min(s.len());
    while !s.is_char_boundary(start) {
        start -= 1;
    }
    while !s.is_char_boundary(end) {
        end += 1;
    }
    if end < start {
        return "";
    }
    &s[start..end]
}
