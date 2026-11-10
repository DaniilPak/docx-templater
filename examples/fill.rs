//! Fill a template from the command line:
//!
//! ```sh
//! cargo run --example fill -- template.docx data.json output.docx [--paragraph-loop] [--linebreaks]
//! ```

use docx_templater::{Options, Template};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let [template, data, output] = positional[..] else {
        eprintln!("usage: fill <template.docx|pptx> <data.json> <output> [--paragraph-loop] [--linebreaks]");
        std::process::exit(2);
    };
    let options = Options::new()
        .paragraph_loop(args.iter().any(|a| a == "--paragraph-loop"))
        .linebreaks(args.iter().any(|a| a == "--linebreaks"));
    let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(data)?)?;
    let template = match Template::from_path_with_options(template, options) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    template.render(&data)?.save(output)?;
    Ok(())
}
