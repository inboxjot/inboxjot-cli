use std::borrow::Cow;
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

use anstyle::{AnsiColor, Style};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Unit};
use serde::Serialize;
use serde_json::{Value, json};
use unicode_width::UnicodeWidthStr;

use crate::error::{Error, ErrorItem, Kind, Next, Result};
use crate::jq;
use crate::ui::bar::Bar;
use crate::ui::header;
use crate::ui::mark::Mark;
use crate::ui::spinner::{self, Shown};
use crate::ui::style::{
  ACTION, DANGER, DIM, SECONDARY, get_event_style, get_width, has_color, hide_cursor,
  show_cursor,
};
use crate::ui::text::{Line, Span};

pub const COMMAND: Style = AnsiColor::Cyan.on_default();
pub const BOLD: Style = Style::new().bold();

const LABEL_WIDTH: usize = 16;
const COLUMN_GAP: usize = 3;
const PROGRESS_STEPS: u64 = 1_000;
const RELATIVE_LIMIT_SECONDS: i64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
  Text,
  Json,
}

#[derive(Clone, Copy, Debug)]
pub struct Column {
  pub label: &'static str,
  pub path: &'static str,
  pub marked: bool,
  pub secondary: bool,
}

impl Column {
  pub const fn secondary(mut self) -> Self {
    self.secondary = true;
    self
  }
}

pub const fn column(label: &'static str, path: &'static str) -> Column {
  Column { label, path, marked: false, secondary: false }
}

pub const fn status_column(label: &'static str, path: &'static str) -> Column {
  Column { label, path, marked: true, secondary: false }
}

#[derive(Debug)]
pub struct Output {
  format: Format,
  jq: Option<String>,
  stdout_tty: bool,
  rich: bool,
  guidance: bool,
  agent: bool,
  argv: Vec<String>,
}

#[derive(Serialize)]
struct AgentOutcome<'a> {
  status: &'static str,
  reason: &'a str,
  message: &'a str,
  next: &'a [Next],
  #[serde(skip_serializing_if = "<[_]>::is_empty")]
  errors: &'a [ErrorItem],
  #[serde(skip_serializing_if = "Option::is_none")]
  request_id: Option<&'a str>,
}

#[derive(Debug)]
pub struct Spinner(Option<(ProgressBar, Shown)>);

#[derive(Debug)]
pub struct Progress {
  bar: Option<ProgressBar>,
  noun: &'static str,
  started: Instant,
  width: usize,
  shown: Option<Shown>,
}

impl Progress {
  pub fn update(&self, done: u64, total: Option<u64>) {
    let Some(bar) = &self.bar else {
      return;
    };
    let fraction = match total {
      Some(total) if total > 0 => (done as f64 / total as f64).clamp(0.0, 1.0),
      _ => 0.0,
    };
    bar.set_position((fraction * PROGRESS_STEPS as f64).round() as u64);
    let text = format_rate(done, total, self.noun, self.started.elapsed());
    bar.set_message(Line::from(Span::styled(text, SECONDARY)).render(has_color()));
  }

  pub fn finish(self) {
    let Some(bar) = self.bar else {
      return;
    };
    bar.finish_and_clear();
    show_cursor();
    anstream::eprintln!("{}", render_meter(self.width, 1.0));
    drop(self.shown);
  }
}

fn render_meter(width: usize, fraction: f64) -> Line<'static> {
  let color = has_color();
  let mut spans = vec![Span::raw("  ")];
  spans.extend(Bar { width, color }.render(fraction).spans);
  spans.push(Span::styled(format!("  {:.0}%", fraction * 100.0), BOLD));
  Line::new(spans)
}

