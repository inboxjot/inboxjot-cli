use std::io::IsTerminal;

use anstream::{AutoStream, ColorChoice};
use anstyle::{Ansi256Color, AnsiColor, Color, Style};

pub const TITLE: Style = Style::new().bold();
pub const SECONDARY: Style = get_gray(247);
pub const DIM: Style = get_gray(241);
pub const FAINT: Style = get_gray(239);
pub const BORDER: Style = get_gray(238);
pub const TRACK: Style = get_gray(235);
pub const ACTION: Style = AnsiColor::Cyan.on_default();
pub const DONE: Style = AnsiColor::Green.on_default();
pub const WARNING: Style = AnsiColor::Yellow.on_default();
pub const DANGER: Style = AnsiColor::Red.on_default();

const FALLBACK_WIDTH: usize = 80;

pub fn get_event_style(event_type: &str) -> Style {
  let tone = match event_type.rsplit('.').next().unwrap_or_default() {
    "queued" | "scheduled" => return DIM,
    "sent" => AnsiColor::Blue,
    "soft_bounced" => AnsiColor::Yellow,
    "unsubscribed" => return SECONDARY,
    "delivered" | "subscribed" | "verified" | "active" | "enabled" => AnsiColor::Green,
    "opened" | "clicked" => AnsiColor::Magenta,
    "bounced" | "bounce" | "failed" | "rejected" => AnsiColor::Red,
    "complained" | "complaint" | "delivery_delayed" | "pending" => AnsiColor::Yellow,
    _ if event_type.starts_with("contact.") => AnsiColor::Cyan,
    _ => return DIM,
  };
  tone.on_default()
}

const fn get_gray(index: u8) -> Style {
  Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(index))))
}

pub fn hide_cursor() {
  if std::io::stderr().is_terminal() {
    let _ = crossterm::execute!(std::io::stderr(), crossterm::cursor::Hide);
  }
}

pub fn show_cursor() {
  if std::io::stderr().is_terminal() {
    let _ = crossterm::execute!(std::io::stderr(), crossterm::cursor::Show);
  }
}

pub fn has_color() -> bool {
  AutoStream::choice(&std::io::stderr()) != ColorChoice::Never
}

pub fn get_width() -> usize {
  if !std::io::stderr().is_terminal() {
    return FALLBACK_WIDTH;
  }

  crossterm::terminal::size()
    .ok()
    .map(|(columns, _)| usize::from(columns))
    .filter(|columns| *columns > 0)
    .unwrap_or(FALLBACK_WIDTH)
}

#[cfg(test)]
mod tests {
  use anstyle::AnsiColor;

  use super::{DIM, SECONDARY, get_event_style};

  #[test]
  fn colors_events_by_their_kind() {
    assert_eq!(get_event_style("email.delivered"), AnsiColor::Green.on_default());
    assert_eq!(get_event_style("email.bounced"), AnsiColor::Red.on_default());
    assert_eq!(get_event_style("email.opened"), AnsiColor::Magenta.on_default());
    assert_eq!(get_event_style("contact.created"), AnsiColor::Cyan.on_default());
    assert_eq!(get_event_style("email.queued"), DIM);
    assert_eq!(get_event_style("email.unsubscribed"), SECONDARY);
    assert_eq!(get_event_style("bounce"), AnsiColor::Red.on_default());
    assert_eq!(get_event_style("something.new"), DIM);
  }
}
