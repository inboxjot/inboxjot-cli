use anstyle::Style;

use crate::ui::style::TRACK;
use crate::ui::text::{Line, Span};

const EIGHTHS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];
const FULL: &str = "█";
const PLAIN_TRACK: &str = "░";

pub struct Bar {
  pub width: usize,
  pub color: bool,
}

impl Bar {
  pub fn render(&self, progress: f64) -> Line<'static> {
    let progress = if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 0.0 };
    let eighths = (progress * (self.width * 8) as f64).round() as usize;
    let whole = eighths / 8;
    let part = eighths % 8;
    let rest = self.width - whole - usize::from(part > 0);

    let track =
      if self.color { Style::new().bg_color(TRACK.get_fg_color()) } else { Style::new() };
    let mut spans = vec![Span::raw(FULL.repeat(whole))];
    if part > 0 {
      spans.push(Span::styled(EIGHTHS[part], track));
    }
    if rest > 0 {
      let empty = if self.color { " " } else { PLAIN_TRACK };
      spans.push(Span::styled(empty.repeat(rest), track));
    }
    Line::new(spans)
  }
}

pub fn render_thin(width: usize, fraction: f64, style: Style) -> Vec<Span<'static>> {
  let fraction = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
  let mut filled = (fraction * width as f64).round() as usize;
  let tick = fraction > 0.0 && filled == 0;
  if tick {
    filled = 1;
  }
  let lead = if tick { "╸".to_owned() } else { "━".repeat(filled) };
  let mut spans = vec![Span::styled(lead, style)];
  if filled < width {
    spans.push(Span::styled("━".repeat(width - filled), TRACK));
  }
  spans
}

#[cfg(test)]
mod tests {
  use anstyle::Style;

  use super::{Bar, render_thin};
  use crate::ui::text::Line;

  fn plain(progress: f64) -> String {
    Bar { width: 8, color: false }.render(progress).to_string()
  }

  #[test]
  fn fills_in_eighths_of_a_cell() {
    assert_eq!(plain(0.0), "░░░░░░░░");
    assert_eq!(plain(0.5), "████░░░░");
    assert_eq!(plain(1.0 / 64.0), "▏░░░░░░░");
    assert_eq!(plain(1.0), "████████");
  }

  #[test]
  fn always_fills_the_width() {
    for step in 0..=100 {
      let line = Bar { width: 40, color: true }.render(f64::from(step) / 100.0);
      assert_eq!(line.width(), 40, "at {step}%");
    }
  }

  #[test]
  fn clamps_bad_progress() {
    assert_eq!(plain(-1.0), "░░░░░░░░");
    assert_eq!(plain(2.0), "████████");
    assert_eq!(plain(f64::NAN), "░░░░░░░░");
  }

  #[test]
  fn thin_bars_show_a_tick_for_tiny_values() {
    let line =
      |fraction| Line::new(render_thin(10, fraction, Style::new())).render(false);
    assert_eq!(line(0.0), "━━━━━━━━━━");
    assert_eq!(line(0.001), "╸━━━━━━━━━");
    assert_eq!(line(0.5), "━━━━━━━━━━");
    assert_eq!(Line::new(render_thin(10, 0.5, Style::new())).width(), 10);
  }
}
