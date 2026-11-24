use anstyle::Style;
use crossterm::event::{KeyCode, KeyEvent};
use unicode_width::UnicodeWidthStr;

use crate::ui::prompt::{Prompt, Step};
use crate::ui::style::{ACTION, DANGER, DIM, DONE, FAINT, SECONDARY, TITLE};
use crate::ui::text::{Line, Span};

const CHROME_LINES: usize = 6;

#[derive(Clone, Debug)]
pub struct Choice {
  pub label: String,
  pub detail: String,
  pub note: String,
  pub group: String,
  pub enabled: bool,
  pub selected: bool,
}

pub struct MultiSelect {
  title: String,
  choices: Vec<Choice>,
  cursor: usize,
  error: Option<String>,
}

enum Row<'a> {
  Group(&'a str),
  Gap,
  Choice(usize),
}

impl MultiSelect {
  pub fn new(title: impl Into<String>, choices: Vec<Choice>) -> Self {
    let cursor = choices.iter().position(|choice| choice.enabled).unwrap_or(0);
    Self { title: title.into(), choices, cursor, error: None }
  }

  fn move_by(&mut self, step: isize) {
    let mut index = self.cursor;
    loop {
      let Some(next) = index
        .checked_add_signed(step)
        .filter(|next| *next < self.choices.len())
      else {
        return;
      };
      index = next;
      if self.choices[index].enabled {
        self.cursor = index;
        return;
      }
    }
  }

  fn toggle_all(&mut self) {
    let enabled = self.choices.iter().filter(|choice| choice.enabled);
    let select = enabled.clone().any(|choice| !choice.selected);
    for choice in self.choices.iter_mut().filter(|choice| choice.enabled) {
      choice.selected = select;
    }
  }

  fn get_rows(&self) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    let mut group: Option<&str> = None;
    for (index, choice) in self.choices.iter().enumerate() {
      if group != Some(choice.group.as_str()) {
        if group.is_some() {
          rows.push(Row::Gap);
        }
        group = Some(&choice.group);
        if !choice.group.is_empty() {
          rows.push(Row::Group(&choice.group));
        }
      }
      rows.push(Row::Choice(index));
    }
    rows
  }

  fn render_choice(&self, index: usize, widths: (usize, usize)) -> Line<'static> {
    let choice = &self.choices[index];
    let label = format!("{:<width$}", choice.label, width = widths.0 + 2);
    let detail = format!("{:<width$}", choice.detail, width = widths.1 + 2);
    if !choice.enabled {
      return Line::new(vec![
        Span::raw("      "),
        Span::styled(label, DIM),
        Span::styled(detail, DIM),
        Span::styled(choice.note.clone(), DIM),
      ]);
    }

    let is_current = index == self.cursor;
    Line::new(vec![
      Span::raw("  "),
      Span::styled(if is_current { "› " } else { "  " }, ACTION),
      if choice.selected {
        Span::styled("◼ ", DONE)
      } else {
        Span::styled("◻ ", DIM)
      },
      Span::styled(label, if choice.selected { Style::new() } else { SECONDARY }),
      Span::styled(detail, DIM),
      Span::styled(choice.note.clone(), DIM),
    ])
  }
}

impl Prompt for MultiSelect {
  type Output = Vec<usize>;

  fn update(&mut self, key: KeyEvent) -> Step<Vec<usize>> {
    match key.code {
      KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
      KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
      KeyCode::Char(' ') => {
        if let Some(choice) =
          self.choices.get_mut(self.cursor).filter(|choice| choice.enabled)
        {
          choice.selected = !choice.selected;
          self.error = None;
        }
      }
      KeyCode::Char('a') => {
        self.toggle_all();
        self.error = None;
      }
      KeyCode::Enter => {
        let picked: Vec<usize> = (0..self.choices.len())
          .filter(|index| self.choices[*index].enabled && self.choices[*index].selected)
          .collect();
        if !picked.is_empty() {
          return Step::Submit(picked);
        }
        self.error =
          Some("Pick at least one with space, or press esc to cancel.".to_owned());
      }
      _ => {}
    }
    Step::Continue
  }

  fn view(&self, _width: usize, height: usize) -> Vec<Line<'static>> {
    let widths = (
      self
        .choices
        .iter()
        .map(|choice| choice.label.width())
        .max()
        .unwrap_or(0),
      self
        .choices
        .iter()
        .map(|choice| choice.detail.width())
        .max()
        .unwrap_or(0),
    );
    let rows = self.get_rows();
    let room = height.saturating_sub(CHROME_LINES).max(3);
    let current = rows
      .iter()
      .position(|row| matches!(row, Row::Choice(index) if *index == self.cursor))
      .unwrap_or(0);
    let start = if rows.len() <= room {
      0
    } else {
      current.saturating_sub(room / 2).min(rows.len() - room)
    };
    let end = (start + room).min(rows.len());

