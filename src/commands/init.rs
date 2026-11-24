use std::path::{Path, PathBuf};

use clap::Args;
use serde_json::{Value, json};

use crate::commands::api_keys::{self, KeyExpiry, Permission};
use crate::commands::trigger::find_verified_domain;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::ui::code::highlight;
use crate::ui::panel;
use crate::ui::prompt::{self, Select};
use crate::ui::style::get_width;
use crate::ui::text::Line;

const ENV_FILE: &str = ".env.local";
const ENV_NAME: &str = "INBOXJOT_API_KEY";
const IGNORE_PATTERNS: [&str; 6] =
  [".env.local", "/.env.local", ".env*.local", ".env*", ".env.*", "*.local"];

#[derive(Debug, Args)]
pub struct InitArgs {
  #[arg(long, default_value = ".", help = "Project directory")]
  pub dir: PathBuf,

  #[arg(long, value_enum, default_value_t = Permission::Sending, help = "What the new key can do")]
  pub permission: Permission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stack {
  Node,
  Python,
  Rust,
  Go,
  Curl,
}

const STACKS: [Stack; 5] =
  [Stack::Node, Stack::Python, Stack::Rust, Stack::Go, Stack::Curl];

impl Stack {
  fn label(self) -> &'static str {
    match self {
      Self::Node => "Node.js",
      Self::Python => "Python",
      Self::Rust => "Rust",
      Self::Go => "Go",
      Self::Curl => "cURL",
    }
  }

  fn name(self) -> &'static str {
    match self {
      Self::Node => "node",
      Self::Python => "python",
      Self::Rust => "rust",
      Self::Go => "go",
      Self::Curl => "curl",
    }
  }

  fn get_install_command(self, dir: &Path) -> Option<String> {
    Some(match self {
      Self::Node => find_node_install(dir).to_owned(),
      Self::Python => find_python_install(dir).to_owned(),
      Self::Rust => "cargo add inboxjot".to_owned(),
      Self::Go => "go get github.com/inboxjot/inboxjot-go".to_owned(),
      Self::Curl => return None,
    })
  }

  fn snippet(self, api_url: &str, from: &str) -> String {
    let to = "delivered@inboxjot.dev";
    match self {
      Self::Node => format!(
        "import {{ Inboxjot }} from 'inboxjot';\nconst inboxjot = new Inboxjot();\nawait inboxjot.emails.send({{\n  from: '{from}',\n  to: '{to}',\n  subject: 'Hello',\n  text: 'It works',\n}});"
      ),
      Self::Python => format!(
        "from inboxjot import Inboxjot\ninboxjot = Inboxjot()\ninboxjot.emails.send({{\n    \"from\": \"{from}\",\n    \"to\": \"{to}\",\n    \"subject\": \"Hello\",\n    \"text\": \"It works\",\n}})"
      ),
      Self::Rust => format!(
        "use inboxjot::{{Inboxjot, SendEmail}};\nlet client = Inboxjot::from_env()?;\nclient.emails().send(&SendEmail {{\n  from: Some(\"{from}\".into()),\n  to: vec![\"{to}\".into()],\n  subject: Some(\"Hello\".into()),\n  text: Some(\"It works\".into()),\n  ..Default::default()\n}}).await?;"
      ),
      Self::Go => format!(
        "client := inboxjot.NewClient(\"\")\n_, err := client.Emails.Send(ctx, inboxjot.SendEmailParams{{\n  From:    \"{from}\",\n  To:      []string{{\"{to}\"}},\n  Subject: \"Hello\",\n  Text:    \"It works\",\n}})"
      ),
      Self::Curl => format!(
        "curl {}/v1/emails \\\n  -H \"Authorization: Bearer $INBOXJOT_API_KEY\" \\\n  -H \"Content-Type: application/json\" \\\n  -d '{{\n    \"from\": \"{from}\",\n    \"to\": \"{to}\",\n    \"subject\": \"Hello\",\n    \"text\": \"It works\"\n  }}'",
        api_url.trim_end_matches('/')
      ),
    }
  }
}

