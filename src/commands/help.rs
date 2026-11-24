use clap::builder::ValueParser;
use clap::error::{ContextKind, ContextValue};
use clap::{Arg, ArgAction, ArgMatches, Command, CommandFactory, FromArgMatches};
use serde_json::{Value, json};
use unicode_width::UnicodeWidthStr;

use crate::cli::{Cli, GlobalArgs};
use crate::context::Context;
use crate::error::{Error, Result};
use crate::ui::style::{ACTION, DIM, SECONDARY, TITLE};
use crate::ui::text::{Line, Span};

const MAX_SEARCH_RESULTS: usize = 10;

#[derive(Debug, clap::Args)]
pub struct HelpArgs {
  #[arg(
    value_name = "COMMAND",
    help = "Command to explain, such as domains add, or search and a query"
  )]
  pub path: Vec<String>,

  #[arg(long, help = "List every command")]
  pub all: bool,
}

pub fn read_invocation(argv: &[String]) -> (Vec<String>, GlobalArgs) {
  let help = Arg::new("help")
    .short('h')
    .long("help")
    .global(true)
    .action(ArgAction::SetTrue);
  let lenient = accept_any_value(Cli::command())
    .disable_help_flag(true)
    .arg(help)
    .ignore_errors(true);
  let mut argv = argv.to_vec();
  while let Err(error) = Cli::command().try_get_matches_from(&argv)
    && let Some(index) = find_invalid_word(&error, &argv)
  {
    argv.remove(index);
  }

  let Ok(matches) = lenient.try_get_matches_from(&argv) else {
    return (Vec::new(), GlobalArgs::default());
  };

  let mut path = Vec::new();
  let mut deepest: &ArgMatches = &matches;
  while let Some((name, child)) = deepest.subcommand() {
    path.push(name.to_owned());
    deepest = child;
  }

  (path, GlobalArgs::from_arg_matches(deepest).unwrap_or_default())
}

fn accept_any_value(command: Command) -> Command {
  command
    .mut_args(|arg| {
      if arg.is_global_set() || !arg.get_action().takes_values() {
        return arg;
      }
      arg.value_parser(ValueParser::os_string())
    })
    .mut_subcommands(accept_any_value)
}

fn find_invalid_word(error: &clap::Error, argv: &[String]) -> Option<usize> {
  let invalid = error
    .get(ContextKind::InvalidSubcommand)
    .or_else(|| error.get(ContextKind::InvalidArg));
  let Some(ContextValue::String(invalid)) = invalid else {
    return None;
  };

  argv
    .iter()
    .skip(1)
    .position(|word| word == invalid)
    .map(|index| index + 1)
}

pub fn run(args: &HelpArgs, context: &Context) -> Result<()> {
  let root = build_root();

  if let Some((first, query)) = args.path.split_first()
    && first == "search"
  {
    if query.is_empty() {
      return Err(Error::usage(
        "missing_input",
        "Add a query, such as inboxjot help search webhook.",
      ));
    }
    return search(&root, &query.join(" "), context);
  }

  if args.all {
    return print_all(&root, context);
  }

  let (command, path) = match find_command(&root, &args.path) {
    Ok(found) => found,
    Err(_)
      if args
        .path
        .first()
        .is_some_and(|first| root.find_subcommand(first).is_none()) =>
    {
      return search(&root, &args.path.join(" "), context);
    }
    Err(error) => return Err(error),
  };
  if context.output.is_json() {
    return context.output.print_json(&describe(command, &path));
  }

  if context.output.is_stdout_tty() {
    for line in render_help(command, &path) {
      anstream::println!("{line}");
    }
    return Ok(());
  }

  anstream::println!("{}", command.clone().render_long_help().ansi());
  Ok(())
}

pub fn print_help_for(argv: &[String]) -> bool {
  let (path, _) = read_invocation(argv);
  let root = build_root();
  let Ok((command, path)) = find_command(&root, &path) else {
    return false;
  };
  anstream::println!();
  for line in render_help(command, &path) {
    anstream::println!("{line}");
  }
  true
}

