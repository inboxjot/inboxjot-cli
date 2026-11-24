use anstyle::Style;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;

use crate::context::Context;
use crate::output::get_path;
use crate::ui::mark::Mark;
use crate::ui::style::{DIM, SECONDARY, TITLE, get_event_style};
use crate::ui::text::{Line, Span};

const FIELD_WIDTH: usize = 10;
const SENT_FIELD_WIDTH: usize = 6;
const EVENT_WIDTH: usize = 11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
  Offsets,
  Notes,
}

pub async fn fetch_events(context: &Context, id: &str) -> Vec<Value> {
  let Ok(api) = context.api().await else {
    return Vec::new();
  };
  api
    .get(&["v1", "emails", id, "events"], &())
    .await
    .ok()
    .and_then(|page| page.get("data").and_then(Value::as_array).cloned())
    .unwrap_or_default()
}

pub fn render_sent(email: &Value) -> Vec<Line<'static>> {
  let mut lines = vec![Line::new(vec![
    Span::styled(format!("  {}", get_subject(email)), TITLE),
    Span::styled(format!("  {}", get_text(email, "id")), DIM),
  ])];
  lines.extend(render_fields(&get_addresses(email), SENT_FIELD_WIDTH));
  lines
}

pub fn render_summary(email: &Value) -> Vec<Line<'static>> {
  let status = get_text(email, "status");
  let mut lines = vec![
    Line::new(vec![
      Span::styled(format!("  {}", get_subject(email)), TITLE),
      Span::raw("  "),
      Mark::Status(get_event_style(status)).span(),
      Span::raw(format!(" {status}")),
    ]),
    Line::new(vec![Span::styled(format!("  {}", get_text(email, "id")), DIM)]),
    Line::blank(),
  ];

  let mut fields = get_addresses(email);
  let domain = get_domain(get_text(email, "from"));
  if !domain.is_empty() {
    fields.push(("domain", domain.to_owned()));
  }
  let tags = format_tags(get_path(email, "tags"));
  if !tags.is_empty() {
    fields.push(("tags", tags));
  }
  for (label, path) in [("scheduled", "scheduled_at"), ("created", "created_at")] {
    if let Some(time) = format_clock(get_text(email, path)) {
      fields.push((label, time));
    }
  }
  lines.extend(render_fields(&fields, FIELD_WIDTH));
  lines
}

pub fn render_events(
  email: &Value,
  events: &[Value],
  detail: Detail,
) -> Vec<Line<'static>> {
  let mut events: Vec<&Value> = events.iter().collect();
  events.sort_by_key(|event| read_time(event).map(|time| time.as_millisecond()));
  let first = events.first().and_then(|event| read_time(event));
  let last = events.len().saturating_sub(1);

  events
    .into_iter()
    .enumerate()
    .map(|(index, event)| {
      let kind = event.get("type").and_then(Value::as_str).unwrap_or_default();
      let label = kind.rsplit('.').next().unwrap_or(kind).replace('_', " ");
      let time = read_time(event);
      let clock = time.map_or_else(String::new, |time| {
        time.to_zoned(TimeZone::system()).strftime("%H:%M:%S").to_string()
      });
      let note = match detail {
        Detail::Notes => describe_event(email, event),
        Detail::Offsets => match (first, time) {
          (Some(first), Some(time)) if index > 0 => {
            let millis = time.as_millisecond() - first.as_millisecond();
            format!("+{}", format_duration(millis.max(0) as f64 / 1000.0))
          }
          _ => String::new(),
        },
      };
      let label_style =
        if detail == Detail::Offsets && index == last { TITLE } else { Style::new() };
      let mut spans = vec![
        Span::styled(format!("  {clock:<8}  "), DIM),
        Mark::Status(get_event_style(kind)).span(),
        Span::raw(" "),
        Span::styled(format!("{label:<EVENT_WIDTH$}"), label_style),
      ];
      if !note.is_empty() {
        spans.push(Span::styled(note, SECONDARY));
      }
      Line::new(spans)
    })
    .collect()
}

pub fn get_seconds_to_settle(email: &Value, events: &[Value]) -> Option<f64> {
  let created: Timestamp = email.get("created_at")?.as_str()?.parse().ok()?;
  let status = email.get("status")?.as_str()?;
  let settled = events
    .iter()
    .filter(|event| {
      let kind = event.get("type").and_then(Value::as_str).unwrap_or_default();
      kind.rsplit('.').next() == Some(status)
    })
    .filter_map(read_time)
    .min()?;
  let millis = settled.as_millisecond() - created.as_millisecond();
  (millis >= 0).then(|| millis as f64 / 1000.0)
}