fn format_rate(done: u64, total: Option<u64>, noun: &str, elapsed: Duration) -> String {
  let mut parts = vec![match total {
    Some(total) => format!("{} of {}", format_count(done), format_count(total)),
    None => format!("{} {noun}", format_count(done)),
  }];
  let seconds = elapsed.as_secs_f64();
  if done > 0 && seconds >= 1.0 {
    let rate = done as f64 / seconds;
    parts.push(format!("{} a second", format_count(rate.round() as u64)));
    if let Some(left) =
      total.map(|total| total.saturating_sub(done)).filter(|left| *left > 0)
    {
      parts.push(format!("about {} left", format_wait(left as f64 / rate)));
    }
  }
  parts.join(" · ")
}

fn format_wait(seconds: f64) -> String {
  let seconds = seconds.ceil() as u64;
  if seconds < 60 {
    return format!("{seconds}s");
  }
  format!("{}m", seconds.div_ceil(60))
}

impl Spinner {
  pub fn set_message(&self, message: impl Into<String>) {
    if let Some((bar, _)) = &self.0 {
      bar.set_message(clean(&message.into()).into_owned());
    }
  }

  pub fn finish(self) {
    if let Some((bar, _)) = self.0 {
      bar.finish_and_clear();
      show_cursor();
    }
  }
}

impl Output {
  pub fn new(format: Format, jq: Option<String>, agent: bool, argv: Vec<String>) -> Self {
    let is_dumb = std::env::var("TERM").is_ok_and(|term| term == "dumb");
    let guidance = std::env::var("INBOXJOT_GUIDANCE").map_or(true, |value| value != "0");

    let rich = std::io::stderr().is_terminal() && !is_dumb;
    Self {
      format,
      jq,
      stdout_tty: std::io::stdout().is_terminal(),
      rich,
      guidance,
      agent,
      argv,
    }
  }

  pub fn is_json(&self) -> bool {
    self.format == Format::Json
  }

  pub fn has_structured_errors(&self) -> bool {
    self.agent || self.is_json()
  }

  pub fn is_rich(&self) -> bool {
    self.rich && !self.is_json()
  }

  pub fn shows_loading(&self) -> bool {
    self.is_rich() && !self.agent
  }

  pub fn begin(&self) {
    if self.is_rich() {
      anstream::eprintln!();
    }
  }

  pub fn print_header(&self) {
    if !self.is_rich() {
      return;
    }

    for line in header::render(env!("CARGO_PKG_VERSION"), get_width()) {
      anstream::eprintln!("{line}");
    }
    anstream::eprintln!();
  }

  pub fn argv(&self) -> &[String] {
    &self.argv
  }

  pub fn is_stdout_tty(&self) -> bool {
    self.stdout_tty
  }

  pub fn print_json(&self, value: &Value) -> Result<()> {
    if let Some(filter) = &self.jq {
      for line in jq::run_filter(filter, value)? {
        if self.stdout_tty {
          write_stdout(&clean(&line))?;
        } else {
          write_stdout(&line)?;
        }
      }
      return Ok(());
    }

    self.write_json(&serde_json::to_string_pretty(value)?)
  }

  pub fn print_json_line(&self, value: &Value) -> Result<()> {
    self.write_json(&serde_json::to_string(value)?)
  }

  fn write_json(&self, text: &str) -> Result<()> {
    if self.stdout_tty {
      write_stdout(&escape_json_controls(text))
    } else {
      write_stdout(text)
    }
  }

  pub fn print_line(&self, text: &str) {
    anstream::println!("{}", clean(text));
  }

