//! Feature tests on small generated documents. Each paragraph built with
//! `p(&[...])` puts every fragment in its own `<w:r>` run, the way Word splits
//! text when spell-checking or formatting part of a word.

mod common;

use common::{body_of, docx, docx_with_parts, p, unzip, xml_of};
use docx_templater::{Options, Syntax, Template};
use serde::Serialize;
use serde_json::{Value, json};

fn render_text(body: &str, opts: Options, data: Value) -> String {
    let tpl = Template::with_options(docx(body), opts).expect("compiles");
    tpl.render(&data).expect("renders").full_text()
}

fn render_body(body: &str, opts: Options, data: Value) -> String {
    let tpl = Template::with_options(docx(body), opts).expect("compiles");
    body_of(&tpl.render(&data).expect("renders").to_bytes().unwrap())
}

#[test]
fn simple_placeholder() {
    assert_eq!(render_text(&p(&["Hello {name}!"]), Options::new(), json!({"name": "John"})), "Hello John!");
}

#[test]
fn tag_split_across_runs() {
    // Word often produces: `{` `na` `me` `}` in different runs
    let body = p(&["Hello ", "{", "na", "me", "}", ", you owe {amo", "unt} EUR"]);
    assert_eq!(render_text(&body, Options::new(), json!({"name": "John", "amount": 12})), "Hello John, you owe 12 EUR");
    // The value goes into the run that contains the opening delimiter; the other runs are emptied.
    let out = render_body(&body, Options::new(), json!({"name": "John", "amount": 12}));
    assert!(out.contains(r#"<w:t xml:space="preserve">John</w:t>"#), "{out}");
    assert!(out.contains("<w:t/>"), "{out}");
}

#[test]
fn loop_tags_split_across_runs() {
    let body = format!("{}{}{}", p(&["{#us", "ers}"]), p(&["- {na", "me}"]), p(&["{/", "users}"]));
    let data = json!({"users": [{"name": "A"}, {"name": "B"}]});
    assert_eq!(render_text(&body, Options::new().paragraph_loop(true), data), "- A- B");
}

#[test]
fn delimiter_split_across_runs() {
    let body = p(&["Hi {", "{name}", "}!"]);
    assert_eq!(render_text(&body, Options::new().delimiters("{{", "}}"), json!({"name": "Ann"})), "Hi Ann!");
}

#[test]
fn values_are_xml_escaped() {
    let out = render_body(&p(&["{v}"]), Options::new(), json!({"v": "<b>&'\""}));
    assert!(out.contains("&lt;b&gt;&amp;&apos;&quot;"), "{out}");
}

#[test]
fn types_are_stringified_like_javascript() {
    let body = p(&["{i}|{f}|{t}|{a}|{o}"]);
    let data = json!({"i": 42, "f": 1.5, "t": true, "a": [1, 2], "o": {"k": 1}});
    assert_eq!(render_text(&body, Options::new(), data), "42|1.5|true|1,2|[object Object]");
}

#[test]
fn dotted_paths_and_dot() {
    let body = p(&["{user.address.city} {items.1} {#tags}[{.}]{/tags}"]);
    let data = json!({"user": {"address": {"city": "Paris"}}, "items": ["a", "b"], "tags": ["x", "y"]});
    assert_eq!(render_text(&body, Options::new(), data), "Paris b [x][y]");
}

#[test]
fn missing_values_default_to_undefined_like_docxtemplater() {
    assert_eq!(render_text(&p(&["[{missing}]"]), Options::new(), json!({})), "[undefined]");
    assert_eq!(render_text(&p(&["[{nil}]"]), Options::new(), json!({"nil": null})), "[undefined]");
}

#[test]
fn custom_null_getter() {
    let opts = Options::new().null_getter(|ctx| match ctx.module {
        None => json!(format!("<{}?>", ctx.tag)),
        Some(_) => Value::Null,
    });
    assert_eq!(render_text(&p(&["{a} {#l}x{/l}"]), opts, json!({})), "<a?> ");
}

#[test]
fn inline_loop() {
    let body = p(&["{#items}{name}, {/items}end"]);
    let data = json!({"items": [{"name": "a"}, {"name": "b"}, {"name": "c"}]});
    assert_eq!(render_text(&body, Options::new(), data), "a, b, c, end");
}

#[test]
fn nested_loops_and_parent_scope_lookup() {
    let body = format!(
        "{}{}{}{}{}",
        p(&["{#companies}"]),
        p(&["{name}:"]),
        p(&["{#users}{name}@{company_id}{/users}"]),
        p(&["{/companies}"]),
        p(&["{title}"]),
    );
    let data = json!({
        "title": "T",
        "companies": [
            {"name": "Acme", "company_id": 1, "users": [{"name": "John"}, {"name": "Jane"}]},
            {"name": "Ecma", "company_id": 2, "users": [{"name": "Bob"}]},
        ]
    });
    assert_eq!(render_text(&body, Options::new().paragraph_loop(true), data), "Acme:John@1Jane@1Ecma:Bob@2T");
}

#[test]
fn conditions_and_inverted_sections() {
    let body = p(&[
        "{#ok}yes{/ok}{^ok}no{/ok}|{#empty}E{/empty}{^empty}none{/empty}|{#obj}{k}{/obj}|{#zero}Z{/zero}{^zero}!Z{/zero}",
    ]);
    let data = json!({"ok": true, "empty": [], "obj": {"k": "v"}, "zero": 0});
    assert_eq!(render_text(&body, Options::new(), data), "yes|none|v|!Z");
    let data = json!({"ok": false, "empty": [1], "obj": null, "zero": 1});
    assert_eq!(render_text(&body, Options::new(), data), "no|E||Z");
}

#[test]
fn paragraph_loop_removes_tag_paragraphs() {
    let body = format!("{}{}{}{}", p(&["{#users}"]), p(&["Hello {name}"]), p(&["{/users}"]), p(&["After"]));
    let data = json!({"users": [{"name": "A"}, {"name": "B"}]});
    let with = render_body(&body, Options::new().paragraph_loop(true), data.clone());
    assert_eq!(with.matches("<w:p>").count(), 3, "{with}");
    let without = render_body(&body, Options::new(), data);
    // without paragraphLoop the paragraphs that held the tags stay (empty)
    assert_eq!(without.matches("<w:p>").count(), 6, "{without}");
}

#[test]
fn table_row_loop() {
    let cell = |t: &str| format!("<w:tc><w:tcPr/>{}</w:tc>", p(&[t]));
    let header = format!("<w:tr>{}{}</w:tr>", cell("Name"), cell("Price"));
    let row = format!("<w:tr>{}{}</w:tr>", cell("{#products}{name}"), cell("{price}{/products}"));
    let body = format!("<w:tbl>{header}{row}</w:tbl>{}", p(&["after"]));
    let data = json!({"products": [{"name": "A", "price": 1}, {"name": "B", "price": 2}, {"name": "C", "price": 3}]});
    let out = render_body(&body, Options::new(), data);
    assert_eq!(out.matches("<w:tr>").count(), 4, "{out}");
    let tpl = Template::from_bytes(docx(&body)).unwrap();
    assert_eq!(tpl.render(&json!({"products": [{"name": "A", "price": 1}]})).unwrap().full_text(), "NamePriceA1after");
    // an empty loop removes the row entirely
    let out = render_body(&body, Options::new(), json!({"products": []}));
    assert_eq!(out.matches("<w:tr>").count(), 1, "{out}");
}

#[test]
fn empty_table_is_dropped() {
    let row = format!("<w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr>", p(&["{#rows}{.}"]), p(&["{/rows}"]));
    let body = format!("<w:tbl><w:tblPr/>{row}</w:tbl>{}", p(&["x"]));
    let out = render_body(&body, Options::new(), json!({"rows": []}));
    assert!(!out.contains("<w:tbl>"), "{out}");
}

#[test]
fn list_item_loop() {
    let li = |t: &str| {
        format!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>{t}</w:t></w:r></w:p>"#
        )
    };
    let body = format!("{}{}{}", li("{#items}"), li("{.}"), li("{/items}"));
    let out = render_body(&body, Options::new().paragraph_loop(true), json!({"items": ["one", "two", "three"]}));
    assert_eq!(out.matches("<w:numPr>").count(), 3, "{out}");
    assert!(out.contains(">one<") && out.contains(">two<") && out.contains(">three<"));
    // dash syntax repeats the w:p explicitly
    let body = li("{-w:p items}{.}{/items}");
    let out = render_body(&body, Options::new(), json!({"items": ["a", "b"]}));
    assert_eq!(out.matches("<w:numPr>").count(), 2, "{out}");
}

#[test]
fn raw_xml_replaces_paragraph() {
    let body = format!("{}{}", p(&["{@", "raw}"]), p(&["after"]));
    let raw = r#"<w:p><w:r><w:rPr><w:color w:val="FF0000"/></w:rPr><w:t>red</w:t></w:r></w:p>"#;
    let out = render_body(&body, Options::new(), json!({"raw": raw}));
    assert!(out.starts_with(raw), "{out}");
    // a falsy value removes the paragraph
    let out = render_body(&body, Options::new(), json!({}));
    assert_eq!(out.matches("<w:p>").count(), 1, "{out}");
}

#[test]
fn linebreaks_option_keeps_run_properties() {
    let body = r#"<w:p><w:r><w:rPr><w:i/></w:rPr><w:t>{text}</w:t></w:r></w:p>"#;
    let out = render_body(body, Options::new().linebreaks(true), json!({"text": "a\nb"}));
    assert_eq!(
        out,
        r#"<w:p><w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve">a</w:t></w:r><w:r><w:br/></w:r><w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve">b</w:t></w:r></w:p>"#
    );
    let out = render_body(body, Options::new(), json!({"text": "a\nb"}));
    assert!(out.contains("a\nb"));
}

#[test]
fn custom_and_changed_delimiters() {
    let body = p(&["[[name]] {x}"]);
    let out = render_text(&body, Options::new().delimiters("[[", "]]"), json!({"name": "N", "x": 1}));
    assert_eq!(out, "N {x}");
    // `{=[% %]=}` switches delimiters for the rest of the document, even split across runs
    let body = format!("{}{}", p(&["{name} {=[% ", "%]=}[%x%] [%={ }=%]{y}"]), p(&["{z}"]));
    let out = render_text(&body, Options::new(), json!({"name": "N", "x": 1, "y": 2, "z": 3}));
    assert_eq!(out, "N 1 23");
    // `<` and `>` work as delimiters too (they are escaped in the XML)
    let out = render_text(&p(&["Hi &lt;name&gt;"]), Options::new().delimiters("<", ">"), json!({"name": "Bo"}));
    assert_eq!(out, "Hi Bo");
    let syntax = Syntax { change_delimiter_prefix: None, ..Syntax::default() };
    let err = Template::with_options(docx(&p(&["{=<% %>=}"])), Options::new().syntax(syntax));
    assert!(err.is_ok());
}

#[test]
fn headers_footers_and_footnotes_are_templated() {
    let bytes = docx_with_parts(
        &p(&["Body {a}"]),
        Some(&p(&["Header {b.c}"])),
        Some(&p(&["{#f}Footer {.}{/f}"])),
        Some(r#"<w:footnote w:id="1"><w:p><w:r><w:t>Note {n}</w:t></w:r></w:p></w:footnote>"#),
    );
    let tpl = Template::from_bytes(bytes).unwrap();
    let files: Vec<_> = tpl.templated_files().collect();
    assert_eq!(files, vec!["word/document.xml", "word/header1.xml", "word/footer1.xml", "word/footnotes.xml"]);
    let r = tpl.render(&json!({"a": 1, "b": {"c": 2}, "f": [3, 4], "n": 5})).unwrap();
    assert_eq!(r.full_text(), "Body 1");
    assert_eq!(r.full_text_of("word/header1.xml"), "Header 2");
    assert_eq!(r.full_text_of("word/footer1.xml"), "Footer 3Footer 4");
    assert_eq!(r.full_text_of("word/footnotes.xml"), "Note 5");
    let out = r.to_bytes().unwrap();
    assert!(xml_of(&out, "word/footnotes.xml").contains("Note 5"));
}

#[test]
fn serde_structs_and_render_twice() {
    #[derive(Serialize)]
    struct Item {
        name: &'static str,
        qty: u32,
    }
    #[derive(Serialize)]
    struct Invoice {
        client: &'static str,
        items: Vec<Item>,
    }
    let tpl = Template::from_bytes(docx(&p(&["{client}: {#items}{qty}x{name} {/items}"]))).unwrap();
    let a = Invoice { client: "ACME", items: vec![Item { name: "rocket", qty: 2 }] };
    let b = Invoice { client: "Ecma", items: vec![] };
    assert_eq!(tpl.render(&a).unwrap().full_text(), "ACME: 2xrocket ");
    assert_eq!(tpl.render(&b).unwrap().full_text(), "Ecma: ");
}

#[test]
fn untouched_entries_are_copied() {
    let bytes = docx(&p(&["{a}"]));
    let out = Template::from_bytes(bytes.clone()).unwrap().render(&json!({"a": "x"})).unwrap().to_bytes().unwrap();
    let before = unzip(&bytes);
    let after = unzip(&out);
    assert_eq!(before.keys().collect::<Vec<_>>(), after.keys().collect::<Vec<_>>());
    assert_eq!(before["_rels/.rels"], after["_rels/.rels"]);
    assert_eq!(before["[Content_Types].xml"], after["[Content_Types].xml"]);
}

#[test]
fn template_full_text_and_debug() {
    let tpl = Template::from_bytes(docx(&p(&["Hello ", "{name}"]))).unwrap();
    assert_eq!(tpl.full_text(), "Hello {name}");
    assert_eq!(tpl.file_type(), docx_templater::FileType::Docx);
    assert!(format!("{tpl:?}").contains("word/document.xml"));
}

#[test]
fn custom_parser_receives_scopes() {
    let opts = Options::new().parser(|tag, ctx| {
        if let Some(rest) = tag.strip_prefix("upper ") {
            return Ok(ctx.scope.get(rest).and_then(Value::as_str).map(|s| json!(s.to_uppercase())));
        }
        Ok(ctx.scope.get(tag).cloned())
    });
    let body = p(&["{#users}{upper name}/{city} {/users}"]);
    let data = json!({"city": "Paris", "users": [{"name": "ann"}, {"name": "bob", "city": "Rome"}]});
    assert_eq!(render_text(&body, opts, data), "ANN/Paris BOB/Rome ");
}

#[test]
fn doc_pr_ids_are_unique_after_loops() {
    let drawing =
        r#"<w:p><w:r><w:drawing><wp:inline><wp:docPr id="7" name="Picture 1"/></wp:inline></w:drawing></w:r></w:p>"#;
    let body = format!("{}{drawing}{}", p(&["{#l}"]), p(&["{/l}"]));
    let out = render_body(&body, Options::new().paragraph_loop(true), json!({"l": [1, 2, 3]}));
    for id in 1..=3 {
        assert!(out.contains(&format!(r#"<wp:docPr id="{id}""#)), "{out}");
    }
    let out = render_body(&body, Options::new().paragraph_loop(true).fix_doc_pr_ids(false), json!({"l": [1, 2]}));
    assert_eq!(out.matches(r#"id="7""#).count(), 2);
}

#[test]
fn anonymous_closing_tag_and_this() {
    let body = p(&["{#users}{this.name};{/}"]);
    assert_eq!(render_text(&body, Options::new(), json!({"users": [{"name": "a"}, {"name": "b"}]})), "a;b;");
}

#[test]
fn trimmed_tags() {
    assert_eq!(render_text(&p(&["{ name }"]), Options::new(), json!({"name": "x"})), "x");
}

#[test]
fn preserve_newlines_in_tags() {
    // a tag spanning a line break: `{first<w:br/>name}`
    let body = r#"<w:p><w:r><w:t>{first</w:t></w:r><w:r><w:br/><w:t>name}</w:t></w:r></w:p>"#;
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    let opts =
        Options::new().syntax(Syntax { preserve_newlines_in_tags: true, ..Syntax::default() }).parser(move |tag, _| {
            s.lock().unwrap().push(tag.to_string());
            Ok(Some(json!("ok")))
        });
    assert_eq!(render_text(body, opts, json!({})), "ok");
    assert_eq!(seen.lock().unwrap()[0], "first\nname");
    // by default the break is ignored
    assert_eq!(render_text(body, Options::new(), json!({"firstname": "F"})), "F");
}

#[test]
fn allow_unclosed_and_unopened_tags() {
    let syntax = Syntax { allow_unclosed_tag: true, allow_unopened_tag: true, ..Syntax::default() };
    let out = render_text(&p(&["a { b {x} c } d"]), Options::new().syntax(syntax), json!({"x": 1}));
    assert_eq!(out, "a { b 1 c } d");
}