pub fn format_duration(seconds: f64) -> String {
  if seconds < 1.0 {
    return format!("{:.0}ms", seconds * 1000.0);
  }
  if seconds < 60.0 {
    return format!("{seconds:.1}s");
  }
  let whole = seconds.round() as u64;
  format!("{}m {}s", whole / 60, whole % 60)
}

pub fn get_first_recipient(email: &Value) -> &str {
  email
    .get("to")
    .and_then(Value::as_array)
    .and_then(|to| to.first())
    .and_then(Value::as_str)
    .unwrap_or_default()
}

pub fn describe_failure(email: &Value, events: &[Value]) -> String {
  let status = get_text(email, "status");
  let event = events.iter().rev().find(|event| {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or_default();
    kind.rsplit('.').next() == Some(status)
  });
  let field = |key: &str| {
    event
      .and_then(|event| event.get(key))
      .and_then(Value::as_str)
      .unwrap_or_default()
  };
  if !field("diagnostic_code").is_empty() {
    return field("diagnostic_code").to_owned();
  }
  if !field("error_message").is_empty() {
    return field("error_message").to_owned();
  }
  match (status, field("bounce_type")) {
    ("bounced", "Transient") => {
      "The address bounced it for now. A later send may work.".to_owned()
    }
    ("bounced", _) => "The address bounced it, so it does not take email.".to_owned(),
    ("rejected", _) => "Inboxjot rejected it before sending.".to_owned(),
    _ => "It could not be sent.".to_owned(),
  }
}

