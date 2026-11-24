pub mod agent;
pub mod api;
pub mod api_keys;
pub mod auth;
pub mod automations;
pub mod brands;
pub mod campaigns;
pub mod completion;
pub mod contacts;
pub mod doctor;
pub mod domains;
pub mod emails;
pub mod events;
pub mod help;
pub mod images;
pub mod init;
pub mod listen;
pub mod logs;
pub mod open;
pub mod segments;
pub mod stats;
pub mod stream;
pub mod suppressions;
pub mod templates;
pub mod topics;
pub mod trigger;
pub mod webhooks;

use crate::cli::Command;
use crate::context::Context;
use crate::error::Result;

pub async fn run(command: Command, context: &Context) -> Result<()> {
  match command {
    Command::Login(args) => auth::login(args, context).await,
    Command::Logout => auth::logout(context).await,
    Command::Whoami => auth::whoami(context).await,
    Command::Send(args) => emails::send(*args, context).await,
    Command::Emails(command) => emails::run(command, context).await,
    Command::Domains(command) => domains::run(command, context).await,
    Command::Events(command) => events::run(command, context).await,
    Command::Contacts(command) => contacts::run(command, context).await,
    Command::Segments(command) => segments::run(command, context).await,
    Command::Brands(command) => brands::run(command, context).await,
    Command::Topics(command) => topics::run(command, context).await,
    Command::Templates(command) => templates::run(command, context).await,
    Command::Campaigns(command) => campaigns::run(command, context).await,
    Command::Images(command) => images::run(command, context).await,
    Command::Webhooks(command) => webhooks::run(command, context).await,
    Command::Listen(args) => listen::run(args, context).await,
    Command::Trigger(args) => trigger::run(args, context).await,
    Command::Automations(command) => automations::run(command, context).await,
    Command::Suppressions(command) => suppressions::run(command, context).await,
    Command::Stats(args) => stats::run(&args, context).await,
    Command::ApiKeys(command) => api_keys::run(command, context).await,
    Command::Logs(command) => logs::run(command, context).await,
    Command::Init(args) => init::run(&args, context).await,
    Command::Open(args) => open::run(&args, context),
    Command::Api(args) => api::run(args, context).await,
    Command::Doctor => doctor::run(context).await,
    Command::Agent(command) => agent::run(command, context),
    Command::Completion(args) => completion::run(&args),
    Command::Help(args) => help::run(&args, context),
  }
}
