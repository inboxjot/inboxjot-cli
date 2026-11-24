mod support;

use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

fn project() -> TempDir {
  let dir = tempfile::Builder::new().prefix("shop").tempdir().unwrap();
  std::fs::write(dir.path().join("package.json"), "{}").unwrap();
  dir
}

async fn mount_create_key(harness: &Harness) {
  Mock::given(method("POST"))
    .and(path("/v1/api-keys"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "id": "api_9",
      "name": "shop (inboxjot init)",
      "prefix": "ibx",
      "short_token": "ab12cd34",
      "long_token": "longsecret"
    })))
    .mount(&harness.server)
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn merges_the_key_into_env_local_and_keeps_other_lines() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  mount_create_key(&harness).await;
  let dir = project();
  let env = "# Local secrets\nDATABASE_URL=postgres://localhost/shop\n\nINBOXJOT_API_KEY=ibx_old_old\nSTRIPE_KEY=sk_test\n";
  std::fs::write(dir.path().join(".env.local"), env).unwrap();
  std::fs::write(dir.path().join(".gitignore"), "node_modules").unwrap();

  let output = harness
    .inboxjot()
    .args(["init", "--yes", "--json", "--dir"])
    .arg(dir.path())
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  let result = parse_json(&output.stdout);
  assert_eq!(result["key_id"], "api_9");
  assert_eq!(result["stack"], "node");
  assert_eq!(
    std::fs::read_to_string(dir.path().join(".env.local")).unwrap(),
    "# Local secrets\nDATABASE_URL=postgres://localhost/shop\n\nINBOXJOT_API_KEY=ibx_ab12cd34_longsecret\nSTRIPE_KEY=sk_test\n"
  );
  assert_eq!(
    std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
    "node_modules\n.env.local\n"
  );

  let requests = harness.requests().await;
  let body = requests[0].body_json::<Value>().unwrap();
  assert_eq!(body["permission"], "sending");
  assert!(body["name"].as_str().unwrap().ends_with(" (inboxjot init)"));
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_env_local_and_prints_next_steps() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  mount_create_key(&harness).await;
  let dir = project();

  let output = harness
    .inboxjot()
    .args(["init", "--dir"])
    .arg(dir.path())
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "api_9\n");
  assert_eq!(
    std::fs::read_to_string(dir.path().join(".env.local")).unwrap(),
    "INBOXJOT_API_KEY=ibx_ab12cd34_longsecret\n"
  );
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(!stderr.contains("Kept"));
  assert!(stderr.contains("npm install inboxjot"));
  assert!(stderr.contains("inboxjot.emails.send"));
  assert!(!stderr.contains("longsecret"));
  assert!(!dir.path().join(".gitignore").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn asks_before_replacing_a_saved_key() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  let dir = project();
  std::fs::write(dir.path().join(".env.local"), "INBOXJOT_API_KEY=ibx_old_old\n")
    .unwrap();

  let output = harness
    .inboxjot()
    .args(["init", "--json", "--dir"])
    .arg(dir.path())
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "confirmation_required");
  assert!(harness.requests().await.is_empty());
  assert_eq!(
    std::fs::read_to_string(dir.path().join(".env.local")).unwrap(),
    "INBOXJOT_API_KEY=ibx_old_old\n"
  );
}
