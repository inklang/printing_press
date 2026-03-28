use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "printing_press", version = "0.1.0", about = "Inklang compiler")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Parser, Debug)]
enum Command {
    Compile(CompileArgs),
}

#[derive(Parser, Debug)]
struct CompileArgs {
    /// Source file (single-file mode)
    #[arg(value_name = "INPUT")]
    input: Option<String>,

    /// Output file (single-file mode)
    #[arg(short, long, value_name = "OUTPUT")]
    output: Option<String>,

    /// Directory containing .ink source files (batch mode)
    #[arg(long, value_name = "DIR")]
    sources: Option<String>,

    /// Output directory (batch mode)
    #[arg(long, value_name = "DIR")]
    out: Option<String>,

    /// Grammar IR file (.json) to use for compilation
    #[arg(long, value_name = "PATH")]
    grammar: Option<String>,

    /// Pretty-print JSON output
    #[arg(short, long)]
    debug: bool,
}

fn main() {
    let args = Args::parse();
    match args.command {
        Command::Compile(c) => {
            // Use explicit --grammar if provided, otherwise auto-discover
            let grammar = if let Some(ref grammar_path) = c.grammar {
                match printing_press::inklang::grammar::load_grammar(grammar_path) {
                    Ok(pkg) => Some(printing_press::inklang::grammar::merge_grammars(vec![pkg])),
                    Err(e) => {
                        eprintln!("error: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                printing_press::inklang::grammar::discover_grammars()
            };

            // Determine mode: if --sources provided, batch mode
            if let Some(sources_dir) = c.sources {
                let out_dir = c.out.expect("--out is required in batch mode");
                batch_compile(&sources_dir, &out_dir, grammar.as_ref(), c.debug);
            } else {
                // Single-file mode
                let input = c.input.expect("INPUT file or --sources required");
                let output = c.output.expect("-o/--output required in single-file mode");
                single_compile(&input, &output, grammar.as_ref(), c.debug);
            }
        }
    }
}

fn single_compile(input: &str, output: &str, grammar: Option<&printing_press::inklang::grammar::MergedGrammar>, debug: bool) {
    match std::fs::read_to_string(input) {
        Ok(source) => {
            let result = if let Some(g) = grammar {
                printing_press::compile_with_grammar(&source, "main", Some(g))
            } else {
                printing_press::compile(&source, "main").map_err(|e| e.into())
            };
            match result {
                Ok(script) => {
                    let json = if debug {
                        serde_json::to_string_pretty(&script).unwrap()
                    } else {
                        serde_json::to_string(&script).unwrap()
                    };
                    std::fs::write(output, json).unwrap();
                    println!("Compiled {} → {}", input, output);
                }
                Err(e) => {
                    eprintln!("error: compilation failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("error: could not read file '{}': {}", input, e);
            std::process::exit(1);
        }
    }
}

fn batch_compile(sources_dir: &str, out_dir: &str, grammar: Option<&printing_press::inklang::grammar::MergedGrammar>, debug: bool) {
    let src_path = std::path::Path::new(sources_dir);
    let out_path = std::path::Path::new(out_dir);

    if let Err(e) = std::fs::create_dir_all(out_path) {
        eprintln!("error: could not create output directory '{}': {}", out_dir, e);
        std::process::exit(1);
    }

    let entries: Vec<_> = match std::fs::read_dir(src_path) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            eprintln!("error: could not read directory '{}': {}", sources_dir, e);
            std::process::exit(1);
        }
    };

    let entries: Vec<_> = entries
        .into_iter()
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "ink"))
        .collect();

    if entries.is_empty() {
        println!("No .ink files found in {}", sources_dir);
        return;
    }

    let mut errors = 0;
    for entry in entries {
        let input_path = entry.path();
        let file_name = input_path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let output_path = out_path.join(format!("{}.inkc", file_name));

        let source = match std::fs::read_to_string(&input_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: could not read file '{}': {}", input_path.display(), e);
                errors += 1;
                continue;
            }
        };

        let result = if let Some(g) = grammar {
            printing_press::compile_with_grammar(&source, file_name, Some(g))
        } else {
            printing_press::compile(&source, file_name).map_err(|e| e.into())
        };

        match result {
            Ok(script) => {
                let json = if debug {
                    serde_json::to_string_pretty(&script).unwrap()
                } else {
                    serde_json::to_string(&script).unwrap()
                };
                std::fs::write(&output_path, json).unwrap();
                println!("Compiled {} → {}", input_path.file_name().unwrap().to_str().unwrap(), output_path.file_name().unwrap().to_str().unwrap());
            }
            Err(e) => {
                eprintln!("error: compilation failed for '{}': {}", input_path.display(), e);
                errors += 1;
            }
        }
    }

    if errors > 0 {
        eprintln!("{} file(s) failed to compile", errors);
        std::process::exit(1);
    }
}
