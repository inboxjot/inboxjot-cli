use std::borrow::Cow;
use std::fmt;

use anstyle::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::output::clean;

const ELLIPSIS: char = '…';

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span<'a> {
  pub content: Cow<'a, str>,
  pub style: Style,
}

impl<'a> Span<'a> {
  pub fn raw(content: impl Into<Cow<'a, str>>) -> Self {
    Self { content: content.into(), style: Style::new() }
  }

  pub fn styled(content: impl Into<Cow<'a, str>>, style: Style) -> Self {
    Self { content: content.into(), style }
  }

  pub fn width(&self) -> usize {
    clean(&self.content).width()
  }
}

impl<'a> From<&'a str> for Span<'a> {
  fn from(content: &'a str) -> Self {
    Self::raw(content)
  }
}

impl From<String> for Span<'_> {
  fn from(content: String) -> Self {
    Self::raw(content)
  }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line<'a> {
  pub spans: Vec<Span<'a>>,
}

impl<'a> Line<'a> {
  pub fn new(spans: Vec<Span<'a>>) -> Self {
    Self { spans }
  }

  pub fn blank() -> Self {
    Self::default()
  }

  pub fn width(&self) -> usize {
    self.spans.iter().map(Span::width).sum()
  }

  pub fn truncate(self, max: usize) -> Self {
    if self.width() <= max {
      return self;
    }

    let mut room = max.saturating_sub(1);
    let mut spans = Vec::new();
    for span in self.spans {
      if room == 0 {
        break;
      }
      let text = clean(&span.content).into_owned();
      let mut kept = String::new();
      for character in text.chars() {
        let width = character.width().unwrap_or(0);
        if width > room {
          room = 0;
          break;
        }
        room -= width;
        kept.push(character);
      }
      spans.push(Span::styled(kept, span.style));
    }

    let style = spans.last().map(|span| span.style).unwrap_or_default();
    spans.push(Span::styled(ELLIPSIS.to_string(), style));
    Self { spans }
  }
}

impl Line<'_> {
  pub fn render(&self, color: bool) -> String {
    if color {
      return self.to_string();
    }
    self.spans.iter().map(|span| clean(&span.content)).collect()
  }
}

impl<'a> From<Vec<Span<'a>>> for Line<'a> {
  fn from(spans: Vec<Span<'a>>) -> Self {
    Self { spans }
  }
}

impl<'a> From<Span<'a>> for Line<'a> {
  fn from(span: Span<'a>) -> Self {
    Self { spans: vec![span] }
  }
}

impl fmt::Display for Line<'_> {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    for span in &self.spans {
      let text = clean(&span.content);
      if span.style == Style::new() {
        formatter.write_str(&text)?;
      } else {
        write!(formatter, "{}{text}{}", span.style.render(), span.style.render_reset())?;
      }
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use anstyle::Style;

  use super::{Line, Span};

  #[test]
  fn measures_wide_characters() {
    let line = Line::new(vec![Span::raw("ab"), Span::raw("日本")]);
    assert_eq!(line.width(), 6);
  }

  #[test]
  fn truncates_with_an_ellipsis_inside_the_width() {
    let line = Line::new(vec![Span::raw("hello "), Span::raw("world")]).truncate(8);
    assert_eq!(line.width(), 8);
    assert_eq!(line.to_string(), "hello w…");
  }

  #[test]
  fn never_splits_a_wide_character() {
    let line = Line::from(Span::raw("日本語")).truncate(4);
    assert_eq!(line.to_string(), "日…");
    assert!(line.width() <= 4);
  }

  #[test]
  fn keeps_short_lines_as_they_are() {
    let line = Line::from(Span::raw("short")).truncate(80);
    assert_eq!(line.to_string(), "short");
  }

  #[test]
  fn escapes_control_characters_from_untrusted_text() {
    let line = Line::from(Span::styled("Riti\u{1b}[2J", Style::new()));
    assert_eq!(line.to_string(), "Riti\\u{1b}[2J");
  }
}
