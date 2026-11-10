use std::fmt;
use std::sync::Arc;

use serde_json::Value;

/// Information about a tag whose value is `null`/missing, passed to the
/// [`Options::null_getter`] callback (docxtemplater's `nullGetter(part)`).
#[derive(Debug, Clone)]
pub struct NullContext<'a> {
    /// The tag expression, e.g. `user.name` for `{user.name}` or `users` for `{#users}`.
    pub tag: &'a str,
    /// The full text between the delimiters, e.g. `#users`.
    pub raw: &'a str,
    /// `None` for a plain `{value}` tag, `Some("loop")` for loops / conditions,
    /// `Some("rawxml")` for `{@raw}` tags (same names as docxtemplater's `part.module`).
    pub module: Option<&'a str>,
    /// Names of the enclosing loops, outermost first.
    pub scope_path: &'a [String],
}

/// Context passed to a custom [`Options::parser`].
#[derive(Debug, Clone)]
pub struct ParserContext<'a> {
    /// The scope currently being searched.
    pub scope: &'a Value,
    /// All scopes, outermost (the root data) first.
    pub scope_list: &'a [&'a Value],
    /// Index of `scope` inside `scope_list`.
    pub num: usize,
    /// Names of the enclosing loops, outermost first.
    pub scope_path: &'a [String],
    /// Iteration index of each enclosing loop.
    pub scope_path_item: &'a [usize],
    /// Length of each enclosing loop.
    pub scope_path_length: &'a [usize],
}

pub(crate) type NullGetterFn = dyn Fn(&NullContext<'_>) -> Value + Send + Sync;
pub(crate) type ParserFn = dyn Fn(&str, &ParserContext<'_>) -> Result<Option<Value>, String> + Send + Sync;

/// Tag delimiters, `{` and `}` by default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delimiters {
    /// Opening delimiter.
    pub start: String,
    /// Closing delimiter.
    pub end: String,
}

impl Default for Delimiters {
    fn default() -> Self {
        Delimiters { start: "{".into(), end: "}".into() }
    }
}

/// Advanced syntax switches (docxtemplater's `syntax` option).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Syntax {
    /// Prefix of the "change delimiters" tag, `=` by default
    /// (`{=[[ ]]=}` switches to `[[`/`]]`). `None` disables the feature.
    pub change_delimiter_prefix: Option<String>,
    /// Treat a lone closing delimiter as text instead of an error.
    pub allow_unopened_tag: bool,
    /// Treat a lone opening delimiter as text instead of an error.
    pub allow_unclosed_tag: bool,
    /// Allow loops whose expansion overlaps another loop.
    pub allow_unbalanced_loops: bool,
    /// Keep line breaks that are inside a tag (as `\n`) in the tag expression.
    pub preserve_newlines_in_tags: bool,
}

impl Default for Syntax {
    fn default() -> Self {
        Syntax {
            change_delimiter_prefix: Some("=".into()),
            allow_unopened_tag: false,
            allow_unclosed_tag: false,
            allow_unbalanced_loops: false,
            preserve_newlines_in_tags: false,
        }
    }
}

