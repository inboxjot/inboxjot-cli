use clap::CommandFactory;

use crate::cli::Cli;
use crate::error::Result;

#[derive(Debug, clap::Args)]
pub struct CompletionArgs {
  #[arg(value_enum, help = "Shell to write the script for")]
  pub shell: clap_complete::Shell,
}

pub fn run(args: &CompletionArgs) -> Result<()> {
  clap_complete::generate(
    args.shell,
    &mut Cli::command(),
    "inboxjot",
    &mut std::io::stdout(),
  );
  Ok(())
}
