/// Batch-parse Limbo source files to test parser coverage.
fn main() {
    let mut pass = 0;
    let mut fail = 0;
    let mut errors: Vec<(String, String)> = Vec::new();

    let dir = std::path::Path::new("external/inferno-os/appl/cmd");
    if !dir.exists() {
        eprintln!("Run from workspace root");
        std::process::exit(1);
    }

    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("b") {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let tokens = match ricevm_limbo::lexer::Lexer::new(&src, &name).tokenize() {
            Ok(t) => t,
            Err(e) => {
                fail += 1;
                errors.push((name, format!("lex: {e}")));
                continue;
            }
        };

        match ricevm_limbo::parser::Parser::new(tokens, &name).parse_file() {
            Ok(_) => pass += 1,
            Err(e) => {
                fail += 1;
                errors.push((name, format!("{e}")));
            }
        }
    }

    println!("Parsed {pass}/{} programs ({fail} failures)", pass + fail);
    if !errors.is_empty() {
        println!("\nFailures:");
        for (name, err) in &errors {
            println!("  {name}: {err}");
        }
    }
}
