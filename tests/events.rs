mod support;

use serde_json::json;
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

#[tokio::test(flavor = "multi_thread")]
async fn tracks_an_event_and_prints_the_contact_id_when_piped() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/events"))
    .and(header_exists("idempotency-key"))
    .respond_with(
      ResponseTemplate::new(202).set_body_json(json!({ "contact_id": "con_1" })),
    )
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["events", "track", "signed_up", "--contact", "ada@acme.dev"])
    .args(["--properties", r#"{"plan":"pro"}"#])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "con_1\n");
  assert_eq!(
    harness.requests().await[0].body_json::<serde_json::Value>().unwrap(),
    json!({
      "id": null,
      "event": "signed_up",
      "email": "ada@acme.dev",
      "user_id": null,
      "occurred_at": null,
      "properties": { "plan": "pro" }
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_data_that_is_not_a_json_object() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .args(["events", "track", "signed_up", "--contact", "ada@acme.dev"])
    .args(["--data", "[1]", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "usage_error");
  assert!(harness.requests().await.is_empty());
}
