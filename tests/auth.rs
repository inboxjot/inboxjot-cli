mod support;

use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, OAUTH_TOKEN, WORKSPACE};

#[tokio::test(flavor = "multi_thread")]
async fn shows_the_workspace_of_a_browser_login() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(["whoami", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  let body = support::parse_json(&output.stdout);
  assert_eq!(body["workspace"], WORKSPACE);
  assert_eq!(body["login"], "browser");
  assert_eq!(body["access"], "Custom (domains:read)");
}

#[tokio::test(flavor = "multi_thread")]
async fn says_when_the_server_no_longer_accepts_the_login() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(401).set_body_json(json!({
      "name": "unauthorized",
      "status": 401,
      "message": "Unauthorized",
      "errors": [],
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(["whoami", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  let body = support::parse_json(&output.stdout);
  assert_eq!(body["status"], "revoked");
  assert_eq!(body["workspace"], WORKSPACE);
}

#[tokio::test(flavor = "multi_thread")]
async fn revokes_the_browser_login_on_logout() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/oauth/revoke"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&harness.server)
    .await;

  let output = harness.inboxjot().args(["logout"]).output().unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  let requests = harness.requests().await;
  let revoke = requests
    .iter()
    .find(|request| request.url.path() == "/v1/oauth/revoke");
  let body = revoke.map(|request| String::from_utf8_lossy(&request.body).into_owned());
  assert!(
    body
      .as_deref()
      .is_some_and(|body| body.contains("token=oauth_rt_testrefresh")),
    "{body:?}"
  );
  assert!(requests.iter().all(|request| request.method.as_str() != "DELETE"));
}

#[tokio::test(flavor = "multi_thread")]
async fn uses_the_browser_login_for_public_commands() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .and(header("authorization", format!("Bearer {OAUTH_TOKEN}").as_str()))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(["domains", "list", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[tokio::test(flavor = "multi_thread")]
async fn tells_an_agent_to_log_in() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .env("CLAUDECODE", "1")
    .args(["domains", "list"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(3));
  assert_eq!(
    support::parse_json(&output.stdout),
    json!({
      "status": "action_required",
      "reason": "not_logged_in",
      "message": "You are not logged in.",
      "next": [{ "command": "inboxjot login", "description": "Log in with the browser" }]
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn keeps_the_saved_api_url_on_a_second_login() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
    .mount(&harness.server)
    .await;
  let first = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(["login", "--api-key", "-"])
    .write_stdin(support::API_KEY)
    .output()
    .unwrap();
  assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));

  let second = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .env_remove("INBOXJOT_API_URL")
    .env("INBOXJOT_DASHBOARD_URL", "http://localhost:3000")
    .args(["login", "--api-key", "-"])
    .write_stdin(support::API_KEY)
    .output()
    .unwrap();
  assert!(second.status.success(), "{}", String::from_utf8_lossy(&second.stderr));

  let config =
    std::fs::read_to_string(harness.config_dir.path().join("config.toml")).unwrap();
  assert!(
    config.contains(&format!("api_url = \"{}\"", harness.server.uri())),
    "{config}"
  );
  assert!(config.contains("dashboard_url = \"http://localhost:3000\""), "{config}");
  assert_eq!(harness.requests().await.len(), 2);
}
