use clap::{command, Parser, Subcommand};
use gen_cli_docs::gen_docs;

mod gen_cli_docs;

#[derive(Debug, Parser)]
#[command(name = "xtask")]
#[command(about = "Execute maintenance tasks for the `rawbit` project.", long_about = None)]
struct TaskArgs {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    GenCliDocs,
}

fn main() {
    let args = TaskArgs::parse();
    match args.command {
        Command::GenCliDocs => {
            gen_docs();
        }
    }
}
