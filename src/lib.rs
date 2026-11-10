//! Fill Word (`.docx`) and PowerPoint (`.pptx`) templates with JSON / serde
//! data, using the same tag syntax as the JavaScript library
//! [docxtemplater](https://github.com/open-xml-templating/docxtemplater).
//!
//! ```no_run
//! use docx_templater::Template;
//! use serde_json::json;
//!
//! let template = Template::from_path("invoice-template.docx")?;
//! let bytes = template
//!     .render(&json!({
//!         "client": { "name": "ACME" },
//!         "items": [ { "name": "Rocket", "price": 10 }, { "name": "Anvil", "price": 3 } ],
//!     }))?
//!     .to_bytes()?;
//! std::fs::write("invoice.docx", bytes)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Supported syntax (docxtemplater's open-source core):
//!
//! | Tag | Meaning |
//! |-----|---------|
//! | `{name}` | value (dotted paths like `{user.address.city}` work) |
//! | `{#items}…{/items}` | loop over an array, or section shown when the value is truthy |
//! | `{^items}…{/items}` | inverted section: shown when the value is falsy / an empty array |
//! | `{@raw}` | insert raw OOXML, replacing the whole paragraph |
//! | `{-w:p items}…{/items}` | loop that repeats the enclosing `w:p` (or any other element) |
//! | `{=[[ ]]=}` | change delimiters for the rest of the document |
//!
//! Tags may be split across several runs by Word (spell checking, partial
//! formatting...); the lexer works on the concatenated text so this is handled
//! transparently, exactly like docxtemplater.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Internal functions return a single `TemplateError`; the public API boxes them in `Error::Template(Vec<_>)`.
#![allow(clippy::result_large_err)]

mod error;
mod filetype;
mod lexer;
mod options;
mod package;
mod parser;
mod part;
mod postparse;
mod render;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use zip::ZipArchive;

pub use error::{Error, TemplateError};
pub use filetype::FileType;
pub use options::{Delimiters, NullContext, Options, ParserContext, Syntax};

use filetype::ct;
use package::{Package, decode_xml, read_entry};
use part::Part;
use postparse::{PostCtx, get_sect_pr, postparse};
use render::{Renderer, Scope, fix_doc_pr, join_uncorrupt, space_preserve_postrender};

struct CompiledFile {
    name: String,
    content_type: String,
    original: String,
    parts: Vec<Part>,
    sects: Vec<Vec<Part>>,
}

/// A compiled template. Compile once, render as many times as needed
/// (`Template` is `Send + Sync`).
pub struct Template {
    package: Package,
    file_type: FileType,
    options: Options,
    files: Vec<CompiledFile>,
    main: Option<String>,
}

impl std::fmt::Debug for Template {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Template")
            .field("file_type", &self.file_type)
            .field("templated_files", &self.files.iter().map(|f| &f.name).collect::<Vec<_>>())
            .field("options", &self.options)
            .finish()
    }
}

impl Template {
    /// Compile a `.docx` / `.pptx` template with default [`Options`].
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, Error> {
        Self::with_options(bytes, Options::default())
    }

