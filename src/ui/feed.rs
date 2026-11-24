use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anstyle::Style;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};

use crate::output::format_count;
use crate::ui::spinner::{self, Shown};
use crate::ui::style::{
  ACTION, DIM, SECONDARY, TRACK, get_width, has_color, hide_cursor, show_cursor,
};
use crate::ui::text::{Line, Span};

const LEVELS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
const STRIP_SECONDS: u64 = 60;
const STRIP_LABEL_WIDTH: usize = 24;
const RESERVED_ROWS: usize = 10;

type Arrivals = Arc<Mutex<VecDeque<Instant>>>;

pub struct Feed {
  bar: Option<ProgressBar>,
  shown: Option<Shown>,
  arrivals: Arrivals,
  rows: VecDeque<Vec<Line<'static>>>,
}

impl Feed {
  pub fn new(rich: bool, strip: bool, waiting: &str) -> Self {
    let arrivals: Arrivals = Arc::default();
    let shown = rich.then(Shown::new);
    let bar = rich.then(|| start_bar(strip, waiting, Arc::clone(&arrivals)));
    Self { bar, shown, arrivals, rows: VecDeque::new() }
  }

  pub fn push(&mut self, lines: Vec<Line<'static>>, counted: bool) {
    let Some(bar) = &self.bar else {
      for line in lines {
        anstream::eprintln!("{line}");
      }
      return;
    };

    if counted && let Ok(mut arrivals) = self.arrivals.lock() {
      let now = Instant::now();
      arrivals.push_back(now);
      while arrivals
        .front()
        .is_some_and(|first| now.duration_since(*first).as_secs() >= STRIP_SECONDS)
      {
        arrivals.pop_front();
      }
    }

    self.rows.push_back(lines);
    let room = get_height().saturating_sub(RESERVED_ROWS).max(3);
    let color = has_color();
    while self.rows.iter().map(Vec::len).sum::<usize>() > room {
      let Some(oldest) = self.rows.pop_front() else {
        break;
      };
      for line in oldest {
        bar.println(line.render(color));
      }
    }

    let mut text = String::new();
    for line in self.rows.iter().flatten() {
      text.push_str(&line.render(color));
      text.push('\n');
    }
    text.push('\n');
    bar.set_message(text);
  }

  pub fn finish(self) {
    let Some(bar) = self.bar else {
      return;
    };
    bar.finish_and_clear();
    show_cursor();
    for line in self.rows.iter().flatten() {
      anstream::eprintln!("{line}");
    }
    drop(self.shown);
  }
}

fn start_bar(strip: bool, waiting: &str, arrivals: Arrivals) -> ProgressBar {
  let color = has_color();
  let template = if strip { "{strip}\n\n{msg}{spinner}" } else { "{msg}{spinner}" };
  let frames: Vec<String> = spinner::FRAMES
    .iter()
    .map(|frame| {
      Line::new(vec![
        Span::raw("  "),
        Span::styled(*frame, ACTION),
        Span::styled(format!(" {waiting}"), SECONDARY),
      ])
      .render(color)
    })
    .collect();
  let frames: Vec<&str> = frames.iter().map(String::as_str).collect();
  let style = ProgressStyle::with_template(template)
    .unwrap_or_else(|_| ProgressStyle::default_spinner())
    .tick_strings(&frames)
    .with_key("strip", move |_: &ProgressState, out: &mut dyn std::fmt::Write| {
      let line = arrivals
        .lock()
        .map(|arrivals| render_strip(&arrivals, Instant::now()))
        .unwrap_or_default();
      let _ = out.write_str(&line.render(color));
    });

  let bar = ProgressBar::new_spinner().with_style(style);
  hide_cursor();
  bar.enable_steady_tick(spinner::TICK);
  bar
}

fn render_strip(arrivals: &VecDeque<Instant>, now: Instant) -> Line<'static> {
  let width = get_width()
    .saturating_sub(STRIP_LABEL_WIDTH + 2)
    .clamp(10, STRIP_SECONDS as usize);
  let window = Duration::from_secs(STRIP_SECONDS).as_secs_f64();
  let mut buckets = vec![0_u64; width];
  let mut total = 0;
  for arrival in arrivals {
    let age = now.duration_since(*arrival).as_secs_f64();
    if age >= window {
      continue;
    }
    total += 1;
    let slot = ((age / window) * width as f64) as usize;
    buckets[width - 1 - slot.min(width - 1)] += 1;
  }

  let peak = buckets.iter().copied().max().unwrap_or(0);
  let mut spans = vec![Span::raw("  ")];
  for count in buckets {
    let level = if count == 0 {
      0
    } else {
      ((count as f64 / peak as f64) * 7.0).round().max(1.0) as usize
    };
    let style = match level {
      0 | 1 => TRACK,
      2 | 3 => DIM,
      4 | 5 => SECONDARY,
      _ => Style::new(),
    };
    spans.push(Span::styled(LEVELS[level], style));
  }
  let noun = if total == 1 { "event" } else { "events" };
  spans.push(Span::styled(
    format!("  {} {noun} · last {STRIP_SECONDS}s", format_count(total)),
    DIM,
  ));
  Line::new(spans)
}

fn get_height() -> usize {
  crossterm::terminal::size()
    .ok()
    .map(|(_, rows)| usize::from(rows))
    .filter(|rows| *rows > 0)
    .unwrap_or(24)
}

#[cfg(test)]
mod tests {
  use std::collections::VecDeque;
  use std::time::{Duration, Instant};

  use super::render_strip;

  #[test]
  fn draws_recent_events_on_the_right_and_counts_them() {
    let now = Instant::now() + Duration::from_secs(120);
    let arrivals: VecDeque<Instant> =
      [now - Duration::from_secs(90), now, now].into_iter().collect();
    let text = render_strip(&arrivals, now).render(false);

    assert!(text.ends_with("█  2 events · last 60s"));
    assert!(text.trim_start().starts_with('▁'));
  }
}
