pub mod convert;
pub mod default;
pub mod infer;
pub mod train;

// TODO: burn with tui/train sets up its own tracing. I might want to figure out
// a better option here.
pub(crate) fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
}

use clap::{Parser, Subcommand};
use convert::ConvertArgs;
use infer::InferArgs;
use train::TrainArgs;

#[derive(Parser)]
#[command(name = "ma", about = "abominable intelligence shenanigans", long_version = lib::build_info::VERSTR)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

// TODO: For one off runs via the `ma` cli this is fine. But if I ever merge
// this back into the main crate with a ui it might cause issues with the stack.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Command {
    /// Display the default burn model architecture
    Default,
    /// Convert etl files to paired npz datasets for training
    Convert(ConvertArgs),
    /// Infer using a trained model, kinda useless tbh, here from burn examples really
    Infer(InferArgs),
    /// Train a model to be slightly useful
    Train(TrainArgs),
}

pub fn run() {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Train(TrainArgs::default())) {
        Command::Default => default::run(),
        Command::Convert(args) => convert::run(args),
        Command::Infer(args) => infer::run(args),
        Command::Train(args) => train::run(args),
    }
}