pub fn render_help(command: &Command, path: &str) -> Vec<Line<'static>> {
  let heading = |text: &str| Line::from(Span::styled(format!("  {text}"), TITLE));
  let mut lines = Vec::new();
  let about = command
    .get_long_about()
    .or_else(|| command.get_about())
    .map(ToString::to_string)
    .unwrap_or_default();
  if !about.is_empty() {
    lines.push(heading(&about));
    lines.push(Line::blank());
  }

  let options: Vec<&Arg> = command
    .get_arguments()
    .filter(|arg| !arg.is_positional() && !arg.is_hide_set() && !arg.is_global_set())
    .filter(|arg| !matches!(arg.get_id().as_str(), "help" | "version"))
    .collect();
  let mut usage = vec![path.to_owned()];
  let subcommands: Vec<&Command> = command
    .get_subcommands()
    .filter(|child| !child.is_hide_set())
    .collect();
  if !subcommands.is_empty() {
    usage.push("<command>".to_owned());
  }
  for arg in command
    .get_arguments()
    .filter(|arg| arg.is_positional() && !arg.is_hide_set())
  {
    let name = get_value_name(arg).to_lowercase();
    usage.push(if arg.is_required_set() {
      format!("<{name}>")
    } else {
      format!("[{name}]")
    });
  }
  if !options.is_empty() {
    usage.push("[options]".to_owned());
  }
  lines.push(heading("Usage"));
  lines.push(Line::from(Span::raw(format!("    {}", usage.join(" ")))));

  let mut rows: Vec<(String, String)> = Vec::new();
  if !subcommands.is_empty() {
    lines.push(Line::blank());
    lines.push(heading("Commands"));
    rows = subcommands
      .iter()
      .map(|child| (child.get_name().to_owned(), get_summary(child)))
      .collect();
    push_rows(&mut lines, &rows);
  }

  if !options.is_empty() {
    lines.push(Line::blank());
    lines.push(heading("Options"));
    rows = options
      .iter()
      .map(|arg| (format_flag(arg), get_help(arg).unwrap_or_default()))
      .collect();
    push_rows(&mut lines, &rows);
  }

  let examples: Vec<String> = command
    .get_after_help()
    .map(ToString::to_string)
    .unwrap_or_default()
    .lines()
    .skip_while(|line| !line.starts_with("Examples"))
    .skip(1)
    .map(|line| line.trim().to_owned())
    .filter(|line| !line.is_empty())
    .collect();
  if !examples.is_empty() {
    lines.push(Line::blank());
    lines.push(heading("Examples"));
    for example in examples {
      lines.push(Line::new(vec![Span::styled("    $ ", DIM), Span::raw(example)]));
    }
  }

  if let Some(page) = path.split_whitespace().nth(1) {
    lines.push(Line::blank());
    lines.push(Line::new(vec![
      Span::styled("  Docs  ", DIM),
      Span::styled(format!("inboxjot.com/docs/cli/{page}"), SECONDARY),
    ]));
  }
  let _ = rows;
  lines
}

fn push_rows(lines: &mut Vec<Line<'static>>, rows: &[(String, String)]) {
  let width = rows.iter().map(|(name, _)| name.width()).max().unwrap_or(0).max(20) + 2;
  for (name, summary) in rows {
    lines.push(Line::new(vec![
      Span::styled(format!("    {name:<width$}"), ACTION),
      Span::styled(summary.clone(), SECONDARY),
    ]));
  }
}

fn format_flag(arg: &Arg) -> String {
  let long = arg.get_long().map(|long| format!("--{long}"));
  let short = arg.get_short().map(|short| format!("-{short}"));
  let mut flag = match (short, long) {
    (Some(short), Some(long)) => format!("{short}, {long}"),
    (None, Some(long)) => long,
    (Some(short), None) => short,
    (None, None) => arg.get_id().to_string(),
  };
  if arg.get_action().takes_values() {
    flag.push_str(&format!(" <{}>", get_value_name(arg).to_lowercase()));
  }
  flag
}

fn build_root() -> Command {
  let mut root = Cli::command();
  root.build();
  root
}

fn find_command<'a>(
  root: &'a Command,
  segments: &[String],
) -> Result<(&'a Command, String)> {
  let mut command = root;
  let mut path = root.get_name().to_owned();

  for segment in segments {
    let Some(subcommand) = command.find_subcommand(segment) else {
      return Err(
        Error::usage(
          "unknown_command",
          format!("{path} has no command named \"{segment}\"."),
        )
        .with_next(format!("inboxjot help search {segment}"), "Search the commands"),
      );
    };
    command = subcommand;
    path = format!("{path} {}", command.get_name());
  }

  Ok((command, path))
}