pub async fn run(args: &InitArgs, context: &Context) -> Result<()> {
  let dir = std::fs::canonicalize(&args.dir).map_err(|error| {
    Error::usage("invalid_dir", format!("Cannot open {}: {error}", args.dir.display()))
  })?;
  let project = dir
    .file_name()
    .map_or_else(|| "project".into(), |name| name.to_string_lossy());
  let env_path = dir.join(ENV_FILE);
  let existing = read_optional(&env_path)?;

  let api = context.login_api().await?.loading("Creating a Sending key…");
  let (detected, found) = detect_stack(&dir);
  let stack = if context.interactive && context.output.is_rich() {
    let note = match found {
      Some(file) => format!("Found {file} in this folder."),
      None => "No project file in this folder.".to_owned(),
    };
    let choices = STACKS.iter().map(|stack| stack.label().to_owned()).collect();
    let cursor = STACKS.iter().position(|stack| *stack == detected).unwrap_or(0);
    let select =
      Select::new("Which SDK should the snippet use?", choices, cursor).with_note(note);
    STACKS[prompt::run(select)?]
  } else {
    detected
  };
  if existing.lines().any(is_key_line) {
    if context.interactive {
      anstream::eprintln!();
    }
    context.confirm(&format!("Replace {ENV_NAME} in {ENV_FILE}?"))?;
  }

  let name = format!("{project} (inboxjot init)");
  let key =
    api_keys::create_key(&api, &name, args.permission, None, KeyExpiry::Never, &[])
      .await?;

  let (content, kept) = merge_env(&existing, &api_keys::get_full_key(&key));
  std::fs::write(&env_path, content)?;
  let ignored = add_to_gitignore(&dir)?;
  let from = match find_verified_domain(&api).await {
    Ok(domain) => format!("hi@{domain}"),
    Err(_) => "hi@your-domain.com".to_owned(),
  };
  let snippet = stack.snippet(&context.get_api_url()?, &from);

  let result = json!({
    "key_id": key.get("id").cloned().unwrap_or(Value::Null),
    "env_file": env_path.display().to_string(),
    "stack": stack.name(),
  });
  if context.output.is_rich() {
    if context.interactive {
      anstream::eprintln!();
    }
    let install = stack.get_install_command(&dir);
    print_setup(
      install.as_deref(),
      &snippet,
      &name,
      args.permission,
      kept,
      ignored,
      context,
    );
    return Ok(());
  }

  let headline = format!("Saved a new API key to {ENV_FILE}");
  context.output.print_result(&result, &headline, "key_id", &[])?;
  context.output.info(&format!("The key is named {name}."));
  if kept > 0 {
    let noun = if kept == 1 { "value" } else { "values" };
    context.output.info(&format!("Kept {kept} other {noun}."));
  }
  if ignored {
    context.output.info(&format!("Added {ENV_FILE} to .gitignore."));
  }

  if let Some(command) = stack.get_install_command(&dir) {
    context.output.next("Install the SDK:", &command);
  }
  context.output.info("Send an email:");
  for line in snippet.lines() {
    context.output.info(&format!("  {line}"));
  }
  Ok(())
}

fn print_setup(
  install: Option<&str>,
  snippet: &str,
  name: &str,
  permission: Permission,
  kept: usize,
  ignored: bool,
  context: &Context,
) {
  let access = match permission {
    Permission::Sending => "Sending access",
    Permission::Full => "Full access",
  };
  context.output.done(&format!("Created a {access} key"), name);
  context.output.success(&format!("Saved {ENV_NAME} to {ENV_FILE}"));
  if kept > 0 {
    let noun = if kept == 1 { "value" } else { "values" };
    context.output.info(&format!("Kept {kept} other {noun}."));
  }
  if ignored {
    context.output.success(&format!("Added {ENV_FILE} to .gitignore"));
  }

  let code: Vec<Line<'static>> = snippet.lines().map(highlight).collect();
  anstream::eprintln!();
  for line in panel::render(code, get_width()) {
    anstream::eprintln!("{line}");
  }
  if let Some(command) = install {
    context.output.print_next(&[("install the SDK", command)]);
  }
}