/// Rendering options. Defaults match docxtemplater's defaults (except
/// [`Options::fix_doc_pr_ids`]).
#[derive(Clone)]
pub struct Options {
    /// Remove the paragraphs that contain only a `{#loop}` / `{/loop}` tag
    /// (docxtemplater's `paragraphLoop`). Default `false`.
    pub paragraph_loop: bool,
    /// Render `\n` in values as line breaks (docxtemplater's `linebreaks`). Default `false`.
    pub linebreaks: bool,
    /// Tag delimiters. Default `{` / `}`.
    pub delimiters: Delimiters,
    /// Advanced syntax switches.
    pub syntax: Syntax,
    /// Silently remove characters that are invalid in XML from values instead of
    /// failing with `invalid_xml_characters` (docxtemplater's `stripInvalidXMLChars`).
    pub strip_invalid_xml_chars: bool,
    /// Renumber `<wp:docPr id>` after rendering so images duplicated by loops keep
    /// unique ids (what docxtemplater's opt-in `fixDocPrCorruption` module does).
    /// Default `true`.
    pub fix_doc_pr_ids: bool,
    pub(crate) null_getter: Option<Arc<NullGetterFn>>,
    pub(crate) parser: Option<Arc<ParserFn>>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            paragraph_loop: false,
            linebreaks: false,
            delimiters: Delimiters::default(),
            syntax: Syntax::default(),
            strip_invalid_xml_chars: false,
            fix_doc_pr_ids: true,
            null_getter: None,
            parser: None,
        }
    }
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("paragraph_loop", &self.paragraph_loop)
            .field("linebreaks", &self.linebreaks)
            .field("delimiters", &self.delimiters)
            .field("syntax", &self.syntax)
            .field("strip_invalid_xml_chars", &self.strip_invalid_xml_chars)
            .field("fix_doc_pr_ids", &self.fix_doc_pr_ids)
            .field("null_getter", &self.null_getter.as_ref().map(|_| "<fn>"))
            .field("parser", &self.parser.as_ref().map(|_| "<fn>"))
            .finish()
    }
}

impl Options {
    /// Default options (same as [`Options::default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// See [`Options::paragraph_loop`](struct.Options.html#structfield.paragraph_loop).
    pub fn paragraph_loop(mut self, on: bool) -> Self {
        self.paragraph_loop = on;
        self
    }

    /// See [`Options::linebreaks`](struct.Options.html#structfield.linebreaks).
    pub fn linebreaks(mut self, on: bool) -> Self {
        self.linebreaks = on;
        self
    }

    /// Use custom delimiters, e.g. `.delimiters("{{", "}}")`.
    pub fn delimiters(mut self, start: impl Into<String>, end: impl Into<String>) -> Self {
        self.delimiters = Delimiters { start: start.into(), end: end.into() };
        self
    }

    /// Set the advanced [`Syntax`] switches.
    pub fn syntax(mut self, syntax: Syntax) -> Self {
        self.syntax = syntax;
        self
    }

    /// See [`Options::strip_invalid_xml_chars`](struct.Options.html#structfield.strip_invalid_xml_chars).
    pub fn strip_invalid_xml_chars(mut self, on: bool) -> Self {
        self.strip_invalid_xml_chars = on;
        self
    }

    /// See [`Options::fix_doc_pr_ids`](struct.Options.html#structfield.fix_doc_pr_ids).
    pub fn fix_doc_pr_ids(mut self, on: bool) -> Self {
        self.fix_doc_pr_ids = on;
        self
    }

    /// Value used when a tag resolves to `null` or is missing
    /// (docxtemplater's `nullGetter`). The default returns `"undefined"` for
    /// plain tags and `""` for loops and raw tags, exactly like docxtemplater.
    ///
    /// ```
    /// use docx_templater::Options;
    /// let opts = Options::new().null_getter(|_ctx| serde_json::Value::from(""));
    /// ```
    pub fn null_getter(mut self, f: impl Fn(&NullContext<'_>) -> Value + Send + Sync + 'static) -> Self {
        self.null_getter = Some(Arc::new(f));
        self
    }

    /// Replace the default tag evaluator (docxtemplater's `parser` option).
    ///
    /// The function is called with the tag expression and one scope at a time,
    /// innermost first; returning `Ok(None)` (or `Ok(Some(Value::Null))`) makes the
    /// lookup continue in the enclosing scope. Returning `Err` produces a
    /// `scopeparser_execution_failed` error.
    pub fn parser(
        mut self,
        f: impl Fn(&str, &ParserContext<'_>) -> Result<Option<Value>, String> + Send + Sync + 'static,
    ) -> Self {
        self.parser = Some(Arc::new(f));
        self
    }
}
