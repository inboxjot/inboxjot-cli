use anstyle::Style;

use crate::ui::style::{DANGER, DONE, SECONDARY, WARNING};
use crate::ui::text::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
  Done,
  Removed,
  Warning,
  Error,
  Status(Style),
}

impl Mark {
  pub fn glyph(self) -> &'static str {
    match self {
      Self::Done => "◆",
      Self::Removed => "◇",
      Self::Warning => "▲",
      Self::Error | Self::Status(_) => "■",
    }
  }

  pub fn style(self) -> Style {
    match self {
      Self::Done => DONE,
      Self::Removed => SECONDARY,
      Self::Warning => WARNING,
      Self::Error => DANGER,
      Self::Status(style) => style,
    }
  }

  pub fn span(self) -> Span<'static> {
    Span::styled(self.glyph(), self.style())
  }
}
