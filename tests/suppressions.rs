mod support;

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

fn suppression() -> Value {
  json!({
    "id": "sup_1",
    "email": "jane@example.com",
    "reason": "manual",
    "scope": "all",
    "source": "api",
    "details": null,
    "creator_name": null,
    "source_email_id": null,
    "expires_at": null,
    "created_at": "2026-11-01T10:00:00Z"
  })
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_suppressions_by_reason() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/suppressions"))
    .and(query_param("reason", "manual"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [suppression()], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["suppressions", "ls", "--reason", "manual"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "ID\tEmail\tReason\tScope\tSource\tCreated\nsup_1\tjane@example.com\tmanual\tall\tapi\t2026-11-01T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn adds_addresses_from_arguments_and_stdin() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/suppressions"))
    .respond_with(
      ResponseTemplate::new(200).set_body_json(json!({ "added": 2, "skipped": 1 })),
    )
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["suppressions", "add", "ada@example.com", "-", "--json"])
    .write_stdin("jane@example.com\n\nbob@example.com\n")
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(parse_json(&output.stdout), json!({ "added": 2, "skipped": 1 }));
  let requests = harness.requests().await;
  assert_eq!(
    requests[0].body_json::<Value>().unwrap(),
    json!({ "emails": ["ada@example.com", "jane@example.com", "bob@example.com"] })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn removes_an_address_by_email() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/suppressions"))
    .and(query_param("query", "jane@example.com"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [suppression()], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("DELETE"))
    .and(path("/v1/suppressions/sup_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ok" })))
    .expect(1)
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["suppressions", "rm", "jane@example.com", "--yes"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_to_remove_without_yes() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);

  let output = harness
    .inboxjot()
    .args(["suppressions", "remove", "sup_1", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "confirmation_required");
  assert!(harness.requests().await.is_empty());
}
