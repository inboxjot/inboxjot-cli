mod support;

use serde_json::json;

use support::{Harness, parse_json};

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_dashboard_url_of_a_page_and_item() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);

  let output = harness
    .inboxjot()
    .env("INBOXJOT_BROWSER", "true")
    .env("INBOXJOT_DASHBOARD_URL", "https://app.inboxjot.test")
    .args(["open", "api-keys", "api_1"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "https://app.inboxjot.test/acme/api-keys/api_1\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn opens_the_workspace_home_by_default() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);

  let output = harness
    .inboxjot()
    .env("INBOXJOT_BROWSER", "true")
    .args(["open", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    parse_json(&output.stdout),
    json!({ "url": "https://app.inboxjot.com/acme" })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn needs_a_browser_login_to_know_the_workspace() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .env("INBOXJOT_BROWSER", "true")
    .args(["open", "emails", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(3));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "needs_browser_login");
}

#[tokio::test(flavor = "multi_thread")]
async fn uses_the_dashboard_url_saved_at_login() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  let config_path = harness.config_dir.path().join("config.toml");
  let config = std::fs::read_to_string(&config_path).unwrap();
  let config = config.replace(
    "[profiles.default]\n",
    "[profiles.default]\ndashboard_url = \"http://localhost:3000\"\n",
  );
  std::fs::write(&config_path, config).unwrap();

  let output = harness
    .inboxjot()
    .env("INBOXJOT_BROWSER", "true")
    .args(["open", "emails"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "http://localhost:3000/acme/emails\n"
  );
}
