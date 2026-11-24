use clap::Args;
use jiff::Zoned;
use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};

use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column, format_count};
use crate::ui::bar::render_thin;
use anstyle::Style;

use crate::ui::style::{DIM, SECONDARY, TITLE, TRACK, get_event_style, get_width};
use crate::ui::text::{Line, Span};

const DETAIL_ROWS: [Column; 9] = [
  column("Period", "period"),
  column("Sent", "sent"),
  column("Delivered", "delivered"),
  column("Opened", "opened"),
  column("Clicked", "clicked"),
  column("Bounced", "bounced"),
  column("Soft bounced", "soft_bounced"),
  column("Complained", "complained"),
  column("Unsubscribed", "unsubscribed"),
];

const RATE_BASES: [(&str, &str); 7] = [
  ("delivered", "sent"),
  ("opened", "delivered"),
  ("clicked", "sent"),
  ("bounced", "sent"),
  ("soft_bounced", "sent"),
  ("complained", "sent"),
  ("unsubscribed", "delivered"),
];

#[derive(Debug, Args)]
pub struct StatsArgs {
  #[arg(
    long = "period",
    alias = "since",
    value_name = "PERIOD",
    default_value = "7d",
    value_parser = parse_period,
    help = "Count email from this long ago until today, such as 30d, or from a date such as 2026-11-01"
  )]
  pub period: Period,
}

#[derive(Clone, Debug)]
pub struct Period {
  start: jiff::civil::Date,
  label: String,
}

pub async fn run(args: &StatsArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading stats…");
  let today = Zoned::now().date();
  let tz = TimeZone::system().iana_name().unwrap_or("UTC").to_owned();
  if args.period.start > today {
    return Err(Error::usage("invalid_since", "The start date is in the future."));
  }

  let query =
    json!({ "from": args.period.start.to_string(), "to": today.to_string(), "tz": tz });
  let report = api.get(&["v1", "stats"], &query).await?;

  if context.output.is_json() {
    return context.output.print_json(&report);
  }

  if context.output.is_stdout_tty() {
    let workspace = context.get_workspace();
    for line in
      render_chart(&report, &args.period.label, workspace.as_deref(), get_width())
    {
      anstream::println!("{line}");
    }
    return Ok(());
  }

  context.output.print_detail(&summarize(&report), &DETAIL_ROWS)
}

fn render_chart(
  report: &Value,
  label: &str,
  workspace: Option<&str>,
  width: usize,
) -> Vec<Line<'static>> {
  let total = report.get("total").cloned().unwrap_or_default();
  let count = |key: &str| total.get(key).and_then(Value::as_u64).unwrap_or(0);
  let field = |key: &str| report.get(key).and_then(Value::as_str).unwrap_or_default();
  let sent = count("sent");
  let bar_width = width.saturating_sub(42).clamp(10, 40);

  let mut range = format!("{} to {}", format_day(field("from")), format_day(field("to")));
  if let Some(workspace) = workspace {
    range = format!("{workspace} · {range}");
  }
  let mut lines = vec![
    Line::new(vec![
      Span::styled(format!("  {label}"), TITLE),
      Span::styled(format!("  {range}"), DIM),
    ]),
    Line::blank(),
  ];
  let metrics: Vec<(&str, &str)> = std::iter::once(("sent", "sent"))
    .chain(RATE_BASES)
    .filter(|(key, _)| *key != "soft_bounced")
    .collect();
  let number_width = metrics
    .iter()
    .map(|(key, _)| format_count(count(key)).len())
    .max()
    .unwrap_or(1)
    + 1;
  for (key, base) in metrics {
    let part = count(key);
    let rate = match (key, count(base)) {
      ("sent", _) | (_, 0) => String::new(),
      (_, whole) => format!("  {}", format_rate(part as f64 * 100.0 / whole as f64)),
    };
    let fraction = if sent == 0 { 0.0 } else { part as f64 / sent as f64 };
    let mut spans = vec![
      Span::raw(format!("  {:<14}", to_label(key))),
      Span::styled(format!("{:>number_width$}  ", format_count(part)), TITLE),
    ];
    spans.extend(render_thin(bar_width, fraction, get_event_style(key)));
    spans.push(Span::styled(rate, SECONDARY));
    lines.push(Line::new(spans));
  }

  let days: Vec<(String, u64)> = report
    .get("stats")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .map(|day| {
      (
        day.get("day").and_then(Value::as_str).unwrap_or_default().to_owned(),
        day.get("sent").and_then(Value::as_u64).unwrap_or(0),
      )
    })
    .collect();
  if days.len() > 1 {
    lines.push(Line::blank());
    lines.push(Line::from(Span::styled("  Sent per day", DIM)));
    lines.extend(render_days(&days, width));
  }
  lines
}

