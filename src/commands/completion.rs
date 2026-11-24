use std::io::Write;

use clap::CommandFactory;

use crate::cli::Cli;
use crate::error::Result;

#[derive(Debug, clap::Args)]
pub struct CompletionArgs {
  #[arg(value_enum, help = "Shell to write the script for")]
  pub shell: clap_complete::Shell,
}

pub fn run(args: &CompletionArgs) -> Result<()> {
  let mut script = Vec::new();
  clap_complete::generate(args.shell, &mut Cli::command(), "inboxjot", &mut script);
  match std::io::stdout().lock().write_all(&script) {
    Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
    result => Ok(result?),
  }
}