  pub fn print_list(
    &self,
    page: &Value,
    columns: &[Column],
    noun: &str,
    empty_next: Option<(&str, &str)>,
  ) -> Result<()> {
    if self.is_json() {
      return self.print_json(page);
    }

    let rows = page
      .get("data")
      .and_then(Value::as_array)
      .map_or(&[][..], Vec::as_slice);
    if rows.is_empty() {
      if !self.stdout_tty {
        self.print_table(rows, columns);
      }
      if self.rich {
        anstream::eprintln!(
          "{}",
          Line::new(vec![Span::styled(format!("  No {noun} yet."), SECONDARY)])
        );
        if let Some((what, command)) = empty_next {
          self.print_next(&[(what, command)]);
        }
      } else {
        anstream::eprintln!("No {noun}");
      }
      return Ok(());
    }

    self.print_table(rows, columns);

    let total = page.get("total").and_then(Value::as_u64);
    let cursor = page.get("next_cursor").and_then(Value::as_str);
    if !self.rich {
      if let Some(total) = total.filter(|total| *total > rows.len() as u64) {
        anstream::eprintln!("  Showing {} of {} {noun}", rows.len(), format_count(total));
      }
      if let Some(cursor) = cursor {
        self.next("Next page:", &self.get_command_with(&["--after", cursor]));
      }
      return Ok(());
    }

    let mut spans = Vec::new();
    if let Some(total) = total.filter(|total| *total > rows.len() as u64) {
      let shown = format_count(rows.len() as u64);
      spans.push(Span::styled(format!("  {shown} of {}", format_count(total)), DIM));
    }
    if let Some(cursor) = cursor.filter(|_| self.guidance) {
      spans.push(Span::raw(if spans.is_empty() { "  " } else { "   " }));
      spans.push(Span::styled(self.get_command_with(&["--after", cursor]), ACTION));
      spans.push(Span::styled("   next page", DIM));
    }
    if !spans.is_empty() {
      anstream::eprintln!();
      anstream::eprintln!("{}", Line::new(spans));
    }

    Ok(())
  }

  pub fn print_detail(&self, value: &Value, rows: &[Column]) -> Result<()> {
    if self.is_json() {
      return self.print_json(value);
    }

    for row in rows {
      let cell = format_value(get_path(value, row.path), row.path, self.stdout_tty, true);
      anstream::println!("{:<LABEL_WIDTH$}{cell}", row.label);
    }

    Ok(())
  }

  pub fn print_result(
    &self,
    value: &Value,
    headline: &str,
    id_path: &str,
    rows: &[Column],
  ) -> Result<()> {
    self.print_outcome(value, headline, id_path, rows, Mark::Done)
  }

  pub fn print_removal(
    &self,
    value: &Value,
    headline: &str,
    id_path: &str,
    rows: &[Column],
  ) -> Result<()> {
    self.print_outcome(value, headline, id_path, rows, Mark::Removed)
  }

  fn print_outcome(
    &self,
    value: &Value,
    headline: &str,
    id_path: &str,
    rows: &[Column],
    mark: Mark,
  ) -> Result<()> {
    if self.is_json() {
      return self.print_json(value);
    }

    if mark == Mark::Removed {
      self.removed(headline);
    } else {
      self.success(headline);
    }
    for row in rows {
      let cell = format_value(get_path(value, row.path), row.path, true, true);
      self.info(&format!("{:<LABEL_WIDTH$}{cell}", row.label));
    }

    if !self.stdout_tty
      && let Some(id) = get_path(value, id_path).and_then(Value::as_str)
    {
      anstream::println!("{id}");
    }

    Ok(())
  }

