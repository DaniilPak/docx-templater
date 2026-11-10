# docx-templater

[![CI](https://github.com/DaniilPak/docx-templater/actions/workflows/ci.yml/badge.svg)](https://github.com/DaniilPak/docx-templater/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/docx-templater.svg)](https://crates.io/crates/docx-templater)
[![docs.rs](https://docs.rs/docx-templater/badge.svg)](https://docs.rs/docx-templater)

Fill Word (`.docx`) and PowerPoint (`.pptx`) templates with JSON or any
`serde::Serialize` data, in pure Rust.

It's a port of the open-source core of the JavaScript library
[docxtemplater](https://github.com/open-xml-templating/docxtemplater) and uses
the same tag syntax, so templates written for docxtemplater work unchanged.
Like docxtemplater, it handles tags that Word has split across several
`<w:r>`/`<w:t>` runs (spell-check marks, partial formatting, revision ids), so
templates typed in real Word just work.

```toml
[dependencies]
docx-templater = "0.1"
serde_json = "1"
```

```rust
use docx_templater::{Options, Template};
use serde_json::json;

let template = Template::with_options(
    std::fs::read("invoice-template.docx")?,
    Options::new().paragraph_loop(true),
)?;

let output: Vec<u8> = template
    .render(&json!({
        "client": { "name": "ACME Corp", "city": "Paris" },
        "items": [
            { "name": "Rocket", "qty": 2, "price": 1000 },
            { "name": "Anvil",  "qty": 1, "price": 30 },
        ],
        "paid": false,
    }))?
    .to_bytes()?;

std::fs::write("invoice.docx", output)?;
```

A template compiles once and can be rendered any number of times
(`Template` is `Send + Sync`). Any `Serialize` type works as data:

```rust
#[derive(serde::Serialize)]
struct Invoice { client: String, items: Vec<Item> }

let bytes = template.render(&invoice)?.to_bytes()?;
```

From the command line: `cargo run --example fill -- template.docx data.json out.docx --paragraph-loop`.

## Syntax

| Tag | Meaning |
| --- | --- |
| `{name}` | Value. Special characters are XML-escaped. Numbers, booleans and arrays are printed the way JavaScript prints them (`1.5`, `true`, `a,b`). |
| `{user.address.city}`, `{items.0}` | Nested values (dotted paths, array indexes). |
| `{.}` / `{this}` | The current loop item, e.g. `{#tags}{.}, {/tags}`. |
| `{#items}…{/items}` | Loop over an array. Inside the loop, tags are looked up on the item first, then in enclosing scopes. |
| `{#cond}…{/cond}` | Condition: shown once if the value is truthy (`true`, non-empty string, non-zero number, object). An object value becomes the scope. |
| `{^cond}…{/cond}` | Inverted section: shown if the value is falsy, missing, or an empty array. |
| `{/}` | Closes the innermost open loop, whatever its name. |
| `{@raw}` | Insert raw OOXML. Replaces the whole paragraph (`w:p`), or the whole shape (`p:sp`) in pptx, so the tag must be the only text in its paragraph. |
| `{-w:p items}…{/items}` | Loop that repeats an explicit enclosing element (`w:p`, `w:tr`, `w:tbl`, `a:p`…). |
| `{=[[ ]]=}` | Change the delimiters for the rest of the document. |

What a loop repeats is chosen automatically:

* **Inline**: both tags in the same paragraph. Only the content between them repeats.
* **Table rows**: the tags are in different cells of the same table. The whole
  `<w:tr>` repeats, so `| {#products}{name} | {price}{/products} |` gives one row
  per product, and an empty array removes the row. Same for `a:tr` in pptx.
* **Paragraphs** (`paragraph_loop(true)`): each tag is alone in its own paragraph.
  The paragraphs in between repeat and the two tag paragraphs are removed.
  This is the right mode for repeating list items, headings or whole blocks.
  Without the option, the tag paragraphs stay behind as empty paragraphs (docxtemplater's default).

Templated parts: `word/document.xml`, headers, footers, footnotes, endnotes and
comments, docProps (`core.xml`, `app.xml`, `custom.xml`), and for pptx the
slides, slide layouts, slide masters and notes.

## Options

```rust
use docx_templater::{Options, Syntax};

let opts = Options::new()
    .paragraph_loop(true)          // docxtemplater's `paragraphLoop`
    .linebreaks(true)              // "\n" in values becomes <w:br/>, run formatting kept
    .delimiters("{{", "}}")        // custom delimiters
    .null_getter(|ctx| match ctx.module {          // docxtemplater's `nullGetter`
        None => serde_json::json!(""),             // plain tags: "" instead of "undefined"
        Some(_) => serde_json::Value::Null,        // loops / raw tags: render nothing
    })
    .parser(|tag, ctx| Ok(ctx.scope.get(tag.trim()).cloned())) // custom tag evaluator
    .strip_invalid_xml_chars(true) // drop control characters instead of failing
    .syntax(Syntax { allow_unbalanced_loops: true, ..Syntax::default() });
```

A missing or `null` value renders as `undefined`, which is docxtemplater's
default. Set `null_getter` to change it.

## Errors

Like docxtemplater's "Multi error", compiling a template reports every problem
in every templated part at once. Each problem carries the same `id`, `message`
and `explanation` as docxtemplater, plus the file and the offset of the tag:

```rust
match Template::from_bytes(bytes) {
    Err(docx_templater::Error::Template(errors)) => {
        for e in &errors {
            // e.g. "word/header1.xml: Unclosed tag (unclosed_tag): The tag beginning with "{title " is unclosed"
            eprintln!("{e}  [tag: {:?}, offset: {:?}]", e.xtag, e.offset);
        }
    }
    Err(other) => eprintln!("{other}"),
    Ok(template) => { /* ... */ }
}
```

Error ids: `unclosed_tag`, `unopened_tag`, `duplicate_open_tag`,
`duplicate_close_tag`, `unclosed_loop`, `unopened_loop`,
`closing_tag_does_not_match_opening_tag`, `unbalanced_loop_tags`,
`loop_position_invalid`, `no_xml_tag_found_at_left` / `_right`,
`raw_tag_outerxml_invalid`, `raw_xml_tag_should_be_only_text_in_paragraph`,
`change_delimiters_invalid`, `malformed_xml`, `file_has_invalid_xml`,
`filetype_not_identified`, `filetype_not_handled`. Rendering can also fail with
`invalid_xml_characters`, `invalid_raw_xml_value` or `scopeparser_execution_failed`.

## Parity with docxtemplater (open-source core)

The test suite renders 70 of docxtemplater's own test templates (Word- and
PowerPoint-authored files from its repository) with the same data and options
as docxtemplater's tests. It compares the output XML of every part with the
`expected-*.docx` / `.pptx` files that docxtemplater itself produced, or with
the text that docxtemplater's tests assert. All of them match.

| Feature | docxtemplater | docx-templater |
| --- | :-: | :-: |
| `{tag}` placeholders, XML escaping | ✅ | ✅ |
| Tags split across runs / text nodes | ✅ | ✅ |
| `{#loop}` over arrays, objects, truthy values | ✅ | ✅ |
| `{^inverted}` sections | ✅ | ✅ |
| `{/}` anonymous closing tag | ✅ | ✅ |
| Automatic table-row loops (`w:tr`, `a:tr`) | ✅ | ✅ |
| `paragraphLoop` option | ✅ | ✅ |
| `{-w:p loop}` dash loops | ✅ | ✅ |
| `{@raw}` raw XML | ✅ | ✅ |
| `linebreaks` option (keeps run properties) | ✅ | ✅ |
| Custom delimiters, `{=<% %>=}` delimiter change | ✅ | ✅ |
| `nullGetter` | ✅ | ✅ (`Options::null_getter`) |
| Custom `parser` | ✅ (JS function) | ✅ (Rust closure, sync) |
| Dotted paths `{a.b.c}` | only with the expressions parser | ✅ built in |
| Angular-style expressions (`{a + b}`, filters) | ✅ (`docxtemplater/expressions.js`) | ❌ (plug in your own via `Options::parser`) |
| Multi-error reporting with ids/offsets | ✅ | ✅ (same ids and messages) |
| `syntax.allowUnopenedTag` / `allowUnclosedTag` / `allowUnbalancedLoops` / `preserveNewlinesInTags` | ✅ | ✅ |
| `stripInvalidXMLChars` | ✅ | ✅ |
| Headers, footers, footnotes, comments, docProps | ✅ | ✅ (+ endnotes) |
| Section breaks / page breaks inside loops | ✅ | ✅ |
| Corruption fixes (empty cells, rows, tables, paragraph after table, `w:sdtContent`) | ✅ | ✅ |
| `.docx` / `.docm` / `.dotx` / `.dotm` | ✅ | ✅ |
| `.pptx` / `.pptm` (incl. table row height and `a16:rowId`) | ✅ | ✅ |
| Unique `wp:docPr` ids for images repeated by loops | opt-in module | ✅ on by default (`fix_doc_pr_ids`) |
| `getFullText()` | ✅ | ✅ (`full_text`, `full_text_of`) |
| Async data resolution (`renderAsync`, promises in data) | ✅ | ❌ (resolve your data before rendering) |
| Module API (custom modules) | ✅ | ❌ |
| `.xlsx`, `.odt` | paid modules | ❌ |
| Image, HTML, table, chart, subtemplate, … modules | paid PRO modules | ❌ (out of scope) |

Small deliberate differences:

* Tag expressions are trimmed, so `{ name }` and `{name}` are the same tag.
  docxtemplater's default parser treats `{name }` as a different key.
* Dotted paths are resolved by default. An exact key such as `"a.b"` still wins.
* `fix_doc_pr_ids` is on by default.

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
* MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

This crate ports the MIT-licensed open-source core of
[docxtemplater](https://github.com/open-xml-templating/docxtemplater) by Edgar
Hipp. Its copyright and MIT notice are kept in [LICENSE-MIT](LICENSE-MIT). The
test fixtures in `tests/fixtures/upstream` come unmodified from docxtemplater's
repository and are used under its MIT license. Many thanks to the docxtemplater
authors. If you need images, HTML or other advanced features in JavaScript,
look at their [PRO modules](https://docxtemplater.com/pro/).
