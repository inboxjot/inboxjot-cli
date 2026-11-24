use std::time::Duration;

use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{Args, ValueEnum};
use serde_json::Value;

use crate::commands::stream::{
  EVENT_TYPES, Event, EventStream, EventType, Item, run_until_stopped,
};
use crate::context::Context;
use crate::error::Result;
use crate::output::get_path;
use crate::time::parse_duration;
use crate::ui::feed::Feed;
use crate::ui::mark::Mark;
use crate::ui::style::{DIM, FAINT, SECONDARY, TITLE, get_event_style};
use crate::ui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

const EVENT_WIDTH: usize = 18;

#[derive(Debug, Args)]
pub struct TailArgs {
  #[arg(
    long,
    value_delimiter = ',',
    value_parser = PossibleValuesParser::new(
      EVENT_TYPES.iter().filter(|kind| kind.is_email()).filter_map(ValueEnum::to_possible_value)
    ).try_map(|name| EventType::from_str(&name, false)),
    hide_possible_values = true,
    help = "Show only these email event types, separated by commas"
  )]
  pub events: Vec<EventType>,

  #[arg(long, value_parser = parse_duration, help = "Stop after this long")]
  pub timeout: Option<Duration>,
}

pub async fn tail(args: TailArgs, context: &Context) -> Result<()> {
  let mut stream = EventStream::connect(context).await?;
  if context.output.is_rich() {
    let source = context.get_workspace().unwrap_or_else(|| "this workspace".to_owned());
    anstream::eprintln!(
      "{}",
      Line::from(Span::styled(format!("  Showing email events in {source}"), TITLE))
    );
    anstream::eprintln!(
      "{}",
      Line::new(vec![Span::styled("  ctrl+c", DIM), Span::styled(" stop", FAINT)])
    );
    anstream::eprintln!();
  }

  let mut feed = Feed::new(context.output.is_rich(), true, "Waiting for events");
  let work = async {
    loop {
      let event = match stream.next().await? {
        Item::Event(event) => event,
        Item::Lost { retry_in } => {
          let note = format!("Lost connection. Retrying in {}s…", retry_in.as_secs());
          feed.push(vec![render_note(Mark::Warning, &note)], false);
          continue;
        }
        Item::Reconnected => {
          feed.push(vec![render_note(Mark::Done, "Reconnected")], false);
          continue;
        }
      };
      let kind = event.get_kind();
      let is_shown = if args.events.is_empty() {
        kind.is_email()
      } else {
        args.events.contains(&kind)
      };
      if !is_shown {
        continue;
      }
      if context.output.is_rich() {
        feed.push(vec![render_event(&event)], true);
      } else {
        print_event(&event, context);
      }
    }
  };
  let result = run_until_stopped(work, args.timeout).await;
  feed.finish();
  result
}

fn render_note(mark: Mark, text: &str) -> Line<'static> {
  let clock = jiff::Zoned::now().strftime("%H:%M:%S").to_string();
  Line::new(vec![
    Span::styled(format!("  {clock}  "), DIM),
    mark.span(),
    Span::raw(format!(" {text}")),
  ])
}

fn render_event(event: &Event) -> Line<'static> {
  let kind = event.get_type().to_owned();
  let width = EVENT_WIDTH.max(kind.width() + 2);
  Line::new(vec![
    Span::styled(format!("  {}  ", event.format_clock()), DIM),
    Mark::Status(get_event_style(&kind)).span(),
    Span::raw(" "),
    Span::raw(format!("{kind:<width$}")),
    Span::styled(format!("{}  ", event.get_subject_id()), SECONDARY),
    Span::raw(get_recipients(event)),
  ])
}

fn get_recipients(event: &Event) -> String {
  get_path(&event.payload, "data.to")
    .and_then(Value::as_array)
    .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
    .unwrap_or_default()
}

fn print_event(event: &Event, context: &Context) {
  if context.output.is_json() {
    let _ = context.output.print_json_line(&event.payload);
    return;
  }

  let to = get_recipients(event);
  let subject = event.get_text("data.subject");
  let line = if context.output.is_stdout_tty() {
    format!(
      "{}  {:<22}  {}  {to}  {subject}",
      event.format_clock(),
      event.get_type(),
      event.get_subject_id()
    )
  } else {
    [event.get_text("created_at"), event.get_type(), event.get_subject_id(), &to, subject]
      .join("\t")
  };
  context.output.print_line(&line);
}
