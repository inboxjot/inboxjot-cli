use anstyle::Style;
use crossterm::event::{KeyCode, KeyEvent};

use crate::ui::prompt::{Prompt, Step};
use crate::ui::style::{DIM, TITLE};
use crate::ui::text::{Line, Span};

pub struct Confirm {
  question: String,
  default: bool,
}

impl Confirm {
  pub fn new(question: impl Into<String>, default: bool) -> Self {
    Self { question: question.into(), default }
  }
}

impl Confirm {
  fn render_question(&self) -> Line<'static> {
    let hint = if self.default { "(Y/n) " } else { "(y/N) " };
    Line::new(vec![
      Span::styled(format!("  {}", self.question), TITLE),
      Span::raw(" "),
      Span::styled(hint, DIM),
    ])
  }
}

impl Prompt for Confirm {
  type Output = bool;

  fn update(&mut self, key: KeyEvent) -> Step<bool> {
    match key.code {
      KeyCode::Char('y' | 'Y') => Step::Submit(true),
      KeyCode::Char('n' | 'N') => Step::Submit(false),
      KeyCode::Enter => Step::Submit(self.default),
      _ => Step::Continue,
    }
  }

  fn view(&self, _width: usize, _height: usize) -> Vec<Line<'static>> {
    let mut line = self.render_question();
    line.spans.push(Span::styled(" ", Style::new().invert()));
    vec![line]
  }

  fn summary(&self, output: &bool) -> Vec<Line<'static>> {
    let mut line = self.render_question();
    line.spans.push(Span::raw(if *output { "y" } else { "n" }));
    vec![line]
  }

  fn title(&self) -> Line<'static> {
    Line::new(vec![Span::styled(format!("  {}", self.question), TITLE)])
  }

  fn cancelled(&self) -> Vec<Line<'static>> {
    let mut line = self.render_question();
    line.spans.push(Span::styled("^C", DIM));
    vec![line]
  }
}

#[cfg(test)]
mod tests {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

  use super::Confirm;
  use crate::ui::prompt::{Prompt, Step};

  fn press(prompt: &mut Confirm, code: KeyCode) -> Option<bool> {
    match prompt.update(KeyEvent::new(code, KeyModifiers::NONE)) {
      Step::Submit(answer) => Some(answer),
      Step::Continue => None,
    }
  }

  #[test]
  fn enter_takes_the_default() {
    assert_eq!(press(&mut Confirm::new("Delete?", false), KeyCode::Enter), Some(false));
    assert_eq!(press(&mut Confirm::new("Configure?", true), KeyCode::Enter), Some(true));
  }

  #[test]
  fn reads_y_and_n_in_either_case() {
    assert_eq!(
      press(&mut Confirm::new("Delete?", false), KeyCode::Char('Y')),
      Some(true)
    );
    assert_eq!(
      press(&mut Confirm::new("Delete?", true), KeyCode::Char('n')),
      Some(false)
    );
  }

  #[test]
  fn ignores_other_keys() {
    assert_eq!(press(&mut Confirm::new("Delete?", false), KeyCode::Char('x')), None);
  }

  #[test]
  fn shows_the_default_in_capitals() {
    let view = Confirm::new("Delete acme.dev?", false).view(80, 24);
    assert_eq!(view[0].render(false), "  Delete acme.dev? (y/N)  ");
    let answered = Confirm::new("Delete acme.dev?", false).summary(&true);
    assert_eq!(answered[0].render(false), "  Delete acme.dev? (y/N) y");
  }
}