fn read_optional(path: &Path) -> Result<String> {
  match std::fs::read_to_string(path) {
    Ok(text) => Ok(text),
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
    Err(error) => Err(error.into()),
  }
}

fn is_key_line(line: &str) -> bool {
  line.split_once('=').is_some_and(|(name, _)| {
    let name = name.trim();
    name.strip_prefix("export ").unwrap_or(name).trim() == ENV_NAME
  })
}

fn is_value_line(line: &str) -> bool {
  let line = line.trim_start();
  !line.starts_with('#') && line.contains('=')
}

fn merge_env(existing: &str, api_key: &str) -> (String, usize) {
  let entry = format!("{ENV_NAME}={api_key}");
  let mut content = String::new();
  let mut kept = 0;
  let mut is_written = false;

  for line in existing.split_inclusive('\n') {
    let body = line.trim_end_matches(['\n', '\r']);
    if !is_key_line(body) {
      kept += usize::from(is_value_line(body));
      content.push_str(line);
    } else if !is_written {
      content.push_str(&entry);
      content.push_str(&line[body.len()..]);
      is_written = true;
    }
  }

  if !is_written {
    if !content.is_empty() && !content.ends_with('\n') {
      content.push('\n');
    }
    content.push_str(&entry);
    content.push('\n');
  }

  (content, kept)
}

fn add_to_gitignore(dir: &Path) -> Result<bool> {
  let path = dir.join(".gitignore");
  let Ok(mut content) = std::fs::read_to_string(&path) else {
    return Ok(false);
  };

  if content.lines().any(|line| IGNORE_PATTERNS.contains(&line.trim())) {
    return Ok(false);
  }

  if !content.is_empty() && !content.ends_with('\n') {
    content.push('\n');
  }
  content.push_str(ENV_FILE);
  content.push('\n');
  std::fs::write(&path, content)?;
  Ok(true)
}

fn find_node_install(dir: &Path) -> &'static str {
  let lockfiles = [
    ("bun.lock", "bun"),
    ("bun.lockb", "bun"),
    ("deno.lock", "deno"),
    ("pnpm-lock.yaml", "pnpm"),
    ("pnpm-workspace.yaml", "pnpm"),
    ("yarn.lock", "yarn"),
    ("package-lock.json", "npm"),
    ("npm-shrinkwrap.json", "npm"),
  ];
  let manager = find_in_project(dir, |dir| {
    find_marker(dir, &lockfiles).or_else(|| read_package_manager(dir))
  })
  .or_else(read_user_agent);
  match manager.as_deref() {
    Some("pnpm") => "pnpm add inboxjot",
    Some("yarn") => "yarn add inboxjot",
    Some("bun") => "bun add inboxjot",
    Some("deno") => "deno add npm:inboxjot",
    _ => "npm install inboxjot",
  }
}

fn read_package_manager(dir: &Path) -> Option<String> {
  let manifest = std::fs::read_to_string(dir.join("package.json")).ok()?;
  let manifest: Value = serde_json::from_str(&manifest).ok()?;
  let declared = manifest
    .get("packageManager")
    .and_then(Value::as_str)
    .or_else(|| manifest.pointer("/devEngines/packageManager/name")?.as_str())?;
  Some(declared.split('@').next()?.to_owned())
}

fn read_user_agent() -> Option<String> {
  let agent = std::env::var("npm_config_user_agent").ok()?;
  Some(agent.split('/').next()?.to_owned())
}