    let mut lines = vec![self.title(), Line::blank()];
    if start > 0 {
      lines.push(Line::new(vec![Span::styled(format!("    ↑ {start} more"), DIM)]));
    }
    for row in &rows[start..end] {
      lines.push(match row {
        Row::Group(name) => {
          Line::new(vec![Span::raw("  "), Span::styled((*name).to_owned(), SECONDARY)])
        }
        Row::Gap => Line::blank(),
        Row::Choice(index) => self.render_choice(*index, widths),
      });
    }
    if end < rows.len() {
      lines.push(Line::new(vec![Span::styled(
        format!("    ↓ {} more", rows.len() - end),
        DIM,
      )]));
    }
    lines.push(Line::blank());
    lines.push(Line::new(vec![
      Span::styled("  space", DIM),
      Span::styled(" toggle · ", FAINT),
      Span::styled("↑↓", DIM),
      Span::styled(" move · ", FAINT),
      Span::styled("enter", DIM),
      Span::styled(" next", FAINT),
    ]));
    if let Some(error) = &self.error {
      lines.push(Line::new(vec![Span::raw("  "), Span::styled(error.clone(), DANGER)]));
    }
    lines
  }

  fn summary(&self, _output: &Vec<usize>) -> Vec<Line<'static>> {
    self.view(usize::MAX, usize::MAX)
  }

  fn title(&self) -> Line<'static> {
    Line::new(vec![Span::styled(format!("  {}", self.title), TITLE)])
  }
}

#[cfg(test)]
mod tests {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

  use super::{Choice, MultiSelect};
  use crate::ui::prompt::{Prompt, Step};

  fn choice(label: &str, enabled: bool, selected: bool) -> Choice {
    Choice {
      label: label.to_owned(),
      detail: String::new(),
      note: String::new(),
      group: if enabled { "Found".to_owned() } else { "Not installed".to_owned() },
      enabled,
      selected,
    }
  }

  fn press(select: &mut MultiSelect, code: KeyCode) -> Option<Vec<usize>> {
    match select.update(KeyEvent::new(code, KeyModifiers::NONE)) {
      Step::Submit(picked) => Some(picked),
      Step::Continue => None,
    }
  }

  fn tools() -> MultiSelect {
    MultiSelect::new(
      "Which tools?",
      vec![
        choice("Claude Code", true, true),
        choice("Cursor", true, false),
        choice("VS Code", false, false),
      ],
    )
  }

  #[test]
  fn submits_the_picked_choices() {
    let mut select = tools();
    let _ = press(&mut select, KeyCode::Down);
    let _ = press(&mut select, KeyCode::Char(' '));
    assert_eq!(press(&mut select, KeyCode::Enter), Some(vec![0, 1]));
  }

  #[test]
  fn skips_choices_that_cannot_be_picked() {
    let mut select = tools();
    let _ = press(&mut select, KeyCode::Down);
    let _ = press(&mut select, KeyCode::Down);
    assert_eq!(select.cursor, 1);
    let _ = press(&mut select, KeyCode::Up);
    let _ = press(&mut select, KeyCode::Up);
    assert_eq!(select.cursor, 0);
  }

  #[test]
  fn toggles_all_but_the_disabled_ones() {
    let mut select = tools();
    let _ = press(&mut select, KeyCode::Char('a'));
    assert_eq!(press(&mut select, KeyCode::Enter), Some(vec![0, 1]));
    let _ = press(&mut select, KeyCode::Char('a'));
    assert!(select.choices.iter().all(|choice| !choice.selected));
  }

  #[test]
  fn refuses_to_submit_nothing() {
    let mut select = tools();
    let _ = press(&mut select, KeyCode::Char(' '));
    assert_eq!(press(&mut select, KeyCode::Enter), None);
    let view = select.view(80, 24);
    assert!(view.last().unwrap().render(false).contains("Pick at least one"));
  }

  #[test]
  fn groups_choices_with_a_gap_between() {
    let view: Vec<String> =
      tools().view(80, 24).iter().map(|line| line.render(false)).collect();
    assert_eq!(view[2], "  Found");
    assert!(view[3].starts_with("  › ◼ Claude Code"));
    assert_eq!(view[5], "");
    assert_eq!(view[6], "  Not installed");
    assert!(view[7].starts_with("      VS Code"));
  }

  #[test]
  fn scrolls_long_lists_around_the_cursor() {
    let choices = (0..30)
      .map(|index| choice(&format!("Tool {index}"), true, false))
      .collect();
    let mut select = MultiSelect::new("Which tools?", choices);
    for _ in 0..20 {
      let _ = press(&mut select, KeyCode::Down);
    }
    let view: Vec<String> =
      select.view(80, 12).iter().map(|line| line.render(false)).collect();
    assert!(view.len() <= 12);
    assert!(view.iter().any(|line| line.contains("› ◻ Tool 20")));
    assert!(view.iter().any(|line| line.contains("more")));
  }
}
