//! Per-format configuration, ported from docxtemplater's `file-type-config.js`
//! and `filetypes.js`.

/// The kind of Office document being templated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    /// Word document (`.docx`, `.docm`, `.dotx`, `.dotm`).
    Docx,
    /// PowerPoint presentation (`.pptx`, `.pptm`).
    Pptx,
}

pub(crate) struct ExpandTag {
    pub contains: &'static str,
    pub expand: &'static str,
    pub only_text_in_tag: bool,
}

pub(crate) struct ShouldContain {
    pub tag: &'static str,
    pub should_contain: &'static [&'static str],
    pub value: &'static str,
    pub drop: bool,
    pub drop_parent: Option<&'static str>,
}

pub(crate) struct FileTypeConfig {
    pub text_tags: &'static [&'static str],
    pub lexed_tags: &'static [&'static str],
    pub dropped_tags_inside_placeholder: &'static [&'static str],
    pub expand_tags: &'static [ExpandTag],
    pub on_paragraph_loop: &'static [ExpandTag],
    pub tag_raw_xml: &'static str,
    pub tag_should_contain: &'static [ShouldContain],
    /// `w` for docx, `a` for pptx.
    pub prefix: &'static str,
}

pub(crate) static DOCX: FileTypeConfig = FileTypeConfig {
    text_tags: &[
        "Company",
        "HyperlinkBase",
        "Manager",
        "cp:category",
        "cp:keywords",
        "dc:creator",
        "dc:description",
        "dc:subject",
        "dc:title",
        "cp:contentStatus",
        "PublishDate",
        "Abstract",
        "CompanyAddress",
        "CompanyPhone",
        "CompanyFax",
        "CompanyEmail",
        "w:t",
        "a:t",
        "m:t",
        "vt:lpstr",
        "vt:lpwstr",
    ],
    lexed_tags: &[
        "w:proofState",
        "w:tc",
        "w:tr",
        "w:tbl",
        "w:ftr",
        "w:hdr",
        "w:body",
        "w:document",
        "w:p",
        "w:r",
        "w:br",
        "w:rPr",
        "w:pPr",
        "w:spacing",
        "w:sdtContent",
        "w:sdt",
        "w:drawing",
        "w:sectPr",
        "w:type",
        "w:headerReference",
        "w:footerReference",
        "w:bookmarkStart",
        "w:bookmarkEnd",
        "w:commentRangeStart",
        "w:commentRangeEnd",
        "w:commentReference",
    ],
    dropped_tags_inside_placeholder: &["w:p", "w:br", "w:bookmarkStart", "w:bookmarkEnd"],
    expand_tags: &[ExpandTag { contains: "w:tc", expand: "w:tr", only_text_in_tag: false }],
    on_paragraph_loop: &[ExpandTag { contains: "w:p", expand: "w:p", only_text_in_tag: true }],
    tag_raw_xml: "w:p",
    tag_should_contain: &[
        ShouldContain {
            tag: "w:sdtContent",
            should_contain: &["w:p", "w:r", "w:commentRangeStart", "w:sdt"],
            value: "<w:p></w:p>",
            drop: false,
            drop_parent: None,
        },
        ShouldContain { tag: "w:tc", should_contain: &["w:p"], value: "<w:p></w:p>", drop: false, drop_parent: None },
        ShouldContain { tag: "w:tr", should_contain: &["w:tc"], value: "", drop: true, drop_parent: None },
        ShouldContain { tag: "w:tbl", should_contain: &["w:tr"], value: "", drop: true, drop_parent: None },
    ],
    prefix: "w",
};