  pub fn print_table(&self, rows: &[Value], columns: &[Column]) {
    let cells: Vec<Vec<String>> = rows
      .iter()
      .map(|row| {
        columns
          .iter()
          .map(|column| {
            let cell = format_value(
              get_path(row, column.path),
              column.path,
              self.stdout_tty,
              false,
            );
            if column.marked && self.stdout_tty { cell.replace('_', " ") } else { cell }
          })
          .collect()
      })
      .collect();

    if !self.stdout_tty {
      let header: Vec<&str> = columns.iter().map(|column| column.label).collect();
      anstream::println!("{}", header.join("\t"));
      for row in cells {
        anstream::println!("{}", row.join("\t"));
      }
      return;
    }

    let is_empty = |cell: &str| cell == "-";
    let mark_width = |column: &Column| if column.marked { 2 } else { 0 };
    let widths: Vec<usize> = columns
      .iter()
      .enumerate()
      .map(|(index, column)| {
        let cell_width = cells.iter().map(|row| row[index].width()).max().unwrap_or(0);
        (cell_width + mark_width(column)).max(column.label.width())
      })
      .collect();

    let room = get_width().saturating_sub(1).max(20);
    let header: Vec<&str> = columns.iter().map(|column| column.label).collect();
    anstream::println!(
      "{}",
      Line::new(vec![Span::styled(
        format!("  {}", get_padded_row(&header, &widths)),
        DIM
      )])
      .truncate(room)
    );
    for row in &cells {
      let mut spans = vec![Span::raw("  ")];
      for (index, (cell, column)) in row.iter().zip(columns).enumerate() {
        let is_last = index + 1 == columns.len();
        let mut used = cell.width();
        if column.marked && !is_empty(cell) {
          spans.push(Mark::Status(get_event_style(&cell.replace(' ', "_"))).span());
          spans.push(Span::raw(" "));
          used += 2;
        }
        let style = if is_empty(cell) || column.path.ends_with("_at") {
          DIM
        } else if column.secondary || (index == 0 && column.path.ends_with("id")) {
          SECONDARY
        } else {
          Style::new()
        };
        spans.push(Span::styled(cell.clone(), style));
        if !is_last {
          spans
            .push(Span::raw(" ".repeat(widths[index].saturating_sub(used) + COLUMN_GAP)));
        }
      }
      anstream::println!("{}", Line::new(spans).truncate(room));
    }
  }

  pub fn success(&self, headline: &str) {
    if self.is_json() {
      return;
    }

    if self.rich {
      self.eprint_marked(Mark::Done, headline);
    } else {
      anstream::eprintln!("{}", clean(headline));
    }
  }

  pub fn done(&self, headline: &str, detail: &str) {
    if self.is_json() {
      return;
    }

    if !self.rich {
      anstream::eprintln!("{}", clean(headline));
      return;
    }
    let mut spans =
      vec![Span::raw("  "), Mark::Done.span(), Span::raw(" "), Span::raw(headline)];
    if !detail.is_empty() {
      spans.push(Span::styled(format!("  {detail}"), DIM));
    }
    anstream::eprintln!("{}", Line::new(spans));
  }

  pub fn done_with(&self, parts: &[(&str, bool)], detail: &str) {
    let headline: String = parts.iter().map(|(text, _)| *text).collect();
    if self.is_json() {
      return;
    }
    if !self.rich {
      anstream::eprintln!("{}", clean(&headline));
      return;
    }
    let mut spans = vec![Span::raw("  "), Mark::Done.span(), Span::raw(" ")];
    for (text, bold) in parts {
      let style = if *bold { BOLD } else { Style::new() };
      spans.push(Span::styled((*text).to_owned(), style));
    }
    if !detail.is_empty() {
      spans.push(Span::styled(format!("  {detail}"), SECONDARY));
    }
    anstream::eprintln!("{}", Line::new(spans));
  }

  pub fn removed(&self, headline: &str) {
    if self.is_json() {
      return;
    }

    if self.rich {
      self.eprint_marked(Mark::Removed, headline);
    } else {
      anstream::eprintln!("{}", clean(headline));
    }
  }

  fn eprint_marked(&self, mark: Mark, message: &str) {
    let line =
      Line::new(vec![Span::raw("  "), mark.span(), Span::raw(" "), Span::raw(message)]);
    anstream::eprintln!("{line}");
  }

  pub fn info(&self, message: &str) {
    if self.is_json() {
      return;
    }

    if self.rich {
      anstream::eprintln!(
        "{}",
        Line::new(vec![Span::raw("    "), Span::styled(message, SECONDARY)])
      );
    } else {
      anstream::eprintln!("  {}", clean(message));
    }
  }

  pub fn warn(&self, message: &str) {
    if self.rich {
      self.eprint_marked(Mark::Warning, message);
    } else {
      anstream::eprintln!("Warning: {}", clean(message));
    }
  }

