mod support;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use support::Harness;

#[tokio::test(flavor = "multi_thread")]
async fn sends_to_the_bounce_address_from_the_verified_domain() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [
        { "id": "dom_1", "name": "draft.dev", "status": "pending" },
        { "id": "dom_2", "name": "acme.com", "status": "verified" }
      ],
      "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path("/v1/emails"))
    .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "eml_1" })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["trigger", "email.bounced"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "eml_1\n");
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("Sent a test email to bounced@inboxjot.dev")
  );
  let requests = harness.requests().await;
  let body: Value = requests[1].body_json().unwrap();
  assert_eq!(body["from"], "trigger@acme.com");
  assert_eq!(body["to"], json!(["bounced@inboxjot.dev"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn fails_without_a_verified_domain() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["trigger", "email.delivered", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  let error = support::parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "no_verified_domain");
  assert_eq!(error["error"]["next"], "inboxjot domains add <domain>");
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_and_then_deletes_a_test_contact() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/contacts"))
    .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "con_1" })))
    .mount(&harness.server)
    .await;
  Mock::given(method("DELETE"))
    .and(path("/v1/contacts/con_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "deleted": true })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["trigger", "contact.deleted"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "con_1\n");
  let requests = harness.requests().await;
  assert_eq!(requests.len(), 2);
  let body: Value = requests[0].body_json().unwrap();
  let email = body["email"].as_str().unwrap();
  assert!(email.starts_with("trigger+") && email.ends_with("@inboxjot.dev"));
  assert_eq!(requests[1].method.as_str(), "DELETE");
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_an_event_it_cannot_cause() {
  let harness = Harness::start().await;

  let output = harness.inboxjot().args(["trigger", "email.opened"]).output().unwrap();

  assert_eq!(output.status.code(), Some(2));
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(stderr.contains("contact.deleted"));
  assert!(harness.requests().await.is_empty());
}
