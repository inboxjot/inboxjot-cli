mod support;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::json;
use sha2::Sha256;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, OAUTH_TOKEN, parse_json};

const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
const PAYLOAD: &str =
  r#"{"id":"evt_1","type":"email.sent","created_at":"2026-11-09T10:00:00Z","data":{}}"#;

fn endpoint() -> serde_json::Value {
  json!({
    "id": "whk_1",
    "name": "api.acme.dev",
    "url": "https://api.acme.dev/hooks",
    "events": ["email.sent"],
    "status": "enabled",
    "disabled_reason": null,
    "created_at": "2026-11-01T10:00:00Z",
    "updated_at": "2026-11-01T10:00:00Z"
  })
}

fn sign(id: &str, timestamp: &str, body: &str) -> String {
  let key = STANDARD.decode(SECRET.trim_start_matches("whsec_")).unwrap();
  let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
  mac.update(format!("{id}.{timestamp}.{body}").as_bytes());

  format!("v1,{}", STANDARD.encode(mac.finalize().into_bytes()))
}

fn now() -> String {
  let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
  elapsed.unwrap().as_secs().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_an_endpoint_and_prints_only_the_secret() {
  let harness = Harness::start().await;
  let mut created = endpoint();
  created["secret"] = json!("whsec_abc");
  Mock::given(method("POST"))
    .and(path("/v1/webhooks"))
    .respond_with(ResponseTemplate::new(201).set_body_json(created))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["webhooks", "create", "--url", "https://api.acme.dev/hooks"])
    .args(["--events", "email.sent,email.bounced"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "whsec_abc\n");
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(stderr.contains("Created the webhook endpoint whk_1"));
  assert!(stderr.contains("The signing secret is on stdout. It is shown once."));
  assert!(!stderr.contains("whsec_abc"));

  let requests = harness.requests().await;
  assert_eq!(
    requests[0].body_json::<serde_json::Value>().unwrap(),
    json!({
      "name": "api.acme.dev",
      "url": "https://api.acme.dev/hooks",
      "events": ["email.sent", "email.bounced"]
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_every_event_when_none_are_given() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/webhooks"))
    .respond_with(ResponseTemplate::new(201).set_body_json(endpoint()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["webhooks", "create", "--url", "https://api.acme.dev/hooks", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let body: serde_json::Value = harness.requests().await[0].body_json().unwrap();
  assert_eq!(body["events"].as_array().unwrap().len(), 13);
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_an_unknown_event_before_calling_the_api() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .args(["webhooks", "create", "--url", "https://api.acme.dev/hooks"])
    .args(["--events", "email.sennt"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(stderr.contains("invalid value 'email.sennt'"));
  assert!(stderr.contains("contact.deleted"));
  assert!(harness.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn updates_only_the_given_fields() {
  let harness = Harness::start().await;
  Mock::given(method("PATCH"))
    .and(path("/v1/webhooks/whk_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(endpoint()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["webhooks", "update", "whk_1", "--disable"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let body: serde_json::Value = harness.requests().await[0].body_json().unwrap();
  assert_eq!(body, json!({ "status": "disabled" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_an_endpoint_with_yes() {
  let harness = Harness::start().await;
  Mock::given(method("DELETE"))
    .and(path("/v1/webhooks/whk_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ok" })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["webhooks", "rm", "whk_1", "--yes"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "whk_1\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn verifies_a_signed_payload_from_stdin() {
  let harness = Harness::start().await;
  let timestamp = now();
  let signature = sign("msg_1", &timestamp, PAYLOAD);

  let output = harness
    .inboxjot()
    .env("INBOXJOT_WEBHOOK_SECRET", SECRET)
    .args(["webhooks", "verify", "--id", "msg_1", "--timestamp", &timestamp])
    .args(["-H", &format!("webhook-signature: {signature}"), "--json"])
    .write_stdin(PAYLOAD)
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), json!({ "valid": true }));
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_a_tampered_payload() {
  let harness = Harness::start().await;
  let timestamp = now();
  let signature = sign("msg_1", &timestamp, PAYLOAD);
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("payload.json");
  std::fs::write(&file, PAYLOAD.replace("evt_1", "evt_2")).unwrap();

  let output = harness
    .inboxjot()
    .args(["webhooks", "verify", "--secret", SECRET, "--payload"])
    .arg(&file)
    .args(["--id", "msg_1", "--timestamp", &timestamp, "--signature", &signature])
    .args(["--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert!(output.stdout.is_empty());
  let error = parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "invalid_signature");
  assert!(!String::from_utf8(output.stderr).unwrap().contains(SECRET));
}

fn delivery() -> serde_json::Value {
  json!({
    "delivery_id": "whd_1",
    "event_id": "evt_1",
    "event_type": "email.delivered",
    "endpoint_id": "whk_1",
    "endpoint_name": "api.acme.dev",
    "email_id": "eml_1",
    "status": 500,
    "attempt": 2,
    "duration_ms": 120,
    "created_at": "2026-11-09T10:00:00Z",
    "last_attempt_at": "2026-11-09T10:05:00Z",
    "next_attempt_at": null
  })
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_deliveries_with_filters() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/webhook-deliveries"))
    .and(query_param("endpoint_id", "whk_1"))
    .and(query_param("outcome", "failed"))
    .and(header("authorization", format!("Bearer {OAUTH_TOKEN}").as_str()))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [delivery()], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(["webhooks", "deliveries", "list", "--endpoint", "whk_1", "--status", "failed"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "ID\tEvent\tEndpoint\tStatus\tAttempt\tSent\nwhd_1\temail.delivered\tapi.acme.dev\t500\t2\t2026-11-09T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn replays_a_delivery() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/webhook-deliveries/whd_1/replay"))
    .respond_with(
      ResponseTemplate::new(200).set_body_json(json!({ "delivery_id": "whd_2" })),
    )
    .expect(1)
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["webhooks", "deliveries", "replay", "whd_1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "whd_2\n");
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("Queued a replay of whd_1")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_a_test_event_without_a_body() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/webhooks/whk_1/test"))
    .respond_with(
      ResponseTemplate::new(200).set_body_json(json!({ "delivery_id": "whd_3" })),
    )
    .expect(1)
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["webhooks", "test", "whk_1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "whd_3\n");
  let request = &harness.requests().await[0];
  assert_eq!(request.body_json::<serde_json::Value>().unwrap(), serde_json::Value::Null);
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(stderr.contains("inboxjot webhooks deliveries inspect whd_3"));
}

#[tokio::test(flavor = "multi_thread")]
async fn shows_only_the_secret_on_stdout() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/webhooks/whk_1/secret"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "secret": SECRET })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["webhooks", "secret", "show", "whk_1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{SECRET}\n"));
  assert!(output.stderr.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn rotates_the_secret_only_with_yes() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/webhooks/whk_1/secret"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "secret": SECRET })))
    .expect(1)
    .mount(&harness.server)
    .await;

  let refused = harness
    .inboxjot()
    .args(["webhooks", "secret", "rotate", "whk_1"])
    .output()
    .unwrap();
  assert_eq!(refused.status.code(), Some(2));
  assert!(harness.requests().await.is_empty());

  let rotated = harness
    .inboxjot()
    .args(["webhooks", "secret", "rotate", "whk_1", "--yes"])
    .output()
    .unwrap();
  assert!(rotated.status.success());
  assert_eq!(String::from_utf8(rotated.stdout).unwrap(), format!("{SECRET}\n"));
  assert!(!String::from_utf8(rotated.stderr).unwrap().contains(SECRET));
}
