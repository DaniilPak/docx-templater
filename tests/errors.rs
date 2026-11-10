//! Error reporting: every problem of every templated part is reported at once
//! (docxtemplater's "Multi error"), with the same ids / messages / offsets.

mod common;

use std::path::PathBuf;

use common::{docx, docx_with_parts, p};
use docx_templater::{Error, Options, Template, TemplateError};
use serde_json::json;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upstream").join(name)).unwrap()
}

fn compile_errors(bytes: Vec<u8>, opts: Options) -> Vec<TemplateError> {
    match Template::with_options(bytes, opts) {
        Err(Error::Template(errors)) => errors,
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => panic!("template compiled without errors"),
    }
}

fn ids(errors: &[TemplateError]) -> Vec<&str> {
    errors.iter().map(|e| e.id.as_str()).collect()
}

#[test]
fn upstream_multi_errors_lexer() {
    // same expectations as docxtemplater's "should fail properly when having lexed + postparsed errors"
    let errors = compile_errors(fixture("multi-errors.docx"), Options::new().paragraph_loop(true));
    let summary: Vec<_> = errors
        .iter()
        .map(|e| (e.id.as_str(), e.message.as_str(), e.xtag.as_deref(), e.context.as_deref(), e.offset))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("unclosed_tag", "Unclosed tag", Some("firstName"), Some("{firstName "), Some(0)),
            ("unclosed_tag", "Unclosed tag", Some("error"), Some("{error  "), Some(22)),
            (
                "duplicate_close_tag",
                "Duplicate close tag, expected one close tag",
                Some("{tag}}"),
                Some("{tag}}"),
                Some(34)
            ),
            (
                "duplicate_open_tag",
                "Duplicate open tag, expected one open tag",
                Some("{{bar}"),
                Some("{{bar}"),
                Some(42)
            ),
        ]
    );
    assert!(errors.iter().all(|e| e.file.as_deref() == Some("word/document.xml")));
}

#[test]
fn upstream_errors_in_header_footer_and_document() {
    let errors = compile_errors(fixture("errors-footer-and-header.docx"), Options::new().paragraph_loop(true));
    let mut got: Vec<_> = errors.iter().map(|e| (e.file.clone().unwrap(), e.id.clone(), e.offset)).collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("word/document.xml".into(), "closing_tag_does_not_match_opening_tag".into(), Some(8)),
            ("word/footer1.xml".into(), "unclosed_tag".into(), Some(2)),
            ("word/header1.xml".into(), "duplicate_close_tag".into(), Some(15)),
        ]
    );
    let header = errors.iter().find(|e| e.id == "duplicate_close_tag").unwrap();
    assert_eq!(header.xtag.as_deref(), Some("itle}}"));
    let footer = errors.iter().find(|e| e.id == "unclosed_tag").unwrap();
    assert_eq!(footer.xtag.as_deref(), Some("footer"));
}

#[test]
fn upstream_dash_loop_without_paragraph() {
    let errors = compile_errors(fixture("paragraph-loop-error.docx"), Options::new());
    let got: Vec<_> = errors.iter().map(|e| (e.id.as_str(), e.message.as_str(), e.offset)).collect();
    assert_eq!(
        got,
        vec![
            ("no_xml_tag_found_at_left", "No tag \"w:p\" was found at the left", Some(12)),
            ("no_xml_tag_found_at_right", "No tag \"w:p\" was found at the right", Some(26)),
        ]
    );
}

#[test]
fn upstream_table_inside_table() {
    let errors = compile_errors(fixture("error-table-tags.docx"), Options::new());
    assert_eq!(ids(&errors), vec!["loop_position_invalid"]);
    assert_eq!(errors[0].xtag.as_deref(), Some("loop"));
    assert_eq!(errors[0].message, "The position of the loop tags \"loop\" would produce invalid XML");
}

#[test]
fn upstream_unclosed_and_unopened_fixtures() {
    let e = compile_errors(fixture("one-unclosed.docx"), Options::new());
    assert!(ids(&e).iter().all(|id| *id == "unclosed_tag"), "{e:?}");
    let e = compile_errors(fixture("multiple-unclosed.docx"), Options::new());
    assert!(e.len() > 1 && ids(&e).iter().all(|id| *id == "unclosed_tag"), "{e:?}");
    let e = compile_errors(fixture("one-unopened.docx"), Options::new());
    assert!(ids(&e).contains(&"unopened_tag"), "{e:?}");
}

#[test]
fn double_braces_with_single_delimiters() {
    let e = compile_errors(docx(&p(&["{{name}}"])), Options::new());
    let got: Vec<_> = e.iter().map(|e| (e.id.as_str(), e.xtag.as_deref(), e.context.as_deref())).collect();
    assert_eq!(
        got,
        vec![
            ("duplicate_open_tag", Some("{{name"), Some("{{name")),
            ("duplicate_close_tag", Some("name}}"), Some("name}}")),
        ]
    );
}