  pub fn next(&self, label: &str, command: &str) {
    if self.is_json() || !self.guidance {
      return;
    }

    if self.rich {
      let what = label.trim_end_matches(':');
      let line = Line::new(vec![
        Span::styled("  next  ", DIM),
        Span::styled(command, ACTION),
        Span::styled(
          if what.is_empty() {
            String::new()
          } else {
            format!("   {}", lowercase_first(what))
          },
          DIM,
        ),
      ]);
      anstream::eprintln!("{line}");
      return;
    }

    anstream::eprintln!("  {label}");
    anstream::eprintln!("    {COMMAND}{command}{COMMAND:#}");
  }

  pub fn print_next(&self, steps: &[(&str, &str)]) {
    if !self.is_rich() || !self.guidance || steps.is_empty() {
      return;
    }

    anstream::eprintln!();
    let width = steps.iter().map(|(_, command)| command.width()).max().unwrap_or(0);
    for (index, (what, command)) in steps.iter().enumerate() {
      let label = if index == 0 { "  next  " } else { "        " };
      let mut spans =
        vec![Span::styled(label, DIM), Span::styled(command.to_string(), ACTION)];
      if !what.is_empty() {
        let padding = " ".repeat(width - command.width() + 3);
        spans.push(Span::styled(format!("{padding}{}", lowercase_first(what)), DIM));
      }
      anstream::eprintln!("{}", Line::new(spans));
    }
  }

  pub fn print_error(&self, error: &Error) {
    if self.agent {
      let outcome = AgentOutcome {
        status: if error.is_action_required() { "action_required" } else { "error" },
        reason: &error.name,
        message: &error.message,
        next: error.next.as_slice(),
        errors: &error.details,
        request_id: error.request_id.as_deref(),
      };
      anstream::println!("{}", serde_json::to_string(&outcome).unwrap_or_default());
      return;
    }

    if self.is_json() {
      let body = json!({
        "error": {
          "name": error.name,
          "message": error.message,
          "errors": error.details,
          "request_id": error.request_id,
          "next": error.next.first().map(|next| &next.command),
        }
      });
      anstream::eprintln!("{body}");
      return;
    }

    if self.rich {
      if error.kind != Kind::Quiet {
        self.eprint_rich_error(error);
      }
      return;
    }

    let message = clean(&error.message);
    anstream::eprintln!("Error: {message}");
    if let Some(reason) = &error.reason {
      anstream::eprintln!("  {}", clean(reason));
    }

    for item in &error.details {
      anstream::eprintln!(
        "  {:<LABEL_WIDTH$}{}",
        clean(&item.location),
        clean(&item.message)
      );
    }

    if let Some(request_id) = &error.request_id {
      anstream::eprintln!("  {:<LABEL_WIDTH$}{request_id}", "Request ID");
    }

    if !error.next.is_empty() {
      anstream::eprintln!("  Run:");
      for next in &error.next {
        anstream::eprintln!("    {COMMAND}{}{COMMAND:#}", next.command);
      }
    }
  }

  fn eprint_rich_error(&self, error: &Error) {
    if error.kind == Kind::Canceled {
      anstream::eprintln!(
        "{}",
        Line::new(vec![Span::styled("  Cancelled. Nothing was changed.", SECONDARY)])
      );
      return;
    }

    let title = Style::new().bold().fg_color(DANGER.get_fg_color());
    let indented = |text: String, style: Style| {
      Line::new(vec![Span::raw("    "), Span::styled(text, style)])
    };
    let mut lines = vec![Line::new(vec![
      Span::raw("  "),
      Mark::Error.span(),
      Span::raw(" "),
      Span::styled(error.message.clone(), title),
    ])];
    if let Some(reason) = &error.reason {
      lines.push(indented(reason.clone(), SECONDARY));
    }
    for item in error.details.iter().filter(|item| item.message != error.message) {
      let text = if item.location.is_empty() {
        item.message.clone()
      } else {
        format!("{}: {}", item.location, item.message)
      };
      lines.push(indented(text, SECONDARY));
    }
    lines.push(indented(error.name.to_string(), DIM));

    if !error.next.is_empty() {
      lines.push(Line::blank());
      let width = error.next.iter().map(|next| next.command.width()).max().unwrap_or(0);
      for next in &error.next {
        lines.push(Line::new(vec![
          Span::raw("    "),
          Span::styled(format!("{:<width$}", next.command), ACTION),
          Span::styled(format!("   {}", lowercase_first(next.description)), DIM),
        ]));
      }
    }

    for line in lines {
      anstream::eprintln!("{line}");
    }
  }

