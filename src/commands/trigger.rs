use clap::{Args, ValueEnum};
use serde_json::{Value, json};

use crate::api::{Api, Page};
use crate::context::Context;
use crate::error::{Error, Result};

const TEST_DOMAIN: &str = "inboxjot.dev";

#[derive(Debug, Args)]
pub struct TriggerArgs {
  #[arg(value_name = "EVENT", value_enum, help = "Event type to cause")]
  pub event: TriggerEvent,

  #[arg(
    long,
    help = "Sender address; the default is trigger@ on the first verified domain"
  )]
  pub from: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum TriggerEvent {
  #[value(name = "email.sent")]
  EmailSent,
  #[value(name = "email.delivered")]
  EmailDelivered,
  #[value(name = "email.bounced")]
  EmailBounced,
  #[value(name = "email.complained")]
  EmailComplained,
  #[value(name = "contact.created")]
  ContactCreated,
  #[value(name = "contact.updated")]
  ContactUpdated,
  #[value(name = "contact.deleted")]
  ContactDeleted,
}

struct Triggered {
  id: String,
  headline: String,
  detail: String,
  note: &'static str,
}

pub async fn run(args: TriggerArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Sending a test event…");
  let event = args
    .event
    .to_possible_value()
    .map(|value| value.get_name().to_owned())
    .unwrap_or_default();
  let triggered = match args.event {
    TriggerEvent::ContactCreated
    | TriggerEvent::ContactUpdated
    | TriggerEvent::ContactDeleted => trigger_contact(&api, args.event).await?,
    TriggerEvent::EmailSent
    | TriggerEvent::EmailDelivered
    | TriggerEvent::EmailBounced
    | TriggerEvent::EmailComplained => {
      trigger_email(&api, args.event, &event, args.from).await?
    }
  };

  if context.output.is_json() {
    return context
      .output
      .print_json(&json!({ "type": event, "id": triggered.id }));
  }
  context.output.done(&triggered.headline, &triggered.detail);
  context.output.info(triggered.note);
  if !context.output.is_stdout_tty() {
    context.output.print_line(&triggered.id);
  }
  context
    .output
    .print_next(&[("see the events arrive", "inboxjot listen")]);
  Ok(())
}

async fn trigger_email(
  api: &Api,
  event: TriggerEvent,
  name: &str,
  from: Option<String>,
) -> Result<Triggered> {
  let (mailbox, note) = match event {
    TriggerEvent::EmailBounced => {
      ("bounced", "It bounces in a moment and sends email.sent, then email.bounced.")
    }
    TriggerEvent::EmailComplained => (
      "complained",
      "It is marked as spam in a moment and sends email.sent, email.delivered, then email.complained.",
    ),
    _ => (
      "delivered",
      "It is delivered in a moment and sends email.sent, then email.delivered.",
    ),
  };
  let from = match from {
    Some(from) => from,
    None => format!("trigger@{}", find_verified_domain(api).await?),
  };
  let to = format!("{mailbox}@{TEST_DOMAIN}");
  let body = json!({
    "from": from,
    "to": [to],
    "subject": "Test event from inboxjot trigger",
    "text": format!("This email causes an {name} event."),
  });

  let email = api.post(&["v1", "emails"], &body).await?;
  Ok(Triggered {
    id: get_id(&email),
    headline: format!("Sent a test email to {to}"),
    detail: format!("from {from}"),
    note,
  })
}

async fn trigger_contact(api: &Api, event: TriggerEvent) -> Result<Triggered> {
  let label = &uuid::Uuid::new_v4().simple().to_string()[..8];
  let body = json!({ "email": format!("trigger+{label}@{TEST_DOMAIN}") });
  let contact = api.post(&["v1", "contacts"], &body).await?;
  let id = get_id(&contact);

  let email = get_text(&contact, "email");
  let (headline, note) = match event {
    TriggerEvent::ContactUpdated => {
      let body = json!({ "first_name": "Trigger" });
      api.patch(&["v1", "contacts", &id], &body).await?;
      (
        format!("Created and updated the test contact {email}"),
        "It sends contact.created, then contact.updated.",
      )
    }
    TriggerEvent::ContactDeleted => {
      api.delete(&["v1", "contacts", &id]).await?;
      (
        format!("Created and deleted the test contact {email}"),
        "It sends contact.created, then contact.deleted.",
      )
    }
    _ => (format!("Created the test contact {email}"), "It sends contact.created."),
  };

  Ok(Triggered { id, headline, detail: String::new(), note })
}

pub async fn find_verified_domain(api: &Api) -> Result<String> {
  let page = api
    .list(&["v1", "domains"], &(), &Page { all: true, ..Page::default() })
    .await?;

  page
    .get("data")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .find(|domain| domain.get("status").and_then(Value::as_str) == Some("verified"))
    .and_then(|domain| domain.get("name").and_then(Value::as_str))
    .map(str::to_owned)
    .ok_or_else(|| {
      Error::failed("no_verified_domain", "No domain is verified yet.")
        .with_next("inboxjot domains add <domain>", "Add a sending domain")
    })
}

fn get_text(value: &Value, key: &str) -> String {
  value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn get_id(value: &Value) -> String {
  value.get("id").and_then(Value::as_str).unwrap_or_default().to_owned()
}