pub(crate) static PPTX: FileTypeConfig = FileTypeConfig {
    text_tags: &[
        "Company",
        "HyperlinkBase",
        "Manager",
        "cp:category",
        "cp:keywords",
        "dc:creator",
        "dc:description",
        "dc:subject",
        "dc:title",
        "a:t",
        "m:t",
        "vt:lpstr",
        "vt:lpwstr",
    ],
    lexed_tags: &[
        "p:sp",
        "a:tc",
        "a:tr",
        "a:tbl",
        "a:graphicData",
        "a:p",
        "a:r",
        "a:rPr",
        "p:txBody",
        "a:txBody",
        "a:off",
        "a:ext",
        "p:graphicFrame",
        "p:xfrm",
        "a16:rowId",
        "a:endParaRPr",
    ],
    dropped_tags_inside_placeholder: &["a:p", "a:endParaRPr"],
    expand_tags: &[ExpandTag { contains: "a:tc", expand: "a:tr", only_text_in_tag: false }],
    on_paragraph_loop: &[ExpandTag { contains: "a:p", expand: "a:p", only_text_in_tag: true }],
    tag_raw_xml: "p:sp",
    tag_should_contain: &[
        ShouldContain {
            tag: "a:tbl",
            should_contain: &["a:tr"],
            value: "",
            drop: false,
            drop_parent: Some("p:graphicFrame"),
        },
        ShouldContain {
            tag: "p:txBody",
            should_contain: &["a:p"],
            value: "<a:p></a:p>",
            drop: false,
            drop_parent: None,
        },
        ShouldContain {
            tag: "a:txBody",
            should_contain: &["a:p"],
            value: "<a:p></a:p>",
            drop: false,
            drop_parent: None,
        },
    ],
    prefix: "a",
};

impl FileType {
    pub(crate) fn config(self) -> &'static FileTypeConfig {
        match self {
            FileType::Docx => &DOCX,
            FileType::Pptx => &PPTX,
        }
    }
}

pub(crate) mod ct {
    pub const DOCX_MAIN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
    pub const DOCM_MAIN: &str = "application/vnd.ms-word.document.macroEnabled.main+xml";
    pub const DOTX_MAIN: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";
    pub const DOTM_MAIN: &str = "application/vnd.ms-word.template.macroEnabledTemplate.main+xml";
    pub const HEADER: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
    pub const FOOTER: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
    pub const FOOTNOTES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";
    pub const ENDNOTES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml";
    pub const COMMENTS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";

    pub const PPTX_SLIDE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
    pub const PPTX_SLIDE_MASTER: &str = "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml";
    pub const PPTX_SLIDE_LAYOUT: &str = "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml";
    pub const PPTX_PRESENTATION: &str =
        "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
    pub const PPTM_PRESENTATION: &str = "application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml";
    pub const PPTX_NOTES: &str = "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";

    pub const XLSX_MAIN: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
    pub const XLSM_MAIN: &str = "application/vnd.ms-excel.sheet.macroEnabled.main+xml";

    pub const CORE: &str = "application/vnd.openxmlformats-package.core-properties+xml";
    pub const APP: &str = "application/vnd.openxmlformats-officedocument.extended-properties+xml";
    pub const CUSTOM: &str = "application/vnd.openxmlformats-officedocument.custom-properties+xml";
    pub const SETTINGS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";
    pub const DIAGRAM_DATA: &str = "application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml";
    pub const DIAGRAM_DRAWING: &str = "application/vnd.ms-office.drawingml.diagramDrawing+xml";

    pub const DOCX_MAINS: &[&str] = &[DOCX_MAIN, DOCM_MAIN, DOTX_MAIN, DOTM_MAIN];
    /// docxtemplater's `filetypes.docx` (+ endnotes).
    pub const DOCX_PARTS: &[&str] =
        &[HEADER, DOCX_MAIN, DOCM_MAIN, DOTX_MAIN, DOTM_MAIN, FOOTER, FOOTNOTES, ENDNOTES, COMMENTS];
    /// docxtemplater's `filetypes.pptx` (+ macro-enabled presentation and notes).
    pub const PPTX_PARTS: &[&str] =
        &[PPTX_SLIDE, PPTX_SLIDE_MASTER, PPTX_SLIDE_LAYOUT, PPTX_PRESENTATION, PPTM_PRESENTATION, PPTX_NOTES];
    pub const XLSX_PARTS: &[&str] = &[XLSX_MAIN, XLSM_MAIN];
    pub const COMMON: &[&str] = &[SETTINGS, CORE, APP, CUSTOM, DIAGRAM_DATA, DIAGRAM_DRAWING];
    /// Content types for which the `linebreaks` option is ignored.
    pub const NON_LINE_BREAKS: &[&str] = &[SETTINGS, CORE, APP, CUSTOM];
}
