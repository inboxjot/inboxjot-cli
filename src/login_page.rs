use crate::credentials::OAuthTokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
  Full,
  ReadOnly,
  Custom,
  Unknown,
}

impl Access {
  pub fn read(tokens: &OAuthTokens) -> Self {
    let granted = &tokens.scopes;
    let supported = &tokens.scopes_supported;
    let reads: Vec<&String> =
      supported.iter().filter(|scope| scope.ends_with(":read")).collect();

    if granted.is_empty() {
      Self::Unknown
    } else if !supported.is_empty()
      && supported.iter().all(|scope| granted.contains(scope))
    {
      Self::Full
    } else if !reads.is_empty()
      && granted.len() == reads.len()
      && reads.iter().all(|scope| granted.contains(scope))
    {
      Self::ReadOnly
    } else {
      Self::Custom
    }
  }

  const fn describe(self) -> &'static str {
    match self {
      Self::Full => "full access",
      Self::ReadOnly => "read-only access",
      Self::Custom | Self::Unknown => "the access you approved",
    }
  }
}

pub enum LoginOutcome<'a> {
  LoggedIn { device: &'a str, workspace: &'a str, access: Access },
  Canceled,
  Failed { reason: &'a str },
}

const PAGE: &str = include_str!("login_page.html");

pub fn render_login_page(outcome: &LoginOutcome<'_>) -> String {
  let (title, body, command) = match outcome {
    LoginOutcome::LoggedIn { device, workspace, access } => (
      "Inboxjot CLI Is Logged In".to_owned(),
      format!(
        "{} can now work in {} with {}. You can close this tab and go back to the terminal.",
        escape_html(device),
        escape_html(workspace),
        access.describe()
      ),
      "inboxjot whoami",
    ),
    LoginOutcome::Canceled => (
      "Login Canceled".to_owned(),
      "The CLI did not get access to your workspace. Run the command again in the terminal to log in."
        .to_owned(),
      "inboxjot login",
    ),
    LoginOutcome::Failed { reason } => (
      "Login Did Not Finish".to_owned(),
      format!("{} Run the command again in the terminal.", escape_html(reason)),
      "inboxjot login",
    ),
  };

  PAGE
    .replace("{{title}}", &title)
    .replace("{{body}}", &body)
    .replace("{{command}}", command)
}

fn escape_html(text: &str) -> String {
  text
    .replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
    .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
  use super::{Access, LoginOutcome, render_login_page};

  #[test]
  fn shows_the_workspace_and_access_of_a_login() {
    let page = render_login_page(&LoginOutcome::LoggedIn {
      device: "riti-thinkpad",
      workspace: "Acme <Mail>",
      access: Access::Full,
    });

    assert!(page.contains("Inboxjot CLI Is Logged In"));
    assert!(
      page.contains("riti-thinkpad can now work in Acme &lt;Mail&gt; with full access.")
    );
    assert!(page.contains("inboxjot whoami"));
  }

  #[test]
  fn asks_to_log_in_again_after_a_failure() {
    let page = render_login_page(&LoginOutcome::Failed { reason: "The code expired." });

    assert!(page.contains("Login Did Not Finish"));
    assert!(page.contains("The code expired. Run the command again in the terminal."));
    assert!(page.contains("inboxjot login"));
  }
}