#[test]
fn unclosed_loop() {
    let e = compile_errors(docx(&format!("{}{}", p(&["{#users}{name}"]), p(&["end"]))), Options::new());
    assert_eq!(ids(&e), vec!["unclosed_loop"]);
    assert_eq!(e[0].explanation, "The loop with tag \"users\" is unclosed");
    assert_eq!(e[0].xtag.as_deref(), Some("users"));
    assert_eq!(e[0].offset, Some(0));
}

#[test]
fn unopened_loop() {
    let e = compile_errors(docx(&p(&["Hello {name} {/users}"])), Options::new());
    assert_eq!(ids(&e), vec!["unopened_loop"]);
    assert_eq!(e[0].offset, Some(13));
}

#[test]
fn mismatched_loop() {
    let e = compile_errors(docx(&p(&["{#users}{name}{/companies}"])), Options::new());
    assert_eq!(ids(&e), vec!["closing_tag_does_not_match_opening_tag"]);
    assert_eq!(e[0].explanation, "The tag \"users\" is closed by the tag \"companies\"");
}

#[test]
fn split_unclosed_tag_points_to_tag() {
    // `{na` + `me` split across runs, never closed
    let e = compile_errors(docx(&p(&["Hello {na", "me and ", "more"])), Options::new());
    assert_eq!(ids(&e), vec!["unclosed_tag"]);
    assert_eq!(e[0].xtag.as_deref(), Some("name"));
    assert_eq!(e[0].offset, Some(6));
}

#[test]
fn errors_from_all_parts_are_collected() {
    let bytes = docx_with_parts(
        &p(&["{#a}"]),
        Some(&p(&["{unclosed"])),
        Some(&p(&["oops}"])),
        Some(r#"<w:footnote w:id="1"><w:p><w:r><w:t>{#x}{/y}</w:t></w:r></w:p></w:footnote>"#),
    );
    let e = compile_errors(bytes, Options::new());
    let mut got: Vec<_> = e.iter().map(|e| (e.file.clone().unwrap(), e.id.clone())).collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("word/document.xml".into(), "unclosed_loop".into()),
            ("word/footer1.xml".into(), "unopened_tag".into()),
            ("word/footnotes.xml".into(), "closing_tag_does_not_match_opening_tag".into()),
            ("word/header1.xml".into(), "unclosed_tag".into()),
        ]
    );
    let msg = Error::Template(e).to_string();
    assert!(msg.starts_with("Multi error: the template has 4 errors"), "{msg}");
}

#[test]
fn raw_tag_must_be_alone_in_paragraph() {
    let e = compile_errors(docx(&p(&["Hello {@raw}"])), Options::new());
    assert_eq!(ids(&e), vec!["raw_xml_tag_should_be_only_text_in_paragraph"]);
    assert_eq!(e[0].xtag.as_deref(), Some("raw"));
}

#[test]
fn loop_spanning_table_and_body_is_invalid() {
    let body = format!("<w:tbl><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>{}", p(&["{#users}"]), p(&["{/users}"]));
    let e = compile_errors(docx(&body), Options::new());
    assert_eq!(ids(&e), vec!["loop_position_invalid"]);
}

#[test]
fn unbalanced_loops_in_table() {
    let row = format!("<w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr>", p(&["{#a}X"]), p(&["{/a}{#b}Y{/b}"]));
    let e = compile_errors(docx(&format!("<w:tbl>{row}</w:tbl>")), Options::new());
    assert_eq!(ids(&e), vec!["unbalanced_loop_tags"]);
}

#[test]
fn render_errors() {
    let tpl = Template::from_bytes(docx(&format!("{}{}", p(&["{name}"]), p(&["{@raw}"])))).unwrap();
    let err = tpl.render(&json!({ "name": "bad\u{1}char", "raw": 42 })).unwrap_err();
    let got: Vec<_> = err.template_errors().iter().map(|e| e.id.as_str()).collect();
    assert_eq!(got, vec!["invalid_xml_characters", "invalid_raw_xml_value"]);
    // stripping invalid characters instead
    let tpl = Template::with_options(docx(&p(&["{name}"])), Options::new().strip_invalid_xml_chars(true)).unwrap();
    let out = tpl.render(&json!({ "name": "ok\u{1}" })).unwrap();
    assert_eq!(out.full_text(), "ok");
}

#[test]
fn custom_parser_errors() {
    let opts = Options::new().parser(|tag, _| if tag == "boom" { Err("kaboom".into()) } else { Ok(None) });
    let tpl = Template::with_options(docx(&p(&["{boom}"])), opts).unwrap();
    let err = tpl.render(&json!({})).unwrap_err();
    assert_eq!(err.template_errors()[0].id, "scopeparser_execution_failed");
    assert!(err.to_string().contains("kaboom"));
}

#[test]
fn not_an_office_file() {
    let err = Template::from_bytes(b"not a zip".to_vec()).unwrap_err();
    assert!(matches!(err, Error::Zip(_)));
}