fn render_days(days: &[(String, u64)], width: usize) -> Vec<Line<'static>> {
  const LEVELS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
  let room = width.saturating_sub(28).max(10);
  let size = days.len().div_ceil(room).max(1);
  let cells: Vec<u64> = days
    .chunks(size)
    .map(|chunk| chunk.iter().map(|(_, sent)| sent).sum())
    .collect();
  let peak = cells.iter().copied().max().unwrap_or(0);

  let mut spans = vec![Span::raw("  ")];
  for value in &cells {
    let level = if peak == 0 || *value == 0 {
      0
    } else {
      ((*value as f64 / peak as f64) * 7.0).round().max(1.0) as usize
    };
    let style = match level {
      0 => TRACK,
      1..=3 => DIM,
      4 | 5 => SECONDARY,
      _ => Style::new(),
    };
    spans.push(Span::styled(LEVELS[level], style));
  }
  if let Some((day, sent)) =
    days.iter().max_by_key(|(_, sent)| *sent).filter(|_| peak > 0)
  {
    spans.push(Span::styled(
      format!("  peak {} on {}", format_count(*sent), format_day(day)),
      DIM,
    ));
  }

  let first = format_day(&days[0].0);
  let last = format_day(&days[days.len() - 1].0);
  let gap = cells.len().saturating_sub(first.len() + last.len()).max(1);
  vec![
    Line::new(spans),
    Line::from(Span::styled(format!("  {first}{}{last}", " ".repeat(gap)), DIM)),
  ]
}

fn format_day(text: &str) -> String {
  text
    .parse::<jiff::civil::Date>()
    .map_or_else(|_| text.to_owned(), |date| date.strftime("%b %-d").to_string())
}

fn format_rate(percent: f64) -> String {
  if percent > 0.0 && percent < 0.01 {
    return "<0.01%".to_owned();
  }
  if percent > 0.0 && percent < 0.1 {
    format!("{percent:.2}%")
  } else {
    format!("{percent:.1}%")
  }
}

fn to_label(key: &str) -> String {
  let label = key.replace('_', " ");
  let mut characters = label.chars();
  characters
    .next()
    .map(|first| first.to_uppercase().chain(characters).collect())
    .unwrap_or_default()
}

fn parse_period(text: &str) -> std::result::Result<Period, String> {
  if let Ok(start) = text.trim().parse::<jiff::civil::Date>() {
    return Ok(Period {
      start,
      label: format!("Since {}", start.strftime("%b %-d")),
    });
  }

  let invalid = || format!("\"{text}\" is not a duration. Use a value such as 7d or 30d");
  let span: jiff::Span = text.trim().parse().map_err(|_| invalid())?;
  let start = Zoned::now().checked_sub(span).map_err(|_| invalid())?.date();
  let days = (Zoned::now().date() - start).get_days();
  let label = if days == 1 { "Last day".to_owned() } else { format!("Last {days} days") };
  Ok(Period { start, label })
}

fn summarize(report: &Value) -> Value {
  let total = report.get("total").cloned().unwrap_or_default();
  let count = |key: &str| total.get(key).and_then(Value::as_u64).unwrap_or(0);
  let field = |key: &str| report.get(key).and_then(Value::as_str).unwrap_or_default();

  let mut summary = Map::new();
  summary
    .insert("period".to_owned(), json!(format!("{} to {}", field("from"), field("to"))));
  summary.insert("sent".to_owned(), json!(format_count(count("sent"))));
  for (key, base) in RATE_BASES {
    let part = count(key);
    let cell = match count(base) {
      0 => format_count(part),
      whole => {
        format!("{}  {:.1}%", format_count(part), part as f64 * 100.0 / whole as f64)
      }
    };
    summary.insert(key.to_owned(), json!(cell));
  }

  Value::Object(summary)
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{format_rate, summarize};

  #[test]
  fn keeps_small_rates_visible() {
    assert_eq!(format_rate(0.037), "0.04%");
    assert_eq!(format_rate(0.0038), "<0.01%");
    assert_eq!(format_rate(0.0), "0.0%");
    assert_eq!(format_rate(98.74), "98.7%");
  }

  #[test]
  fn adds_rates_to_counts() {
    let report = json!({
      "from": "2026-11-02",
      "to": "2026-11-09",
      "total": { "sent": 200, "delivered": 190, "opened": 95, "clicked": 10, "bounced": 4 }
    });
    let summary = summarize(&report);

    assert_eq!(summary["period"], "2026-11-02 to 2026-11-09");
    assert_eq!(summary["delivered"], "190  95.0%");
    assert_eq!(summary["opened"], "95  50.0%");
    assert_eq!(summary["clicked"], "10  5.0%");
    assert_eq!(summary["complained"], "0  0.0%");
  }
}
