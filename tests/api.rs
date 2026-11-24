mod support;

use serde_json::json;
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

#[tokio::test(flavor = "multi_thread")]
async fn gets_a_path_with_its_query_and_prints_the_body_as_is() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .and(query_param("per_page", "5"))
    .and(query_param("status", "verified"))
    .and(header("x-trace", "1"))
    .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[]}"#))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api", "v1/domains?per_page=5", "-X", "GET", "-f", "status=verified"])
    .args(["-H", "X-Trace: 1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "{\"data\":[]}\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn posts_typed_fields_as_json() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/contacts"))
    .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "con_1" })))
    .mount(&harness.server)
    .await;
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("note.txt");
  std::fs::write(&file, "hello").unwrap();

  let output = harness
    .inboxjot()
    .args(["api", "/v1/contacts", "-f", "email=ada@acme.dev", "-f", "zip=01234"])
    .args(["-F", "age=36", "-F", "vip=true", "-F", "plan=null"])
    .arg("-F")
    .arg(format!("note=@{}", file.display()))
    .output()
    .unwrap();

  assert!(output.status.success());
  let body: serde_json::Value = harness.requests().await[0].body_json().unwrap();
  assert_eq!(
    body,
    json!({
      "email": "ada@acme.dev",
      "zip": "01234",
      "age": 36,
      "vip": true,
      "plan": null,
      "note": "hello"
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn merges_every_page_with_paginate() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/topics"))
    .and(query_param_is_missing("after"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [{ "id": "top_1" }], "next_cursor": "c1", "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path("/v1/topics"))
    .and(query_param("after", "c1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [{ "id": "top_2" }], "next_cursor": null, "prev_cursor": "c1"
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api", "/v1/topics", "--paginate", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let page = parse_json(&output.stdout);
  assert_eq!(page["data"], json!([{ "id": "top_1" }, { "id": "top_2" }]));
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_the_error_body_and_exits_with_one() {
  let harness = Harness::start().await;
  let body = json!({
    "name": "not_found", "status": 404, "message": "Domain not found", "errors": [], "request_id": "req_1"
  });
  Mock::given(method("GET"))
    .and(path("/v1/domains/dom_9"))
    .respond_with(ResponseTemplate::new(404).set_body_json(&body))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api", "/v1/domains/dom_9", "-i"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  let stdout = String::from_utf8(output.stdout).unwrap();
  assert!(stdout.starts_with("HTTP 404 Not Found\n"));
  assert!(stdout.ends_with(&format!("\n\n{body}\n")));
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert_eq!(stderr, "Error: Domain not found (HTTP 404)\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn exits_with_three_when_the_key_is_forbidden() {
  let harness = Harness::start().await;
  Mock::given(method("DELETE"))
    .and(path("/v1/domains/dom_1"))
    .respond_with(ResponseTemplate::new(403).set_body_json(json!({
      "name": "forbidden", "status": 403, "message": "Forbidden", "errors": [], "request_id": "req_1"
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api", "-X", "delete", "/v1/domains/dom_1"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(3));
}

#[tokio::test(flavor = "multi_thread")]
async fn keeps_an_encoded_path_as_written() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "con_1" })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api", "/v1/contacts/ada%40x.com?expand=topics"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  let requests = harness.requests().await;
  assert_eq!(requests[0].url.path(), "/v1/contacts/ada%40x.com");
  assert_eq!(requests[0].url.query(), Some("expand=topics"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_a_failed_write_once() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/contacts"))
    .respond_with(ResponseTemplate::new(500).insert_header("retry-after", "0"))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api", "/v1/contacts", "-f", "email=ada@acme.dev"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert_eq!(harness.requests().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn retries_a_rate_limited_write_with_the_same_key() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/contacts"))
    .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
    .up_to_n_times(1)
    .with_priority(1)
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path("/v1/contacts"))
    .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "con_1" })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["api", "/v1/contacts", "-f", "email=ada@acme.dev"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  let requests = harness.requests().await;
  assert_eq!(requests.len(), 2);
  assert_eq!(
    requests[0].headers["idempotency-key"],
    requests[1].headers["idempotency-key"]
  );
}
