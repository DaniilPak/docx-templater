//! Parity tests against docxtemplater's own test-suite: the templates in
//! `tests/fixtures/upstream` (authored in Word / PowerPoint, MIT licensed,
//! copied from github.com/open-xml-templating/docxtemplater) are rendered with
//! the same data and options as upstream, and compared with the documents
//! docxtemplater itself produced (`expected-*.docx`), or with the text that
//! upstream asserts.

mod common;

use std::path::PathBuf;

use docx_templater::{Options, Syntax, Template};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Case {
    name: String,
    expected: Option<String>,
    data: Value,
    #[serde(default)]
    options: Value,
    #[serde(default)]
    texts: std::collections::BTreeMap<String, String>,
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upstream")
}

fn options(v: &Value) -> Options {
    let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let mut o = Options::new()
        .paragraph_loop(b("paragraph_loop"))
        .linebreaks(b("linebreaks"))
        // upstream only renumbers docPr ids with an opt-in module
        .fix_doc_pr_ids(b("fix_doc_pr_ids"));
    if let Some(d) = v.get("delimiters").and_then(Value::as_array) {
        o = o.delimiters(d[0].as_str().unwrap(), d[1].as_str().unwrap());
    }
    if b("strict_parser") {
        // docxtemplater's default parser: exact key lookup, no trimming, no dotted paths
        o = o.parser(|tag, ctx| {
            Ok(match tag {
                "." => Some(ctx.scope.clone()),
                _ => ctx.scope.get(tag).cloned(),
            })
        });
    }
    o = o.syntax(Syntax {
        allow_unbalanced_loops: b("allow_unbalanced_loops"),
        allow_unopened_tag: b("allow_unopened_tag"),
        ..Syntax::default()
    });
    o
}

#[test]
fn upstream_fixtures_render_like_docxtemplater() {
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(dir().join("cases.json")).unwrap()).unwrap();
    let only = std::env::var("CASE").ok();
    let mut failures = Vec::new();
    let mut ran = 0;
    for case in &cases {
        let label = format!("{} -> {}", case.name, case.expected.as_deref().unwrap_or("text"));
        if only.as_ref().is_some_and(|o| !label.contains(o.as_str())) {
            continue;
        }
        ran += 1;
        let bytes = std::fs::read(dir().join(&case.name)).unwrap();
        let tpl = match Template::with_options(bytes, options(&case.options)) {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("{label}: compile error: {e}"));
                continue;
            }
        };
        let rendered = match tpl.render(&case.data) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{label}: render error: {e}"));
                continue;
            }
        };
        for (file, text) in &case.texts {
            let actual = rendered.full_text_of(file);
            if &actual != text {
                failures.push(format!("{label}: text of {file}\n  actual:   {actual:?}\n  expected: {text:?}"));
            }
        }
        let out = rendered.to_bytes().unwrap();
        if let Some(expected) = &case.expected {
            let expected = std::fs::read(dir().join(expected)).unwrap();
            for d in common::zip_diff(&out, &expected) {
                failures.push(format!("{label}: {d}"));
            }
        }
    }
    assert!(ran > 0);
    assert!(failures.is_empty(), "{} failure(s) out of {ran} cases:\n{}", failures.len(), failures.join("\n\n"));
}
