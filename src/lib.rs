macro_rules! examples {
  ($($line:literal),+ $(,)?) => {
    concat!("Examples:", $("\n  ", $line),+)
  };
}

mod api;
mod cli;
mod commands;
mod context;
mod credentials;
mod error;
mod jq;
mod keychain;
mod login_page;
mod oauth;
mod output;
mod time;
mod ui;

use std::process::ExitCode;

use clap::Parser;
use clap::error::ErrorKind;

use crate::cli::{Cli, GlobalArgs};
use crate::context::Context;
use crate::error::{Error, ErrorItem};

pub async fn run() -> ExitCode {
  let argv: Vec<String> = std::env::args_os()
    .map(|arg| arg.to_string_lossy().into_owned())
    .collect();

  let cli = match Cli::try_parse_from(&argv) {
    Ok(cli) => cli,
    Err(error) => return print_parse_error(&error, argv),
  };

  set_color_choice(&cli.global);
  let context = Context::new(&cli.global, argv);
  let Some(command) = cli.command else {
    return print_home(&context);
  };
  context.output.begin();
  let result = commands::run(command, &context).await;
  ui::style::show_cursor();
  report(result, &context)
}

fn print_parse_error(error: &clap::Error, argv: Vec<String>) -> ExitCode {
  let (path, global) = commands::help::read_invocation(&argv);
  let context = Context::new(&global, argv);

  if error.kind() == ErrorKind::DisplayHelp && global.json {
    let args = commands::help::HelpArgs { path, all: false };
    return report(commands::help::run(&args, &context), &context);
  }

  let is_help =
    matches!(error.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
  if error.kind() == ErrorKind::DisplayHelp
    && context.output.is_stdout_tty()
    && commands::help::print_help_for(context.output.argv())
  {
    return ExitCode::SUCCESS;
  }
  if is_help || !context.output.has_structured_errors() {
    let _ = error.print();
    return ExitCode::from(if is_help { 0 } else { 2 });
  }

  let is_missing = matches!(
    error.kind(),
    ErrorKind::MissingRequiredArgument
      | ErrorKind::MissingSubcommand
      | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
  );
  let message = error.kind().as_str().unwrap_or("The command is not valid");
  let rendered = error.render().to_string();
  let detail = rendered.split("\n\nUsage:").next().unwrap_or_default();
  let detail = detail.split("\n\nFor more information").next().unwrap_or_default();
  let detail = detail.trim().trim_start_matches("error: ");

  let mut usage_error = Error::usage(
    if is_missing { "missing_input" } else { "usage_error" },
    format!("{message}."),
  );
  usage_error.details = vec![ErrorItem {
    message: detail.to_owned(),
    location: "command".to_owned(),
  }];
  if is_missing {
    let command = ["inboxjot"].into_iter().chain(path.iter().map(String::as_str));
    usage_error = usage_error.with_next(
      format!("{} --help", command.collect::<Vec<_>>().join(" ")),
      "Show the arguments and options of the command",
    );
  }

  report(Err(usage_error), &context)
}

fn print_home(context: &Context) -> ExitCode {
  if context.output.is_json() {
    let args = commands::help::HelpArgs { path: Vec::new(), all: false };
    return report(commands::help::run(&args, context), context);
  }

  context.output.begin();
  context.output.print_header();
  let mut command = <Cli as clap::CommandFactory>::command();
  if context.output.is_rich() && context.output.is_stdout_tty() {
    command.build();
    for line in commands::help::render_help(&command, "inboxjot") {
      anstream::println!("{line}");
    }
    return ExitCode::SUCCESS;
  }
  anstream::print!("{}", command.render_long_help().ansi());
  ExitCode::SUCCESS
}

fn report(result: error::Result<()>, context: &Context) -> ExitCode {
  match result {
    Ok(()) => ExitCode::SUCCESS,
    Err(error) => {
      context.output.print_error(&error);
      error.exit_code()
    }
  }
}

fn set_color_choice(global: &GlobalArgs) {
  let force_color = std::env::var("FORCE_COLOR").ok();
  let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
  let choice = if global.no_color || no_color {
    anstream::ColorChoice::Never
  } else {
    match force_color.as_deref() {
      Some("0") => anstream::ColorChoice::Never,
      Some(_) => anstream::ColorChoice::Always,
      None => anstream::ColorChoice::Auto,
    }
  };

  choice.write_global();
}
