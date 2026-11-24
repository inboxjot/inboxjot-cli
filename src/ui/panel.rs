use crate::ui::style::BORDER;
use crate::ui::text::{Line, Span};

const MARGIN: &str = "  ";
const MIN_INNER_WIDTH: usize = 56;

pub fn render(content: Vec<Line<'static>>, max_width: usize) -> Vec<Line<'static>> {
  let room = max_width.saturating_sub(MARGIN.len() + 6).max(10);
  let content: Vec<Line<'static>> =
    content.into_iter().map(|line| line.truncate(room)).collect();
  let longest = content.iter().map(Line::width).max().unwrap_or(0);
  let inner = longest.max(MIN_INNER_WIDTH).min(room);
  let rule = "─".repeat(inner + 4);

  let row = |line: Line<'static>| {
    let padding = " ".repeat(inner.saturating_sub(line.width()) + 2);
    let mut spans = vec![Span::raw(MARGIN), Span::styled("│  ", BORDER)];
    spans.extend(line.spans);
    spans.push(Span::raw(padding));
    spans.push(Span::styled("│", BORDER));
    Line::new(spans)
  };

  let mut lines =
    vec![Line::new(vec![Span::raw(MARGIN), Span::styled(format!("╭{rule}╮"), BORDER)])];
  lines.push(row(Line::blank()));
  lines.extend(content.into_iter().map(row));
  lines.push(row(Line::blank()));
  lines
    .push(Line::new(vec![Span::raw(MARGIN), Span::styled(format!("╰{rule}╯"), BORDER)]));
  lines
}

#[cfg(test)]
mod tests {
  use super::render;
  use crate::ui::text::{Line, Span};

  #[test]
  fn draws_a_padded_box_at_least_the_design_width() {
    let lines = render(vec![Line::from(Span::raw("short"))], 120);
    let text: Vec<String> = lines.iter().map(|line| line.render(false)).collect();
    assert_eq!(text.len(), 5);
    assert_eq!(text[0], format!("  ╭{}╮", "─".repeat(60)));
    assert_eq!(text[1], format!("  │{}│", " ".repeat(60)));
    assert_eq!(text[2], format!("  │  short{}│", " ".repeat(53)));
    assert_eq!(text[4], format!("  ╰{}╯", "─".repeat(60)));
  }

  #[test]
  fn clips_lines_wider_than_the_terminal() {
    let long = Line::from(Span::raw("x".repeat(200)));
    let lines = render(vec![long], 40);
    assert!(lines.iter().all(|line| line.width() <= 40));
  }
}
