mod support;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

fn report() -> Value {
  json!({
    "from": "2026-11-02",
    "to": "2026-11-09",
    "tz": "UTC",
    "stats": [],
    "total": {
      "sent": 1000, "delivered": 980, "opened": 490, "clicked": 50,
      "bounced": 20, "soft_bounced": 0, "complained": 1, "unsubscribed": 3
    },
    "previous": null,
    "domains": [],
    "links": [],
    "sources": {},
    "refreshed_at": "2026-11-09T10:00:00Z"
  })
}

#[tokio::test(flavor = "multi_thread")]
async fn asks_for_the_days_since_the_start_and_prints_rates() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/stats"))
    .respond_with(ResponseTemplate::new(200).set_body_json(report()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["stats", "--since", "2026-11-02"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  let stdout = String::from_utf8(output.stdout).unwrap();
  assert!(stdout.contains("Period          2026-11-02 to 2026-11-09\n"));
  assert!(stdout.contains("Delivered       980  98.0%\n"));
  assert!(stdout.contains("Opened          490  50.0%\n"));

  let requests = harness.requests().await;
  let query: Vec<(String, String)> = requests[0]
    .url
    .query_pairs()
    .map(|(key, value)| (key.into_owned(), value.into_owned()))
    .collect();
  assert!(query.contains(&("from".to_owned(), "2026-11-02".to_owned())));
  assert!(query.iter().any(|(key, _)| key == "to"));
  assert!(query.iter().any(|(key, _)| key == "tz"));
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_report_with_json() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/stats"))
    .respond_with(ResponseTemplate::new(200).set_body_json(report()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["stats", "--since", "7d", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(parse_json(&output.stdout), report());
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_a_start_that_is_not_a_duration() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);

  let output = harness
    .inboxjot()
    .args(["stats", "--since", "soon"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert!(harness.requests().await.is_empty());
}