fn search(root: &Command, query: &str, context: &Context) -> Result<()> {
  let query = query.to_lowercase();
  let words: Vec<&str> = query.split_whitespace().collect();
  let mut matches: Vec<(String, String)> = list_commands(root)
    .into_iter()
    .skip(1)
    .map(|(path, command)| (path, get_summary(command)))
    .filter(|(path, summary)| {
      let text = format!("{path} {summary}").to_lowercase();
      words.iter().all(|word| text.contains(word))
    })
    .collect();
  matches.sort_by_key(|(path, _)| (!path.contains(&query), path.len()));
  matches.truncate(MAX_SEARCH_RESULTS);

  if context.output.is_json() || !context.output.is_stdout_tty() {
    if matches.is_empty() && !context.output.is_json() {
      anstream::eprintln!("No commands match \"{query}\".");
      return Ok(());
    }
    return print_rows(&matches, context);
  }

  if matches.is_empty() {
    anstream::println!(
      "{}",
      Line::from(Span::styled(format!("  No commands match “{query}”."), SECONDARY))
    );
  } else {
    let noun = if matches.len() == 1 { "command matches" } else { "commands match" };
    anstream::println!(
      "{}",
      Line::from(Span::styled(format!("  {} {noun} “{query}”", matches.len()), TITLE))
    );
    anstream::println!();
    let width = matches.iter().map(|(path, _)| path.width()).max().unwrap_or(0);
    for (path, summary) in &matches {
      let mut spans = vec![Span::raw("  ")];
      spans.extend(mark_match(path, &query));
      spans.push(Span::raw(" ".repeat(width - path.width() + 3)));
      spans.push(Span::styled(summary.clone(), SECONDARY));
      anstream::println!("{}", Line::new(spans));
    }
  }
  anstream::println!();
  anstream::println!(
    "{}",
    Line::new(vec![
      Span::styled("  Run ", DIM),
      Span::styled("inboxjot <command> --help", ACTION),
      Span::styled(" for options and examples.", DIM),
    ])
  );
  Ok(())
}

fn mark_match(path: &str, query: &str) -> Vec<Span<'static>> {
  let Some(start) = path.to_lowercase().find(query).filter(|_| !query.is_empty()) else {
    return vec![Span::raw(path.to_owned())];
  };
  let end = start + query.len();
  vec![
    Span::raw(path[..start].to_owned()),
    Span::styled(path[start..end].to_owned(), ACTION.bold()),
    Span::raw(path[end..].to_owned()),
  ]
}

fn print_all(root: &Command, context: &Context) -> Result<()> {
  let commands = list_commands(root);
  if context.output.is_json() {
    let tree: Vec<Value> = commands
      .iter()
      .map(|(path, command)| describe(command, path))
      .collect();
    return context.output.print_json(&json!({ "commands": tree }));
  }

  let rows: Vec<(String, String)> = commands
    .into_iter()
    .skip(1)
    .map(|(path, command)| (path, get_summary(command)))
    .collect();
  print_rows(&rows, context)
}

fn print_rows(rows: &[(String, String)], context: &Context) -> Result<()> {
  if context.output.is_json() {
    let commands: Vec<Value> = rows
      .iter()
      .map(|(path, summary)| json!({ "path": path, "summary": summary }))
      .collect();
    return context.output.print_json(&json!({ "commands": commands }));
  }

  let width = rows.iter().map(|(path, _)| path.width()).max().unwrap_or(0);
  for (path, summary) in rows {
    let padding = " ".repeat(width - path.width());
    context.output.print_line(&format!("{path}{padding}  {summary}"));
  }

  Ok(())
}

fn list_commands(root: &Command) -> Vec<(String, &Command)> {
  let mut commands = vec![(root.get_name().to_owned(), root)];
  push_commands(root, root.get_name().to_owned(), &mut commands);
  commands
}