  pub fn start_spinner(&self, message: &str, below: &[Line<'static>]) -> Spinner {
    if self.is_json() {
      return Spinner(None);
    }

    if !self.rich {
      anstream::eprintln!("  {}", clean(message));
      return Spinner(None);
    }

    let shown = Shown::new();
    let bar = ProgressBar::new_spinner()
      .with_style(get_spinner_style(below))
      .with_message(clean(message).into_owned());
    hide_cursor();
    bar.enable_steady_tick(spinner::TICK);
    Spinner(Some((bar, shown)))
  }

  pub fn start_progress(&self, noun: &'static str, below: &[Line<'static>]) -> Progress {
    let width = get_width().saturating_sub(10).clamp(10, 64);
    let started = Instant::now();
    if !self.is_rich() {
      return Progress { bar: None, noun, started, width, shown: None };
    }

    let color = has_color();
    let mut template = "{meter}\n\n  {msg}".to_owned();
    if !below.is_empty() {
      template.push('\n');
      for line in below {
        template.push('\n');
        template.push_str(&line.render(color).replace('{', "{{").replace('}', "}}"));
      }
    }
    let style = ProgressStyle::with_template(&template)
      .unwrap_or_else(|_| ProgressStyle::default_bar())
      .with_key("meter", move |state: &ProgressState, out: &mut dyn std::fmt::Write| {
        let line = render_meter(width, f64::from(state.fraction()));
        let _ = out.write_str(&line.render(color));
      });
    let shown = Shown::new();
    let bar = ProgressBar::new(PROGRESS_STEPS).with_style(style);
    bar.set_message(Line::from(Span::styled("Starting…", SECONDARY)).render(color));
    hide_cursor();
    bar.enable_steady_tick(Duration::from_millis(100));
    Progress {
      bar: Some(bar),
      noun,
      started,
      width,
      shown: Some(shown),
    }
  }

  pub fn get_command_with(&self, extra: &[&str]) -> String {
    let flag = extra.first().copied().filter(|flag| flag.starts_with("--"));
    let takes_value = extra.len() > 1;
    let mut words = vec!["inboxjot"];
    let mut args = self.argv.iter().skip(1);

    while let Some(word) = args.next() {
      let Some(flag) = flag else {
        words.push(word);
        continue;
      };

      if word == flag {
        if takes_value {
          args.next();
        }
        continue;
      }

      if word.strip_prefix(flag).is_some_and(|rest| rest.starts_with('=')) {
        continue;
      }

      words.push(word);
    }

    words.extend(extra);
    shlex::try_join(words).unwrap_or_default()
  }
}

pub fn get_spinner_style(below: &[Line<'static>]) -> ProgressStyle {
  let color = has_color();
  let frames = spinner::get_frames(color);
  let frames: Vec<&str> = frames.iter().map(String::as_str).collect();
  let mut template = "  {spinner} {msg}".to_owned();
  if !below.is_empty() {
    template.push('\n');
    for line in below {
      template.push('\n');
      template.push_str(&line.render(color).replace('{', "{{").replace('}', "}}"));
    }
  }
  ProgressStyle::with_template(&template)
    .unwrap_or_else(|_| ProgressStyle::default_spinner())
    .tick_strings(&frames)
}

fn lowercase_first(text: &str) -> String {
  let mut characters = text.chars();
  characters
    .next()
    .map_or_else(String::new, |first| first.to_lowercase().chain(characters).collect())
}

pub fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
  path.split('.').try_fold(value, |value, key| value.get(key))
}

