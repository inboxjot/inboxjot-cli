mod support;

use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, OAUTH_TOKEN, parse_json};

fn campaign() -> serde_json::Value {
  json!({
    "id": "cpg_1",
    "name": "October update",
    "status": "draft",
    "from": "Acme <hello@acme.dev>",
    "reply_to": null,
    "subject": "What is new",
    "preheader": "",
    "html": "<p>Hi</p>",
    "text": "Hi",
    "template_id": null,
    "template": null,
    "include_segment_ids": ["seg_1"],
    "include_segments": [{ "id": "seg_1", "name": "Customers" }],
    "exclude_segment_ids": [],
    "exclude_segments": [],
    "topic_id": "top_1",
    "topic": { "id": "top_1", "name": "Product updates" },
    "audience_count": 1204,
    "queued_count": 0,
    "skipped_count": 0,
    "paused_reason": null,
    "failure_reason": null,
    "scheduled_at": null,
    "started_at": null,
    "completed_at": null,
    "warmup_next_limit": null,
    "created_at": "2026-11-01T10:00:00Z",
    "updated_at": "2026-11-01T10:00:00Z"
  })
}

async fn mount_campaign(harness: &Harness) {
  Mock::given(method("GET"))
    .and(path("/v1/campaigns/cpg_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(campaign()))
    .mount(&harness.server)
    .await;
}

async fn mount_send(harness: &Harness) {
  Mock::given(method("POST"))
    .and(path("/v1/campaigns/cpg_1/send"))
    .respond_with(ResponseTemplate::new(202).set_body_json(campaign()))
    .mount(&harness.server)
    .await;
}

async fn get_post_bodies(harness: &Harness) -> Vec<serde_json::Value> {
  harness
    .requests()
    .await
    .iter()
    .filter(|request| request.method.as_str() != "GET")
    .map(|request| request.body_json().unwrap())
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_list_body_with_json() {
  let harness = Harness::start().await;
  let page = json!({ "data": [campaign()], "next_cursor": null, "prev_cursor": null });
  Mock::given(method("GET"))
    .and(path("/v1/campaigns"))
    .and(query_param("status", "draft"))
    .respond_with(ResponseTemplate::new(200).set_body_json(&page))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["campaigns", "list", "--status", "draft", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), page);
}

#[tokio::test(flavor = "multi_thread")]
async fn shows_the_audience_when_inspecting() {
  let harness = Harness::start().await;
  mount_campaign(&harness).await;

  let output = harness
    .inboxjot()
    .args(["campaigns", "inspect", "cpg_1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert!(
    String::from_utf8(output.stdout)
      .unwrap()
      .contains("Audience        Customers on Product updates (1,204 contacts)")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_a_campaign_with_segments() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/campaigns"))
    .respond_with(ResponseTemplate::new(201).set_body_json(campaign()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["campaigns", "create", "--name", "October update", "--subject", "Hi"])
    .args(["--segment", "seg_1,seg_2", "--exclude-segment", "seg_3", "--topic", "top_1"])
    .args(["--template", "tpl_1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "cpg_1\n");
  assert_eq!(
    get_post_bodies(&harness).await,
    [json!({
      "name": "October update",
      "subject": "Hi",
      "template_id": "tpl_1",
      "include_segment_ids": ["seg_1", "seg_2"],
      "exclude_segment_ids": ["seg_3"],
      "topic_id": "top_1"
    })]
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_a_test_to_each_address() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/campaigns/cpg_1/test"))
    .respond_with(
      ResponseTemplate::new(202).set_body_json(json!({ "ids": ["eml_1", "eml_2"] })),
    )
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["campaigns", "test", "cpg_1", "--to", "riti@acme.dev,kamran@acme.dev"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "eml_1\neml_2\n");
  assert_eq!(
    get_post_bodies(&harness).await,
    [json!({ "to": ["riti@acme.dev", "kamran@acme.dev"] })]
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_to_send_without_yes() {
  let harness = Harness::start().await;
  mount_campaign(&harness).await;
  mount_send(&harness).await;

  let output = harness
    .inboxjot()
    .args(["campaigns", "send", "cpg_1", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  let error = parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "confirmation_required");
  assert_eq!(
    error["error"]["message"],
    "This sends the campaign October update. Add --yes to confirm."
  );
  assert_eq!(error["error"]["next"], "inboxjot campaigns send cpg_1 --json --yes");
  assert!(get_post_bodies(&harness).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_now_with_yes() {
  let harness = Harness::start().await;
  mount_campaign(&harness).await;
  mount_send(&harness).await;

  let output = harness
    .inboxjot()
    .args(["campaigns", "send", "cpg_1", "--yes"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "cpg_1\n");
  assert_eq!(get_post_bodies(&harness).await, [json!({})]);
}

#[tokio::test(flavor = "multi_thread")]
async fn schedules_a_send_with_at() {
  let harness = Harness::start().await;
  mount_campaign(&harness).await;
  mount_send(&harness).await;

  let output = harness
    .inboxjot()
    .args(["campaigns", "send", "cpg_1", "--at", "2026-11-20T09:00:00Z", "--yes"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    get_post_bodies(&harness).await,
    [json!({ "scheduled_at": "2026-11-20T09:00:00Z" })]
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancels_with_yes_and_refuses_without() {
  let harness = Harness::start().await;
  mount_campaign(&harness).await;
  Mock::given(method("POST"))
    .and(path("/v1/campaigns/cpg_1/cancel"))
    .respond_with(ResponseTemplate::new(200).set_body_json(campaign()))
    .expect(1)
    .mount(&harness.server)
    .await;

  let refused = harness
    .inboxjot()
    .args(["campaigns", "cancel", "cpg_1"])
    .output()
    .unwrap();
  let canceled = harness
    .inboxjot()
    .args(["campaigns", "cancel", "cpg_1", "--yes"])
    .output()
    .unwrap();

  assert_eq!(refused.status.code(), Some(2));
  assert!(canceled.status.success());
  assert!(
    String::from_utf8(canceled.stderr)
      .unwrap()
      .contains("Canceled October update")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn restores_a_version() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/campaigns/cpg_1/versions/ver_1/restore"))
    .respond_with(ResponseTemplate::new(200).set_body_json(campaign()))
    .expect(1)
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["campaigns", "versions", "restore", "cpg_1", "ver_1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "cpg_1\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn pauses_a_campaign_with_the_browser_login() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/campaigns/cpg_1/pause"))
    .and(header("authorization", format!("Bearer {OAUTH_TOKEN}").as_str()))
    .respond_with(ResponseTemplate::new(200).set_body_json(campaign()))
    .expect(1)
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(["campaigns", "pause", "cpg_1"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "cpg_1\n");
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("Paused October update")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn resumes_only_with_yes() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/campaigns/cpg_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(campaign()))
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path("/v1/campaigns/cpg_1/resume"))
    .respond_with(ResponseTemplate::new(200).set_body_json(campaign()))
    .expect(1)
    .mount(&harness.server)
    .await;

  let refused = harness
    .inboxjot()
    .args(["campaigns", "resume", "cpg_1", "--json"])
    .output()
    .unwrap();
  let resumed = harness
    .inboxjot()
    .args(["campaigns", "resume", "cpg_1", "--yes"])
    .output()
    .unwrap();

  assert_eq!(refused.status.code(), Some(2));
  let error = parse_json(&refused.stderr);
  assert_eq!(error["error"]["name"], "confirmation_required");
  assert!(
    error["error"]["message"]
      .as_str()
      .unwrap()
      .starts_with("Resume sending the campaign October update?")
  );
  assert!(resumed.status.success());
  assert!(
    String::from_utf8(resumed.stderr)
      .unwrap()
      .contains("Resumed October update")
  );
}
