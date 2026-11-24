mod support;

use serde_json::json;
use wiremock::matchers::{header, header_exists, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use support::{API_KEY, Harness, parse_json};

fn domain() -> serde_json::Value {
  json!({
    "id": "dom_1",
    "name": "acme.com",
    "status": "pending",
    "region": "us-east-1",
    "mail_from_domain": "send.acme.com",
    "sent_last_30_days": 0,
    "verified_at": null,
    "created_at": "2026-11-01T10:00:00Z",
    "records": [
      { "kind": "dkim", "type": "CNAME", "name": "ij1._domainkey.acme.com", "value": "ij1.dkim.example.com", "priority": null, "status": "verified" },
      { "kind": "return_path", "type": "MX", "name": "send.acme.com", "value": "feedback.example.com", "priority": 10, "status": "pending" }
    ]
  })
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_domains_as_tab_separated_rows_when_piped() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .and(query_param("per_page", "50"))
    .and(header("authorization", format!("Bearer {API_KEY}").as_str()))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [domain()], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness.inboxjot().args(["domains", "list"]).output().unwrap();
  let stdout = String::from_utf8(output.stdout).unwrap();

  assert!(output.status.success());
  assert_eq!(
    stdout,
    "Name\tStatus\tRegion\tSent (30d)\tCreated\nacme.com\tpending\tus-east-1\t0\t2026-11-01T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_api_body_with_json() {
  let harness = Harness::start().await;
  let page = json!({ "data": [domain()], "next_cursor": null, "prev_cursor": null });
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(200).set_body_json(&page))
    .mount(&harness.server)
    .await;

  let output = harness.inboxjot().args(["domains", "ls", "--json"]).output().unwrap();

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), page);
}

#[tokio::test(flavor = "multi_thread")]
async fn adds_a_domain_and_prints_its_id_when_piped() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/domains"))
    .and(header_exists("idempotency-key"))
    .respond_with(ResponseTemplate::new(201).set_body_json(domain()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["domains", "add", "acme.com"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "dom_1\n");
  let requests = harness.requests().await;
  assert_eq!(
    requests[0].body_json::<serde_json::Value>().unwrap(),
    json!({ "name": "acme.com" })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolves_a_domain_name_before_inspecting_it() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [domain()], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path("/v1/domains/dom_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(domain()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["domains", "records", "acme.com", "--format", "bind"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "ij1._domainkey.acme.com.\t3600\tIN\tCNAME\tij1.dkim.example.com.\nsend.acme.com.\t3600\tIN\tMX\t10 feedback.example.com.\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_to_delete_without_yes_when_it_cannot_prompt() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains/dom_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(domain()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["domains", "delete", "dom_1", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  let error = parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "confirmation_required");
  assert_eq!(error["error"]["next"], "inboxjot domains delete dom_1 --json --yes");
  assert!(
    harness
      .requests()
      .await
      .iter()
      .all(|request| request.method.as_str() != "DELETE")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn maps_an_unauthorized_answer_to_exit_code_three() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(401).set_body_json(json!({
      "name": "unauthorized", "status": 401, "message": "Unauthorized", "errors": [], "request_id": "req_1"
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["domains", "list", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(3));
  let error = parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "unauthorized");
  assert_eq!(error["error"]["request_id"], "req_1");
  assert_eq!(error["error"]["next"], "inboxjot login");
}

#[tokio::test(flavor = "multi_thread")]
async fn tells_an_agent_to_confirm_with_yes() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [{ "id": "dom_1", "name": "acme.com" }], "next_cursor": null
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path("/v1/domains/dom_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(domain()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .env("CLAUDECODE", "1")
    .args(["domains", "delete", "acme.com"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    parse_json(&output.stdout),
    json!({
      "status": "action_required",
      "reason": "confirmation_required",
      "message": "Delete acme.com? Emails from this domain stop sending. Add --yes to confirm.",
      "next": [{
        "command": "inboxjot domains delete acme.com --yes",
        "description": "Run the command again after the user approves it"
      }]
    })
  );
  assert!(
    harness
      .requests()
      .await
      .iter()
      .all(|request| request.method.as_str() != "DELETE")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_an_api_failure_to_an_agent_as_an_error() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains/dom_404"))
    .respond_with(ResponseTemplate::new(404).set_body_json(json!({
      "name": "not_found", "status": 404, "message": "The domain was not found.", "request_id": "req_2"
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["--non-interactive", "domains", "inspect", "dom_404"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert_eq!(
    parse_json(&output.stdout),
    json!({
      "status": "error",
      "reason": "not_found",
      "message": "The domain was not found.",
      "next": [],
      "request_id": "req_2"
    })
  );
}