fn find_python_install(dir: &Path) -> &'static str {
  let lockfiles = [
    ("uv.lock", "uv add inboxjot"),
    ("poetry.lock", "poetry add"),
    ("pdm.lock", "pdm add"),
    ("Pipfile", "pipenv install"),
  ];
  match find_in_project(dir, |dir| find_marker(dir, &lockfiles)).as_deref() {
    Some("uv add inboxjot") => "uv add inboxjot",
    Some("poetry add") => "poetry add inboxjot",
    Some("pdm add") => "pdm add inboxjot",
    Some("pipenv install") => "pipenv install inboxjot",
    _ => "pip install inboxjot",
  }
}

fn find_marker(dir: &Path, markers: &[(&str, &str)]) -> Option<String> {
  markers
    .iter()
    .find(|(file, _)| dir.join(file).is_file())
    .map(|(_, value)| (*value).to_owned())
}

fn find_in_project(dir: &Path, find: impl Fn(&Path) -> Option<String>) -> Option<String> {
  for dir in dir.ancestors() {
    if let Some(found) = find(dir) {
      return Some(found);
    }
    if dir.join(".git").exists() {
      break;
    }
  }
  None
}

fn detect_stack(dir: &Path) -> (Stack, Option<&'static str>) {
  let markers = [
    ("package.json", Stack::Node),
    ("pyproject.toml", Stack::Python),
    ("requirements.txt", Stack::Python),
    ("Cargo.toml", Stack::Rust),
    ("go.mod", Stack::Go),
  ];
  markers
    .into_iter()
    .find(|(file, _)| dir.join(file).is_file())
    .map_or((Stack::Curl, None), |(file, stack)| (stack, Some(file)))
}

#[cfg(test)]
mod tests {
  use super::{find_node_install, find_python_install, merge_env};

  #[test]
  fn uses_the_package_manager_of_the_project() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".git")).unwrap();
    let app = root.path().join("apps/web");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("package.json"), "{}").unwrap();
    assert_eq!(find_node_install(&app), "npm install inboxjot");

    std::fs::write(root.path().join("pnpm-lock.yaml"), "").unwrap();
    assert_eq!(find_node_install(&app), "pnpm add inboxjot");

    std::fs::write(app.join("package.json"), r#"{ "packageManager": "bun@1.2.0" }"#)
      .unwrap();
    assert_eq!(find_node_install(&app), "bun add inboxjot");

    std::fs::write(app.join("yarn.lock"), "").unwrap();
    assert_eq!(find_node_install(&app), "yarn add inboxjot");

    let api = root.path().join("apps/api");
    std::fs::create_dir_all(&api).unwrap();
    let manifest = r#"{ "devEngines": { "packageManager": { "name": "deno" } } }"#;
    std::fs::write(api.join("package.json"), manifest).unwrap();
    assert_eq!(find_node_install(&api), "deno add npm:inboxjot");

    assert_eq!(find_python_install(&app), "pip install inboxjot");
    std::fs::write(root.path().join("uv.lock"), "").unwrap();
    assert_eq!(find_python_install(&app), "uv add inboxjot");
  }

  #[test]
  fn replaces_the_key_and_keeps_every_other_line() {
    let existing =
      "# comment\r\nDATABASE_URL=postgres://x\r\nexport INBOXJOT_API_KEY=old\r\nOTHER=1";
    let (content, kept) = merge_env(existing, "ibx_new");

    assert_eq!(
      content,
      "# comment\r\nDATABASE_URL=postgres://x\r\nINBOXJOT_API_KEY=ibx_new\r\nOTHER=1"
    );
    assert_eq!(kept, 2);
  }

  #[test]
  fn appends_the_key_on_a_new_line() {
    assert_eq!(merge_env("", "ibx_new").0, "INBOXJOT_API_KEY=ibx_new\n");
    assert_eq!(merge_env("A=1", "ibx_new").0, "A=1\nINBOXJOT_API_KEY=ibx_new\n");
  }
}
