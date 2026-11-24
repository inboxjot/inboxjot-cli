mod confirm;
mod input;
mod multi_select;
mod select;
mod terminal;

pub use confirm::Confirm;
pub use input::Input;
pub use multi_select::{Choice, MultiSelect};
pub use select::Select;
pub use terminal::run;

use crossterm::event::KeyEvent;

use crate::ui::style::DIM;
use crate::ui::text::{Line, Span};

pub enum Step<T> {
  Continue,
  Submit(T),
}

pub trait Prompt {
  type Output;

  fn update(&mut self, key: KeyEvent) -> Step<Self::Output>;

  fn view(&self, width: usize, height: usize) -> Vec<Line<'static>>;

  fn summary(&self, output: &Self::Output) -> Vec<Line<'static>>;

  fn title(&self) -> Line<'static>;

  fn cancelled(&self) -> Vec<Line<'static>> {
    vec![self.title(), Line::new(vec![Span::styled("  ^C", DIM)])]
  }
}