fn render_fields(fields: &[(&'static str, String)], width: usize) -> Vec<Line<'static>> {
  fields
    .iter()
    .map(|(label, value)| {
      Line::new(vec![
        Span::styled(format!("  {label:<width$}"), DIM),
        Span::raw(value.clone()),
      ])
    })
    .collect()
}

fn get_addresses(email: &Value) -> Vec<(&'static str, String)> {
  let mut fields = vec![("from", get_text(email, "from").to_owned())];
  for (label, path) in [("to", "to"), ("cc", "cc"), ("bcc", "bcc")] {
    let list = join_strings(get_path(email, path));
    if !list.is_empty() {
      fields.push((label, list));
    }
  }
  fields
}

fn get_text<'a>(email: &'a Value, path: &str) -> &'a str {
  get_path(email, path).and_then(Value::as_str).unwrap_or_default()
}

fn get_subject(email: &Value) -> String {
  let subject = get_text(email, "subject");
  if subject.is_empty() { "(no subject)".to_owned() } else { subject.to_owned() }
}

fn get_domain(from: &str) -> &str {
  let address = from
    .rsplit_once('<')
    .map_or(from, |(_, rest)| rest.trim_end_matches('>'));
  address.rsplit_once('@').map_or("", |(_, domain)| domain.trim())
}

fn format_clock(text: &str) -> Option<String> {
  let time: Timestamp = text.parse().ok()?;
  let zoned = time.to_zoned(TimeZone::system());
  let this_year = Timestamp::now().to_zoned(TimeZone::system()).year();
  let pattern =
    if zoned.year() == this_year { "%b %-d, %H:%M:%S" } else { "%b %-d %Y, %H:%M:%S" };
  Some(zoned.strftime(pattern).to_string())
}

fn read_time(event: &Value) -> Option<Timestamp> {
  event.get("occurred_at")?.as_str()?.parse().ok()
}

fn describe_event(email: &Value, event: &Value) -> String {
  let text = |key: &str| {
    event
      .get(key)
      .and_then(Value::as_str)
      .filter(|value| !value.is_empty() && *value != "unknown")
      .unwrap_or_default()
  };
  let kind = event.get("type").and_then(Value::as_str).unwrap_or_default();
  match kind.rsplit('.').next().unwrap_or_default() {
    "queued" => format_source(get_text(email, "source")).to_owned(),
    "delivered" => text("smtp_response").to_owned(),
    "opened" => {
      let device = if text("device").is_empty() { text("os") } else { text("device") };
      [text("client"), device]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
    }
    "clicked" => text("link").to_owned(),
    "bounced" | "soft_bounced" if !text("diagnostic_code").is_empty() => {
      text("diagnostic_code").to_owned()
    }
    "bounced" | "soft_bounced" => [text("bounce_type"), text("bounce_sub")]
      .into_iter()
      .filter(|part| !part.is_empty())
      .collect::<Vec<_>>()
      .join(" · "),
    "failed" | "rejected" => text("error_message").to_owned(),
    _ => String::new(),
  }
}

fn format_source(source: &str) -> &str {
  match source {
    "api" => "API",
    "smtp" => "SMTP",
    "dashboard" => "Dashboard",
    "mcp" => "MCP",
    "campaign" => "Campaign",
    "automation" => "Automation",
    _ => "",
  }
}

fn join_strings(value: Option<&Value>) -> String {
  value
    .and_then(Value::as_array)
    .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
    .unwrap_or_default()
}

fn format_tags(value: Option<&Value>) -> String {
  let Some(tags) = value.and_then(Value::as_array) else {
    return String::new();
  };
  tags
    .iter()
    .filter_map(|tag| {
      let name = tag.get("name")?.as_str()?;
      let value = tag.get("value").and_then(Value::as_str).unwrap_or_default();
      Some(if value.is_empty() { name.to_owned() } else { format!("{name}:{value}") })
    })
    .collect::<Vec<_>>()
    .join(", ")
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{
    Detail, describe_failure, format_duration, get_seconds_to_settle, render_events,
    render_sent, render_summary,
  };

  fn plain(lines: Vec<crate::ui::text::Line<'static>>) -> Vec<String> {
    lines.iter().map(|line| line.render(false)).collect()
  }

  #[test]
  fn shows_offsets_from_the_first_event_after_a_send() {
    let email = json!({ "source": "api" });
    let events = json!([
      { "type": "delivered", "occurred_at": "2026-11-10T20:05:14.000Z" },
      { "type": "queued", "occurred_at": "2026-11-10T20:05:12.669Z" },
      { "type": "sent", "occurred_at": "2026-11-10T20:05:13.000Z" },
    ]);
    let lines = plain(render_events(&email, events.as_array().unwrap(), Detail::Offsets));

    assert!(lines[0].contains("■ queued") && !lines[0].contains('+'));
    assert!(lines[1].ends_with("■ sent       +331ms"));
    assert!(lines[2].ends_with("■ delivered  +1.3s"));
  }

  #[test]
  fn shows_what_each_event_means_when_inspecting() {
    let email = json!({ "source": "api" });
    let events = json!([
      { "type": "queued", "occurred_at": "2026-11-10T20:05:13Z" },
      { "type": "opened", "occurred_at": "2026-11-10T20:09:41Z", "client": "Apple Mail", "device": "iPhone" },
      { "type": "clicked", "occurred_at": "2026-11-10T20:10:02Z", "link": "https://acme.dev/start" },
    ]);
    let lines = plain(render_events(&email, events.as_array().unwrap(), Detail::Notes));

    assert!(lines[0].ends_with("queued     API"));
    assert!(lines[1].ends_with("opened     Apple Mail · iPhone"));
    assert!(lines[2].ends_with("clicked    https://acme.dev/start"));
  }

  #[test]
  fn measures_from_creation_to_the_settling_event() {
    let email =
      json!({ "status": "delivered", "created_at": "2026-11-10T20:05:12.900Z" });
    let events = json!([
      { "type": "email.sent", "occurred_at": "2026-11-10T20:05:13.200Z" },
      { "type": "email.delivered", "occurred_at": "2026-11-10T20:05:14.000Z" },
      { "type": "email.clicked", "occurred_at": "2026-11-10T20:10:02.000Z" },
    ]);
    assert_eq!(get_seconds_to_settle(&email, events.as_array().unwrap()), Some(1.1));
    assert_eq!(get_seconds_to_settle(&json!({ "status": "sent" }), &[]), None);
  }

  #[test]
  fn formats_durations_from_milliseconds_to_minutes() {
    assert_eq!(format_duration(0.046), "46ms");
    assert_eq!(format_duration(1.04), "1.0s");
    assert_eq!(format_duration(125.0), "2m 5s");
  }

  #[test]
  fn echoes_a_send_with_its_addresses() {
    let email = json!({
      "id": "eml_1", "subject": "Welcome", "from": "hi@acme.dev",
      "to": ["riti@maily.to"], "cc": [],
    });
    let lines = plain(render_sent(&email));

    assert_eq!(
      lines,
      ["  Welcome  eml_1", "  from  hi@acme.dev", "  to    riti@maily.to"]
    );
  }

  #[test]
  fn shows_only_the_fields_that_are_set() {
    let email = json!({
      "id": "eml_1", "subject": "Welcome", "status": "delivered",
      "from": "Acme <hi@acme.dev>", "to": ["riti@maily.to"], "cc": [], "bcc": [],
      "tags": [{ "name": "onboarding", "value": "" }],
    });
    let lines = plain(render_summary(&email));

    assert_eq!(lines[0], "  Welcome  ■ delivered");
    assert!(lines.contains(&"  domain    acme.dev".to_owned()));
    assert!(lines.contains(&"  tags      onboarding".to_owned()));
    assert!(!lines.iter().any(|line| line.trim_start().starts_with("cc")));
  }

  #[test]
  fn explains_a_bounce_without_a_diagnostic() {
    let email = json!({ "status": "bounced" });
    let events = json!([{ "type": "bounced", "bounce_type": "Permanent" }]);
    assert_eq!(
      describe_failure(&email, events.as_array().unwrap()),
      "The address bounced it, so it does not take email."
    );
  }
}
