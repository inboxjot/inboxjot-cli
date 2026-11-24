mod support;

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use support::Harness;

fn row(id: &str, time: &str) -> Value {
  json!({
    "request_id": id,
    "method": "POST",
    "path": "/v1/emails",
    "status": 200,
    "summary": "Sent an email",
    "api_key_id": "api_1",
    "api_key_name": "Production",
    "occurred_at": time
  })
}

fn page(rows: &[Value]) -> ResponseTemplate {
  ResponseTemplate::new(200).set_body_json(json!({
    "data": rows, "next_cursor": null, "prev_cursor": null
  }))
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_requests_with_filters() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/logs"))
    .and(query_param("outcome", "failed"))
    .and(query_param("api_key_id", "api_1"))
    .respond_with(page(&[row("req_1", "2026-11-09T10:00:00Z")]))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["logs", "list", "--outcome", "failed", "--api-key", "api_1"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "Request ID\tMethod\tPath\tStatus\tAPI key\tTime\nreq_1\tPOST\t/v1/emails\t200\tProduction\t2026-11-09T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn inspects_a_request_with_its_bodies() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  let mut detail = row("req_1", "2026-11-09T10:00:00Z");
  detail["request_body"] = json!("{\"to\":\"jane@example.com\"}");
  detail["response_body"] = json!("");
  Mock::given(method("GET"))
    .and(path("/v1/logs/req_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(detail))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["logs", "inspect", "req_1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let stdout = String::from_utf8(output.stdout).unwrap();
  assert!(stdout.contains("Request body\n{\n  \"to\": \"jane@example.com\"\n}\n"));
  assert!(!stdout.contains("Response body"));
}

#[tokio::test(flavor = "multi_thread")]
async fn tails_new_requests_once_and_oldest_first() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  let first = row("req_1", "2026-11-09T10:00:00Z");
  let second = row("req_2", "2026-11-09T10:00:01Z");
  let third = row("req_3", "2026-11-09T10:00:02Z");
  Mock::given(method("GET"))
    .and(path("/v1/logs"))
    .respond_with(page(&[second.clone(), first.clone()]))
    .up_to_n_times(1)
    .with_priority(1)
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path("/v1/logs"))
    .respond_with(page(&[third.clone(), second.clone(), first.clone()]))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["logs", "tail", "--timeout", "3s", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  let lines: Vec<Value> = String::from_utf8(output.stdout)
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect();
  assert_eq!(lines, [first, second, third]);
  assert!(harness.requests().await.len() >= 2);
}
