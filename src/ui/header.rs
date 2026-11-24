use crate::ui::style::{ACTION, DIM, FAINT, SECONDARY, TITLE};
use crate::ui::text::{Line, Span};

const WORDMARK: [&str; 3] = [
  "▄▄ ▄▄  ▄▄ ▄▄▄▄   ▄▄▄  ▄▄ ▄▄   ▄▄  ▄▄▄  ▄▄▄▄▄▄",
  "██ ███▄██ ██▄██ ██▀██ ▀█▄█▀   ██ ██▀██   ██",
  "██ ██ ▀██ ██▄█▀ ▀███▀ ██ ██ ▄▄█▀ ▀███▀   ██",
];
const TAGLINE: &str = "Email for builders and their agents";
const INDENT: &str = "  ";
const ROOMY: usize = 50;

pub fn render(version: &str, width: usize) -> Vec<Line<'static>> {
  let version = format!("v{version}");
  let mut lines = Vec::new();

  if width >= ROOMY {
    lines.extend(
      WORDMARK
        .iter()
        .map(|row| Line::new(vec![Span::raw(INDENT), Span::raw(*row)])),
    );
    lines.push(Line::blank());
    lines.push(Line::blank());
    lines.push(Line::new(vec![
      Span::raw(INDENT),
      Span::styled(TAGLINE, SECONDARY),
      Span::styled(format!("  {version}"), DIM),
    ]));
    lines.push(Line::new(vec![
      Span::raw(INDENT),
      Span::styled("inboxjot login", ACTION),
      Span::styled(" to start", DIM),
      Span::styled(" · ", FAINT),
      Span::styled("--help", ACTION),
      Span::styled(" for commands", DIM),
      Span::styled(" · ", FAINT),
      Span::styled("inboxjot.com/docs", DIM),
    ]));
  } else {
    lines.push(Line::new(vec![
      Span::raw(INDENT),
      Span::styled("inboxjot", TITLE),
      Span::styled(format!("  {version}"), DIM),
    ]));
    lines.push(Line::new(vec![Span::raw(INDENT), Span::styled(TAGLINE, SECONDARY)]));
    lines.push(Line::new(vec![
      Span::raw(INDENT),
      Span::styled("login", ACTION),
      Span::styled(" · ", FAINT),
      Span::styled("--help", ACTION),
    ]));
  }

  lines
    .into_iter()
    .map(|line| line.truncate(width.saturating_sub(1)))
    .collect()
}

#[cfg(test)]
mod tests {
  use super::render;

  #[test]
  fn shows_the_wordmark_when_it_fits() {
    let lines = render("0.1.0", 100);
    assert!(lines[0].to_string().contains("▄▄ ▄▄"));
    assert!(lines.iter().any(|line| line.to_string().contains("v0.1.0")));
  }

  #[test]
  fn drops_to_one_line_on_narrow_terminals() {
    let lines = render("0.1.0", 40);
    assert!(lines[0].to_string().contains("inboxjot"));
    assert!(lines.iter().all(|line| line.width() < 40));
  }
}