    /// Read and compile a template file with default [`Options`].
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::from_bytes(std::fs::read(path)?)
    }

    /// Read and compile a template file.
    pub fn from_path_with_options(path: impl AsRef<Path>, options: Options) -> Result<Self, Error> {
        Self::with_options(std::fs::read(path)?, options)
    }

    /// Compile a template. All syntax errors of all templated parts (document,
    /// headers, footers, footnotes, slides...) are returned together in
    /// [`Error::Template`].
    pub fn with_options(bytes: impl Into<Vec<u8>>, options: Options) -> Result<Self, Error> {
        let package = Package::open(bytes.into())?;
        let file_type = package.file_type()?;
        let cfg = file_type.config();
        let names = package.templated_parts(file_type);
        let main = names
            .iter()
            .find(|n| {
                let c = package.content_type(n);
                match file_type {
                    FileType::Docx => ct::DOCX_MAINS.contains(&c),
                    FileType::Pptx => c == ct::PPTX_SLIDE,
                }
            })
            .cloned();

        let mut archive = ZipArchive::new(Cursor::new(package.bytes.as_slice()))?;
        let mut files = Vec::with_capacity(names.len());
        let mut errors = Vec::new();
        for name in names {
            let content_type = package.content_type(&name).to_string();
            let original = decode_xml(read_entry(&mut archive, &name)?);
            let with_file = |mut e: TemplateError| {
                e.file = Some(name.clone());
                e
            };
            let (lexed, lex_errors) = match lexer::lex(&original, cfg, &options.delimiters, &options.syntax) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(with_file(e));
                    continue;
                }
            };
            errors.extend(lex_errors.into_iter().map(with_file));
            let sects = if ct::DOCX_MAINS.contains(&content_type.as_str()) { get_sect_pr(&lexed) } else { Vec::new() };
            let parsed = parser::parse(lexed, cfg, &options.syntax);
            let mut ctx = PostCtx::new(file_type, &options, &sects);
            let parts = postparse(parsed, None, &mut ctx);
            errors.extend(ctx.errors.into_iter().map(with_file));
            files.push(CompiledFile { name, content_type, original, parts, sects });
        }
        drop(archive);
        if !errors.is_empty() {
            return Err(Error::Template(errors));
        }
        Ok(Template { package, file_type, options, files, main })
    }

    /// Whether this is a Word or PowerPoint template.
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    /// The options the template was compiled with.
    pub fn options(&self) -> &Options {
        &self.options
    }

    /// Paths (inside the zip) of the parts that are templated.
    pub fn templated_files(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|f| f.name.as_str())
    }

    /// Text of the main document part (`word/document.xml`, or the first slide)
    /// before rendering, like docxtemplater's `getFullText()`.
    pub fn full_text(&self) -> String {
        self.main
            .as_deref()
            .and_then(|m| self.files.iter().find(|f| f.name == m))
            .map(|f| lexer::full_text(&f.original, self.file_type.config()))
            .unwrap_or_default()
    }

    /// Render the template with any serializable data.
    pub fn render<T: Serialize + ?Sized>(&self, data: &T) -> Result<Rendered<'_>, Error> {
        let value = serde_json::to_value(data)?;
        self.render_value(&value)
    }

    /// Render the template with a JSON value.
    pub fn render_value(&self, data: &Value) -> Result<Rendered<'_>, Error> {
        let mut out: HashMap<String, String> = HashMap::new();
        let mut errors = Vec::new();
        let root = Scope::root(data);
        for file in &self.files {
            let mut renderer = Renderer::new(&self.options, self.file_type, &file.content_type, &file.sects);
            renderer.render(&file.parts, &root);
            if !renderer.errors.is_empty() {
                errors.extend(renderer.errors.into_iter().map(|mut e| {
                    e.file = Some(file.name.clone());
                    e
                }));
                continue;
            }
            let mut parts = renderer.out;
            if self.file_type == FileType::Docx {
                space_preserve_postrender(&mut parts);
            }
            join_uncorrupt(&mut parts, self.file_type.config(), &file.content_type);
            out.insert(file.name.clone(), parts.concat());
        }
        if !errors.is_empty() {
            return Err(Error::Template(errors));
        }
        if self.file_type == FileType::Docx && self.options.fix_doc_pr_ids {
            let mut next_id = 1u64;
            for file in &self.files {
                if !ct::DOCX_PARTS.contains(&file.content_type.as_str()) {
                    continue;
                }
                if let Some(content) = out.get(&file.name)
                    && let Some(fixed) = fix_doc_pr(content, &mut next_id)
                {
                    out.insert(file.name.clone(), fixed);
                }
            }
        }
        for file in &self.files {
            if out.get(&file.name).is_some_and(|c| *c == file.original) {
                out.remove(&file.name);
            }
        }
        Ok(Rendered { template: self, files: out })
    }
}

/// The result of [`Template::render`].
pub struct Rendered<'t> {
    template: &'t Template,
    files: HashMap<String, String>,
}

impl std::fmt::Debug for Rendered<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rendered").field("changed_files", &self.files.keys().collect::<Vec<_>>()).finish()
    }
}

impl Rendered<'_> {
    /// The rendered XML of a templated part, e.g. `word/document.xml`.
    pub fn xml(&self, name: &str) -> Option<&str> {
        self.files
            .get(name)
            .map(String::as_str)
            .or_else(|| self.template.files.iter().find(|f| f.name == name).map(|f| f.original.as_str()))
    }

    /// Text of the rendered main document (or of `name` with
    /// [`Rendered::full_text_of`]), like docxtemplater's `getFullText()`.
    pub fn full_text(&self) -> String {
        self.template.main.as_deref().map(|m| self.full_text_of(m)).unwrap_or_default()
    }

    /// Text of a rendered templated part, e.g. `word/header1.xml`.
    pub fn full_text_of(&self, name: &str) -> String {
        self.xml(name).map(|x| lexer::full_text(x, self.template.file_type.config())).unwrap_or_default()
    }

    /// Build the output `.docx` / `.pptx`.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.template.package.write(&self.files)
    }

    /// Write the output document to a file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        std::fs::write(path, self.to_bytes()?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_send_sync() {
        fn assert<T: Send + Sync>() {}
        assert::<Template>();
    }

    #[test]
    fn default_get_paths() {
        let v = serde_json::json!({"a": {"b": [ {"c": 1} ]}, "x.y": 2});
        assert_eq!(render::default_get("a.b.0.c", &v), Some(&serde_json::json!(1)));
        assert_eq!(render::default_get("x.y", &v), Some(&serde_json::json!(2)));
        assert_eq!(render::default_get(" . ", &v), Some(&v));
        assert_eq!(render::default_get("missing", &v), None);
    }

    #[test]
    fn js_strings() {
        use serde_json::json;
        assert_eq!(render::js_to_string(&json!(1.0)), "1");
        assert_eq!(render::js_to_string(&json!(1.5)), "1.5");
        assert_eq!(render::js_to_string(&json!([1, "a", null])), "1,a,");
        assert_eq!(render::js_to_string(&json!({})), "[object Object]");
        assert_eq!(render::js_to_string(&json!(true)), "true");
    }
}
