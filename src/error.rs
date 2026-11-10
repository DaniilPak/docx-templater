use std::fmt;

/// One problem found in a template, mirroring the `properties` object of
/// docxtemplater's errors (`id`, `explanation`, `xtag`, `file`, `offset`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateError {
    /// Stable identifier, identical to docxtemplater's `properties.id`
    /// (e.g. `unclosed_tag`, `unopened_loop`, `raw_xml_tag_should_be_only_text_in_paragraph`).
    pub id: String,
    /// Short message, identical to docxtemplater's `error.message`.
    pub message: String,
    /// Human readable explanation, identical to docxtemplater's `properties.explanation`.
    pub explanation: String,
    /// The offending tag (without delimiters), when known.
    pub xtag: Option<String>,
    /// Surrounding text, when known.
    pub context: Option<String>,
    /// File inside the package, e.g. `word/document.xml`.
    pub file: Option<String>,
    /// Offset of the tag inside the concatenated text of the file.
    pub offset: Option<usize>,
}

impl TemplateError {
    pub(crate) fn new(id: &str, message: impl Into<String>, explanation: impl Into<String>) -> Self {
        TemplateError {
            id: id.to_string(),
            message: message.into(),
            explanation: explanation.into(),
            xtag: None,
            context: None,
            file: None,
            offset: None,
        }
    }

    pub(crate) fn xtag(mut self, xtag: impl Into<String>) -> Self {
        self.xtag = Some(xtag.into());
        self
    }

    pub(crate) fn context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    pub(crate) fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{file}: ")?;
        }
        write!(f, "{} ({}): {}", self.message, self.id, self.explanation)
    }
}

impl std::error::Error for TemplateError {}

/// Errors returned by this crate.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The template (or rendering it with the given data) has one or more
    /// problems. Like docxtemplater's "Multi error", every problem found in
    /// every file is reported, not only the first one.
    Template(Vec<TemplateError>),
    /// The input is not a valid zip archive or could not be written.
    Zip(zip::result::ZipError),
    /// I/O error.
    Io(std::io::Error),
    /// The data could not be serialized to JSON.
    Data(serde_json::Error),
}

impl Error {
    /// The individual template errors (empty for other kinds of errors).
    pub fn template_errors(&self) -> &[TemplateError] {
        match self {
            Error::Template(errors) => errors,
            _ => &[],
        }
    }

    pub(crate) fn single(e: TemplateError) -> Self {
        Error::Template(vec![e])
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Template(errors) if errors.len() == 1 => write!(f, "{}", errors[0]),
            Error::Template(errors) => {
                write!(f, "Multi error: the template has {} errors", errors.len())?;
                for e in errors {
                    write!(f, "\n  - {e}")?;
                }
                Ok(())
            }
            Error::Zip(e) => write!(f, "zip error: {e}"),
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Data(e) => write!(f, "could not serialize data: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Zip(e) => Some(e),
            Error::Io(e) => Some(e),
            Error::Data(e) => Some(e),
            Error::Template(_) => None,
        }
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Error::Zip(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Data(e)
    }
}

// Constructors mirroring docxtemplater's errors.js

pub(crate) fn first_word(s: &str) -> &str {
    s.split(' ').next().unwrap_or("")
}

fn truncate30(s: &str) -> String {
    s.chars().take(30).collect()
}

pub(crate) fn unopened_tag(xtag: &str, offset: usize) -> TemplateError {
    TemplateError::new(
        "unopened_tag",
        "Unopened tag",
        format!("The tag beginning with \"{}\" is unopened", truncate30(xtag)),
    )
    .xtag(xtag.split(' ').next_back().unwrap_or(""))
    .context(xtag)
    .offset(offset)
}

pub(crate) fn duplicate_open_tag(xtag: &str, offset: usize) -> TemplateError {
    TemplateError::new(
        "duplicate_open_tag",
        "Duplicate open tag, expected one open tag",
        format!("The tag beginning with \"{}\" has duplicate open tags", truncate30(xtag)),
    )
    .xtag(first_word(xtag))
    .context(xtag)
    .offset(offset)
}

pub(crate) fn duplicate_close_tag(xtag: &str, offset: usize) -> TemplateError {
    TemplateError::new(
        "duplicate_close_tag",
        "Duplicate close tag, expected one close tag",
        format!("The tag ending with \"{}\" has duplicate close tags", truncate30(xtag)),
    )
    .xtag(first_word(xtag))
    .context(xtag)
    .offset(offset)
}

pub(crate) fn unclosed_tag(xtag: &str, offset: usize) -> TemplateError {
    let first = first_word(xtag);
    let name = first.char_indices().nth(1).map(|(i, _)| &first[i..]).unwrap_or("");
    TemplateError::new(
        "unclosed_tag",
        "Unclosed tag",
        format!("The tag beginning with \"{}\" is unclosed", truncate30(xtag)),
    )
    .xtag(name)
    .context(xtag)
    .offset(offset)
}

pub(crate) fn malformed_xml() -> TemplateError {
    TemplateError::new("malformed_xml", "Malformed xml", "The template contains malformed xml")
}

pub(crate) fn invalid_xml(offset: usize) -> TemplateError {
    TemplateError::new(
        "file_has_invalid_xml",
        "An XML file has invalid xml",
        "The docx contains invalid XML, it is most likely corrupt",
    )
    .offset(offset)
}

pub(crate) fn change_delimiters_invalid() -> TemplateError {
    TemplateError::new("change_delimiters_invalid", "New Delimiters cannot be parsed", "Cannot parser delimiters")
}