pub fn format_value(
  value: Option<&Value>,
  path: &str,
  tty: bool,
  detailed: bool,
) -> String {
  let empty = if tty { "-" } else { "" };

  match value {
    None | Some(Value::Null) => empty.to_owned(),
    Some(Value::String(text)) if text.is_empty() => empty.to_owned(),
    Some(Value::String(text)) if path.ends_with("_at") => {
      format_time(text, tty, detailed).unwrap_or_else(|| clean(text).into_owned())
    }
    Some(Value::String(text)) => clean(text).into_owned(),
    Some(Value::Bool(flag)) => if *flag { "yes" } else { "no" }.to_owned(),
    Some(Value::Number(number)) => match number.as_u64() {
      Some(count) if tty => format_count(count),
      _ => number.to_string(),
    },
    Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
      let words: Vec<Cow<'_, str>> =
        items.iter().filter_map(Value::as_str).map(clean).collect();
      if words.is_empty() { empty.to_owned() } else { words.join(", ") }
    }
    Some(other) => other.to_string(),
  }
}

fn write_stdout(text: &str) -> Result<()> {
  let mut stdout = std::io::stdout().lock();
  match writeln!(stdout, "{text}") {
    Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => std::process::exit(0),
    result => Ok(result?),
  }
}

fn escape_json_controls(text: &str) -> Cow<'_, str> {
  let is_unsafe = |character: char| matches!(character, '\u{7f}'..='\u{9f}');
  if !text.chars().any(is_unsafe) {
    return Cow::Borrowed(text);
  }

  text
    .chars()
    .map(|character| {
      if is_unsafe(character) {
        format!("\\u{:04x}", u32::from(character))
      } else {
        character.to_string()
      }
    })
    .collect()
}

pub fn clean(text: &str) -> Cow<'_, str> {
  let is_unsafe =
    |character: char| character.is_control() && !matches!(character, '\n' | '\t');
  if !text.chars().any(is_unsafe) {
    return Cow::Borrowed(text);
  }

  text
    .chars()
    .map(|character| {
      if is_unsafe(character) {
        character.escape_unicode().to_string()
      } else {
        character.to_string()
      }
    })
    .collect()
}

pub fn format_size(bytes: u64) -> String {
  let size = bytes as f64;
  match bytes {
    0..1_000 => format!("{bytes} B"),
    1_000..1_000_000 => format!("{:.0} KB", size / 1e3),
    1_000_000..1_000_000_000 => format!("{:.1} MB", size / 1e6),
    _ => format!("{:.1} GB", size / 1e9),
  }
}

pub fn format_count(count: u64) -> String {
  let digits = count.to_string();
  let mut formatted = String::new();

  for (index, digit) in digits.chars().enumerate() {
    if index > 0 && (digits.len() - index).is_multiple_of(3) {
      formatted.push(',');
    }
    formatted.push(digit);
  }

  formatted
}

pub fn format_time(text: &str, tty: bool, detailed: bool) -> Option<String> {
  let timestamp: Timestamp = text.parse().ok()?;
  if !tty {
    return Some(timestamp.to_string());
  }

  let seconds = Timestamp::now().as_second() - timestamp.as_second();
  let relative = format_relative(seconds);
  let zoned = timestamp.to_zoned(TimeZone::system());

  if detailed {
    return Some(format!("{} ({relative})", zoned.strftime("%Y-%m-%d %H:%M %Z")));
  }

  if (-3_600..RELATIVE_LIMIT_SECONDS).contains(&seconds) {
    return Some(relative);
  }

  if seconds < 0 {
    let today = Timestamp::now().to_zoned(TimeZone::system()).date();
    let day = match (zoned.date() - today).get_days() {
      0 => "today".to_owned(),
      1 => "tomorrow".to_owned(),
      _ => zoned.strftime("%b %-d,").to_string(),
    };
    return Some(format!("{day} {}", zoned.strftime("%H:%M")));
  }

  Some(zoned.strftime("%b %-d, %Y").to_string())
}

