use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "printing_press",
    version = "0.1.0",
    about = "Inklang compiler"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Parser, Debug)]
enum Command {
    Compile {
        #[arg(value_name = "INPUT")]
        input: String,
        #[arg(short, long, value_name = "OUTPUT")]
        output: String,
        #[arg(short, long)]
        debug: bool,
    },
}

fn main() {
    let args = Args::parse();
    match args.command {
        Command::Compile { input, output, debug } => {
            match std::fs::read_to_string(&input) {
                Ok(source) => {
                    match printing_press::compile(&source, "main") {
                        Ok(script) => {
                            let json = if debug {
                                serde_json::to_string_pretty(&script).unwrap()
                            } else {
                                serde_json::to_string(&script).unwrap()
                            };
                            std::fs::write(&output, json).unwrap();
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
    }
}
