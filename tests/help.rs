mod support;

use serde_json::Value;

use support::{Harness, parse_json};

async fn run(args: &[&str]) -> std::process::Output {
  let harness = Harness::start().await;
  harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(args)
    .output()
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn searches_commands_offline() {
  let output = run(&["help", "search", "webhook"]).await;

  assert!(output.status.success());
  let stdout = String::from_utf8(output.stdout).unwrap();
  assert!(stdout.lines().count() <= 10);
  assert!(
    stdout
      .lines()
      .any(|line| line.starts_with("inboxjot webhooks verify "))
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn says_when_no_command_matches() {
  let output = run(&["help", "search", "zebra"]).await;

  assert!(output.status.success());
  assert!(output.stdout.is_empty());
  assert_eq!(String::from_utf8(output.stderr).unwrap(), "No commands match \"zebra\".\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn names_the_unknown_segment() {
  let output = run(&["help", "domains", "zap"]).await;

  assert_eq!(output.status.code(), Some(2));
  assert!(String::from_utf8(output.stderr).unwrap().contains("\"zap\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_long_help_for_a_command_path() {
  let output = run(&["help", "webhooks", "ls"]).await;

  assert!(output.status.success());
  let stdout = String::from_utf8(output.stdout).unwrap();
  assert!(stdout.contains("Usage: inboxjot webhooks list"));
}

#[tokio::test(flavor = "multi_thread")]
async fn describes_a_command_with_help_and_json() {
  let output = run(&["webhooks", "create", "--help", "--json"]).await;

  assert!(output.status.success());
  let help = parse_json(&output.stdout);
  assert_eq!(help["path"], "inboxjot webhooks create");
  assert!(help["summary"].is_string());
  assert!(help["subcommands"].as_array().unwrap().is_empty());
  let url = help["options"]
    .as_array()
    .unwrap()
    .iter()
    .find(|option| option["long"] == "url")
    .unwrap();
  assert_eq!(url["value_name"], "URL");
  for key in ["short", "default", "possible_values", "help"] {
    assert!(url.get(key).is_some(), "missing {key}");
  }

  let domains = parse_json(&run(&["domains", "-h", "--json"]).await.stdout);
  let names: Vec<&Value> = domains["subcommands"]
    .as_array()
    .unwrap()
    .iter()
    .map(|sub| &sub["name"])
    .collect();
  assert!(names.contains(&&Value::from("add")));
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_whole_tree_as_json() {
  let output = run(&["help", "--all", "--json"]).await;

  assert!(output.status.success());
  let tree = parse_json(&output.stdout);
  let paths: Vec<&str> = tree["commands"]
    .as_array()
    .unwrap()
    .iter()
    .filter_map(|c| c["path"].as_str())
    .collect();
  assert_eq!(paths[0], "inboxjot");
  assert!(paths.contains(&"inboxjot webhooks verify"));
  assert!(paths.contains(&"inboxjot doctor"));
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_a_zsh_completion_script() {
  let output = run(&["completion", "zsh"]).await;

  assert!(output.status.success());
  let stdout = String::from_utf8(output.stdout).unwrap();
  assert!(stdout.starts_with("#compdef inboxjot"));
}

#[tokio::test(flavor = "multi_thread")]
async fn describes_a_command_after_global_options() {
  let output = run(&["--non-interactive", "domains", "add", "--help", "--json"]).await;

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout)["path"], "inboxjot domains add");
}

#[tokio::test(flavor = "multi_thread")]
async fn names_the_missing_input_for_an_agent() {
  let output = run(&["--non-interactive", "domains", "add"]).await;

  assert_eq!(output.status.code(), Some(2));
  let outcome = parse_json(&output.stdout);
  assert_eq!(outcome["status"], "action_required");
  assert_eq!(outcome["reason"], "missing_input");
  assert_eq!(outcome["next"][0]["command"], "inboxjot domains add --help");
  assert!(outcome["errors"][0]["message"].as_str().unwrap().contains("<DOMAIN>"));
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_json_after_an_invalid_value() {
  let output = run(&["trigger", "email.bad", "--json"]).await;

  assert_eq!(output.status.code(), Some(2));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "usage_error");
}