fn format_relative(seconds: i64) -> String {
  let units = [(Unit::Day, 86_400), (Unit::Hour, 3_600), (Unit::Minute, 60)];
  let size = seconds.abs();
  let (amount, suffix) = units
    .iter()
    .find(|(_, length)| size >= *length)
    .map(|(unit, length)| {
      let suffix = match unit {
        Unit::Day => "d",
        Unit::Hour => "h",
        _ => "m",
      };
      (size / length, suffix)
    })
    .unwrap_or((size, "s"));

  if (0..60).contains(&seconds) {
    return "just now".to_owned();
  }
  if seconds < 0 {
    format!("in {amount}{suffix}")
  } else {
    format!("{amount}{suffix} ago")
  }
}

fn get_padded_row(cells: &[&str], widths: &[usize]) -> String {
  let mut line = String::new();

  for (index, cell) in cells.iter().enumerate() {
    let is_last = index + 1 == cells.len();
    line.push_str(cell);
    if !is_last {
      line.push_str(&" ".repeat(widths[index] - cell.width() + COLUMN_GAP));
    }
  }

  line
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{
    clean, escape_json_controls, format_count, format_relative, format_value, get_path,
  };

  #[test]
  fn escapes_terminal_control_characters() {
    assert_eq!(clean("Riti\u{1b}]52;c;eA==\u{7}"), "Riti\\u{1b}]52;c;eA==\\u{7}");
    assert_eq!(clean("two\nlines\tand tab"), "two\nlines\tand tab");
    assert_eq!(
      format_value(Some(&json!("x\u{1b}[2K")), "first_name", false, false),
      "x\\u{1b}[2K"
    );
  }

  #[test]
  fn escapes_control_characters_inside_lists() {
    let names = json!(["Beta\u{1b}]0;owned\u{7}", "VIP"]);
    let cell = format_value(Some(&names), "segment_names", true, true);
    assert!(!cell.contains('\u{1b}'));
    assert!(!cell.contains('\u{7}'));
    assert!(cell.ends_with(", VIP"));
  }

  #[test]
  fn keeps_json_valid_while_hiding_c1_controls() {
    let value = json!({ "subject": "Hi\u{7f}\u{9b}2J" });
    let text = serde_json::to_string(&value).unwrap();
    let escaped = escape_json_controls(&text);

    assert!(!escaped.contains('\u{7f}') && !escaped.contains('\u{9b}'));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&escaped).unwrap(), value);
  }

  #[test]
  fn formats_counts_with_commas() {
    assert_eq!(format_count(7), "7");
    assert_eq!(format_count(1_000), "1,000");
    assert_eq!(format_count(12_408_000), "12,408,000");
  }

  #[test]
  fn formats_relative_ages() {
    assert_eq!(format_relative(30), "just now");
    assert_eq!(format_relative(90), "1m ago");
    assert_eq!(format_relative(7_200), "2h ago");
    assert_eq!(format_relative(-300), "in 5m");
  }

  #[test]
  fn reads_nested_paths_and_formats_cells() {
    let value =
      json!({ "domain": { "name": "acme.com" }, "tags": ["a", "b"], "n": null });

    assert_eq!(get_path(&value, "domain.name"), Some(&json!("acme.com")));
    assert_eq!(format_value(get_path(&value, "tags"), "tags", true, false), "a, b");
    assert_eq!(format_value(get_path(&value, "n"), "n", true, false), "-");
    assert_eq!(format_value(get_path(&value, "n"), "n", false, false), "");
  }
}
