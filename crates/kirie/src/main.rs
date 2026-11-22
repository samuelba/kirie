use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "kirie",
    version,
    about = "Cut a piece out of your screen and mark it up"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Take a screenshot.
    Capture,
    /// Open an image in the editor.
    Edit { path: std::path::PathBuf },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    match cli.command {
        None | Some(Command::Capture | Command::Edit { .. }) => {
            anyhow::bail!("not implemented yet")
        }
    }
}
