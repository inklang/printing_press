use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "printing_press")]
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
    },
}

fn main() {
    let args = Args::parse();
    match args.command {
        Command::Compile { input, output } => {
            let source = std::fs::read_to_string(&input).unwrap();
            let script = printing_press::compile(&source, "main");
            let json = serde_json::to_string_pretty(&script).unwrap();
            std::fs::write(&output, json).unwrap();
            println!("Compiled {} → {}", input, output);
        }
    }
}
