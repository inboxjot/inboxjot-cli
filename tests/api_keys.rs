mod support;

use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, OAUTH_TOKEN, parse_json};

fn key() -> Value {
  json!({
    "id": "api_1",
    "name": "Production",
    "prefix": "ibx",
    "short_token": "ab12cd34",
    "status": "active",
    "permission": "sending",
    "domain_id": null,
    "domain_name": null,
    "last_used_at": null,
    "expires_at": null,
    "created_at": "2026-11-01T10:00:00Z"
  })
}

fn created_key() -> Value {
  let mut key = key();
  key["long_token"] = json!("longsecretlongsecret");
  key
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_keys_with_the_browser_login() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/api-keys"))
    .and(header("authorization", format!("Bearer {OAUTH_TOKEN}").as_str()))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [key()], "next_cursor": null, "prev_cursor": null, "total": 1
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(["api-keys", "ls"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "ID\tName\tToken\tPermission\tDomain\tLast used\tCreated\napi_1\tProduction\tab12cd34\tsending\t\t\t2026-11-01T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refreshes_an_expired_login_before_the_request() {
  let harness = Harness::start().await;
  harness.save_browser_login(0);
  let resource = harness.server.uri().replace(':', "%3A").replace('/', "%2F");
  Mock::given(method("POST"))
    .and(path("/v1/oauth/token"))
    .and(body_string_contains("grant_type=refresh_token"))
    .and(body_string_contains(format!("resource={resource}").as_str()))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "access_token": "oauth_at_fresh",
      "token_type": "Bearer",
      "expires_in": 3600,
      "refresh_token": "oauth_rt_fresh"
    })))
    .expect(1)
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path("/v1/api-keys"))
    .and(header("authorization", "Bearer oauth_at_fresh"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [], "next_cursor": null, "prev_cursor": null, "total": 0
    })))
    .expect(1)
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(["api-keys", "list", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  let saved =
    std::fs::read_to_string(harness.config_dir.path().join("credentials.toml")).unwrap();
  assert!(saved.contains("oauth_at_fresh"));
  assert!(saved.contains("oauth_rt_fresh"));
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_only_the_new_key_on_stdout() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(
      ResponseTemplate::new(200)
        .set_body_json(json!({ "data": [{ "id": "dom_1", "name": "acme.com" }], "next_cursor": null, "prev_cursor": null, "total": 1 })),
    )
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path("/v1/api-keys"))
    .respond_with(ResponseTemplate::new(200).set_body_json(created_key()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api-keys", "create", "--name", "Production", "--permission", "sending"])
    .args(["--domain", "acme.com", "--expires-in", "90d"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "ibx_ab12cd34_longsecretlongsecret\n"
  );
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(stderr.contains("shown only once"));
  assert!(!stderr.contains("longsecret"));

  let requests = harness.requests().await;
  let post = requests
    .iter()
    .find(|request| request.method.as_str() == "POST")
    .unwrap();
  assert_eq!(
    post.body_json::<Value>().unwrap(),
    json!({
      "name": "Production",
      "permission": "sending",
      "domain_id": "dom_1",
      "expires_in": "90d",
      "ip_allowlist": []
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_to_revoke_without_yes() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/api-keys/api_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(key()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api-keys", "revoke", "api_1", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  let error = parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "confirmation_required");
  assert_eq!(error["error"]["next"], "inboxjot api-keys revoke api_1 --json --yes");
  assert!(
    harness
      .requests()
      .await
      .iter()
      .all(|request| request.method.as_str() != "DELETE")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn rotates_a_key_and_prints_the_new_secret() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/api-keys/api_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(key()))
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path("/v1/api-keys/api_1/roll"))
    .respond_with(ResponseTemplate::new(200).set_body_json(created_key()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api-keys", "rotate", "api_1", "--old-key-expires-in", "1h", "--yes"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "ibx_ab12cd34_longsecretlongsecret\n"
  );
  let requests = harness.requests().await;
  let post = requests
    .iter()
    .find(|request| request.method.as_str() == "POST")
    .unwrap();
  assert_eq!(post.body_json::<Value>().unwrap(), json!({ "old_key_expires_in": "1h" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn needs_a_browser_login_to_create_a_key() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .args(["api-keys", "create", "--name", "CI", "--permission", "sending", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(3));
  let error = parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "needs_browser_login");
  assert!(harness.requests().await.is_empty());
}