fn push_commands<'a>(
  command: &'a Command,
  path: String,
  commands: &mut Vec<(String, &'a Command)>,
) {
  for child in command.get_subcommands().filter(|child| !child.is_hide_set()) {
    let child_path = format!("{path} {}", child.get_name());
    commands.push((child_path.clone(), child));
    push_commands(child, child_path, commands);
  }
}

fn describe(command: &Command, path: &str) -> Value {
  let subcommands: Vec<Value> = command
    .get_subcommands()
    .filter(|child| !child.is_hide_set())
    .map(|child| json!({ "name": child.get_name(), "summary": get_summary(child) }))
    .collect();
  let arguments: Vec<Value> = command
    .get_positionals()
    .filter(|arg| !arg.is_hide_set())
    .map(|arg| {
      json!({
        "name": get_value_name(arg),
        "required": arg.is_required_set(),
        "help": get_help(arg),
      })
    })
    .collect();
  let options: Vec<Value> = command
    .get_arguments()
    .filter(|arg| !arg.is_positional() && !arg.is_hide_set())
    .map(describe_option)
    .collect();

  json!({
    "path": path,
    "summary": get_summary(command),
    "subcommands": subcommands,
    "arguments": arguments,
    "options": options,
    "examples": get_examples(command),
  })
}

fn describe_option(arg: &Arg) -> Value {
  let takes_values = arg.get_action().takes_values();
  let defaults: Vec<String> = arg
    .get_default_values()
    .iter()
    .map(|value| value.to_string_lossy().into_owned())
    .collect();
  let possible_values: Vec<String> = if takes_values {
    arg
      .get_possible_values()
      .iter()
      .filter(|value| !value.is_hide_set())
      .map(|value| value.get_name().to_owned())
      .collect()
  } else {
    Vec::new()
  };

  json!({
    "long": arg.get_long(),
    "short": arg.get_short().map(String::from),
    "value_name": takes_values.then(|| get_value_name(arg)),
    "default": (takes_values && !defaults.is_empty()).then(|| defaults.join(",")),
    "possible_values": possible_values,
    "help": get_help(arg),
  })
}

fn get_summary(command: &Command) -> String {
  command.get_about().map(ToString::to_string).unwrap_or_default()
}

fn get_help(arg: &Arg) -> Option<String> {
  arg.get_help().map(ToString::to_string)
}

fn get_value_name(arg: &Arg) -> String {
  arg
    .get_value_names()
    .and_then(|names| names.first())
    .map_or_else(|| arg.get_id().as_str().to_uppercase(), ToString::to_string)
}

fn get_examples(command: &Command) -> Vec<String> {
  let text = command.get_after_help().map(ToString::to_string).unwrap_or_default();
  text
    .strip_prefix("Examples:")
    .map(|lines| {
      lines
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
  use super::{build_root, get_examples, list_commands, read_invocation};
  use crate::cli::Cli;
  use clap::CommandFactory;

  #[test]
  fn reads_the_command_path_past_global_options() {
    let argv = ["inboxjot", "--profile", "work", "--non-interactive", "domains", "add"]
      .into_iter()
      .chain(["--help", "--json"])
      .map(str::to_owned)
      .collect::<Vec<_>>();
    let (path, global) = read_invocation(&argv);

    assert_eq!(path, ["domains", "add"]);
    assert!(global.json);
    assert_eq!(global.profile, "work");
    assert_eq!(global.non_interactive, Some(true));
  }

  #[test]
  fn gives_every_command_an_example() {
    let root = build_root();
    let missing: Vec<String> = list_commands(&root)
      .into_iter()
      .skip(1)
      .filter(|(_, command)| {
        !command.has_subcommands() && get_examples(command).is_empty()
      })
      .map(|(path, _)| path)
      .collect();

    assert!(missing.is_empty(), "no examples for {missing:?}");
  }

  #[test]
  fn parses_every_example() {
    let root = build_root();
    let mut failures = Vec::new();

    for (_, command) in list_commands(&root) {
      for example in get_examples(command) {
        let line = example.split(" > ").next().unwrap_or(&example);
        let words: Vec<String> = shlex::split(line)
          .unwrap_or_default()
          .into_iter()
          .take_while(|word| word != "<")
          .collect();
        if let Err(error) = Cli::command().try_get_matches_from(&words) {
          failures.push(format!("{example}: {}", error.kind()));
        }
      }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
  }
}
