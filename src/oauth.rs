use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::get;
use oauth2::basic::{
  BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
  BasicTokenType,
};
use oauth2::{
  AuthUrl, AuthorizationCode, Client, ClientId, CsrfToken, EndpointNotSet,
  ExtraTokenFields, PkceCodeChallenge, RedirectUrl, RefreshToken, RequestTokenError,
  Scope, StandardRevocableToken, StandardTokenResponse, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::api::CLI_USER_AGENT;
use crate::credentials::OAuthTokens;
use crate::error::{Error, Result};
use crate::login_page::{Access, LoginOutcome, render_login_page};

pub const CLIENT_ID: &str = "inboxjot-cli";

const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

type CliClient = Client<
  BasicErrorResponse,
  StandardTokenResponse<WorkspaceFields, BasicTokenType>,
  BasicTokenIntrospectionResponse,
  StandardRevocableToken,
  BasicRevocationErrorResponse,
  EndpointNotSet,
  EndpointNotSet,
  EndpointNotSet,
  EndpointNotSet,
  EndpointNotSet,
>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Discovery {
  pub authorization_endpoint: String,
  pub token_endpoint: String,
  pub revocation_endpoint: String,
  #[serde(default)]
  pub scopes_supported: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WorkspaceFields {
  #[serde(default)]
  pub workspace_slug: Option<String>,
  #[serde(default)]
  pub workspace_name: Option<String>,
  #[serde(default)]
  pub user_name: Option<String>,
  #[serde(default)]
  pub user_email: Option<String>,
  #[serde(default)]
  pub refresh_token_expires_in: Option<i64>,
}

impl ExtraTokenFields for WorkspaceFields {}

#[derive(Debug)]
pub struct Grant {
  pub tokens: OAuthTokens,
  pub workspace_slug: Option<String>,
  pub workspace_name: Option<String>,
  pub user_name: Option<String>,
  pub user_email: Option<String>,
}

pub fn build_http() -> Result<reqwest::Client> {
  Ok(
    reqwest::Client::builder()
      .user_agent(CLI_USER_AGENT)
      .redirect(reqwest::redirect::Policy::none())
      .timeout(Duration::from_secs(30))
      .tls_certs_merge(crate::api::get_fallback_roots())
      .build()?,
  )
}

pub async fn read_discovery(http: &reqwest::Client, api_url: &str) -> Result<Discovery> {
  let url =
    format!("{}/.well-known/oauth-authorization-server", api_url.trim_end_matches('/'));
  let response = http.get(&url).send().await?.error_for_status()?;

  Ok(response.json().await?)
}

pub fn get_device_name() -> String {
  let host = gethostname::gethostname().to_string_lossy().into_owned();
  host.trim_end_matches(".local").to_owned()
}

const CODE_LETTERS: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

pub fn create_login_code() -> Result<String> {
  let mut bytes = [0_u8; 8];
  getrandom::fill(&mut bytes).map_err(|error| {
    Error::failed("random_error", format!("Could not create a login code: {error}"))
  })?;
  let letters: String = bytes
    .iter()
    .map(|byte| char::from(CODE_LETTERS[usize::from(*byte) % CODE_LETTERS.len()]))
    .collect();
  Ok(format!("{}-{}", &letters[..4], &letters[4..]))
}

pub async fn authorize(
  http: &reqwest::Client,
  discovery: &Discovery,
  api_url: &str,
  login_code: &str,
  open_url: impl FnOnce(&str),
) -> Result<Grant> {
  let listener = TcpListener::bind("127.0.0.1:0").await?;
  let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr()?.port());

  let client = CliClient::new(ClientId::new(CLIENT_ID.to_owned()))
    .set_auth_uri(parse_url(AuthUrl::new(discovery.authorization_endpoint.clone()))?)
    .set_token_uri(parse_url(TokenUrl::new(discovery.token_endpoint.clone()))?)
    .set_redirect_uri(parse_url(RedirectUrl::new(redirect))?);

  let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
  let scopes = discovery.scopes_supported.iter().cloned().map(Scope::new);
  let (url, state) = client
    .authorize_url(CsrfToken::new_random)
    .add_scopes(scopes)
    .add_extra_param("resource", api_url.trim_end_matches('/'))
    .add_extra_param("device_name", get_device_name())
    .add_extra_param("login_code", login_code)
    .set_pkce_challenge(challenge)
    .url();

  open_url(url.as_str());
  let callback = wait_for_callback(listener, state.secret().clone()).await?;
  let result: Result<Grant> = async {
    let code = read_login_code(&callback.params, &state)?;
    let send = |request| send_oauth_request(http, request);
    let response = client
      .exchange_code(AuthorizationCode::new(code))
      .set_pkce_verifier(verifier)
      .add_extra_param("resource", api_url.trim_end_matches('/'))
      .request_async(&send)
      .await
      .map_err(get_token_error)?;

    Ok(to_grant(CLIENT_ID, &discovery.scopes_supported, &response))
  }
  .await;

  let device = get_device_name();
  let outcome = match &result {
    Ok(grant) => LoginOutcome::LoggedIn {
      device: &device,
      workspace: grant
        .workspace_name
        .as_deref()
        .or(grant.workspace_slug.as_deref())
        .unwrap_or("your workspace"),
      access: Access::read(&grant.tokens),
    },
    Err(error) if error.name == "login_denied" => LoginOutcome::Canceled,
    Err(error) => LoginOutcome::Failed { reason: &error.message },
  };
  callback.respond(render_login_page(&outcome)).await;

  result
}

fn read_login_code(
  params: &HashMap<String, String>,
  state: &CsrfToken,
) -> Result<String> {
  if params.get("state").map(String::as_str) != Some(state.secret().as_str()) {
    return Err(Error::auth("login_failed", "The browser answered for another login."));
  }

  if let Some(error) = params.get("error") {
    if error == "access_denied" {
      return Err(Error::auth("login_denied", "The login was canceled in the browser."));
    }
    let description =
      params.get("error_description").map_or(error.as_str(), String::as_str);
    return Err(Error::auth(
      "login_failed",
      format!("The login did not finish: {description}."),
    ));
  }

  params
    .get("code")
    .cloned()
    .ok_or_else(|| Error::auth("login_failed", "The browser did not send a login code."))
}

pub async fn refresh(
  http: &reqwest::Client,
  token_endpoint: &str,
  api_url: &str,
  tokens: &OAuthTokens,
) -> Result<Grant> {
  let client = CliClient::new(ClientId::new(tokens.client_id.clone()))
    .set_token_uri(parse_url(TokenUrl::new(token_endpoint.to_owned()))?);

  let send = |request| send_oauth_request(http, request);
  let refresh_token = RefreshToken::new(tokens.refresh_token.clone());
  let response = client
    .exchange_refresh_token(&refresh_token)
    .add_extra_param("resource", api_url.trim_end_matches('/'))
    .request_async(&send)
    .await
    .map_err(|error| match error {
      RequestTokenError::ServerResponse(_) => {
        Error::auth("login_expired", "Your login expired.").explain_lost_login()
      }
      error => get_token_error(error),
    })?;

  Ok(to_grant(&tokens.client_id, &tokens.scopes_supported, &response))
}

pub async fn revoke(
  http: &reqwest::Client,
  revocation_endpoint: &str,
  tokens: &OAuthTokens,
) -> Result<()> {
  let form = [
    ("token", tokens.refresh_token.as_str()),
    ("token_type_hint", "refresh_token"),
    ("client_id", tokens.client_id.as_str()),
  ];
  http
    .post(revocation_endpoint)
    .form(&form)
    .send()
    .await?
    .error_for_status()?;

  Ok(())
}

struct Callback {
  params: HashMap<String, String>,
  page: oneshot::Sender<String>,
  server: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Callback {
  async fn respond(self, page: String) {
    let _ = self.page.send(page);
    let _ = tokio::time::timeout(Duration::from_secs(2), self.server).await;
  }
}

async fn wait_for_callback(listener: TcpListener, state: String) -> Result<Callback> {
  let (sender, receiver) = oneshot::channel();
  let sender = Arc::new(Mutex::new(Some(sender)));
  let (done_sender, done_receiver) = oneshot::channel::<()>();
  let done_sender = Arc::new(Mutex::new(Some(done_sender)));

  let app = Router::new().route(
    "/callback",
    get(move |Query(params): Query<HashMap<String, String>>| {
      let is_current = params.get("state") == Some(&state);
      let sender = is_current
        .then(|| sender.lock().unwrap_or_else(PoisonError::into_inner).take())
        .flatten();
      let done = is_current
        .then(|| done_sender.lock().unwrap_or_else(PoisonError::into_inner).take())
        .flatten();
      async move {
        let Some(sender) = sender else {
          return (StatusCode::BAD_REQUEST, Html(String::new()));
        };
        let (page_sender, page_receiver) = oneshot::channel();
        let _ = sender.send((params, page_sender));
        let page = page_receiver.await.unwrap_or_default();
        if let Some(done) = done {
          let _ = done.send(());
        }
        (StatusCode::OK, Html(page))
      }
    }),
  );

  let server = tokio::spawn(async move {
    axum::serve(listener, app)
      .with_graceful_shutdown(async move {
        let _ = done_receiver.await;
      })
      .await
  });

  match tokio::time::timeout(LOGIN_TIMEOUT, receiver).await {
    Ok(Ok((params, page))) => Ok(Callback { params, page, server }),
    Ok(Err(_)) => {
      server.abort();
      Err(Error::failed("login_failed", "The browser login stopped."))
    }
    Err(_) => {
      server.abort();
      Err(
        Error::failed("timeout", "The browser login did not finish in 5 minutes.")
          .with_next("inboxjot login", "Start the login again"),
      )
    }
  }
}

async fn send_oauth_request(
  http: &reqwest::Client,
  request: oauth2::HttpRequest,
) -> std::result::Result<oauth2::HttpResponse, std::io::Error> {
  let request = reqwest::Request::try_from(request).map_err(std::io::Error::other)?;
  let response = http.execute(request).await.map_err(std::io::Error::other)?;

  let mut builder = http::Response::builder().status(response.status());
  if let Some(headers) = builder.headers_mut() {
    *headers = response.headers().clone();
  }

  let body = response.bytes().await.map_err(std::io::Error::other)?;
  builder.body(body.to_vec()).map_err(std::io::Error::other)
}

fn to_grant(
  client_id: &str,
  scopes_supported: &[String],
  response: &StandardTokenResponse<WorkspaceFields, BasicTokenType>,
) -> Grant {
  let expires_in = response.expires_in().map_or(3_600, |duration| duration.as_secs());
  let expires_at =
    jiff::Timestamp::now().as_second() + i64::try_from(expires_in).unwrap_or(0);

  Grant {
    tokens: OAuthTokens {
      client_id: client_id.to_owned(),
      access_token: response.access_token().secret().clone(),
      refresh_token: response
        .refresh_token()
        .map(|token| token.secret().clone())
        .unwrap_or_default(),
      expires_at,
      scopes: response
        .scopes()
        .map(|scopes| scopes.iter().map(|scope| scope.to_string()).collect())
        .unwrap_or_default(),
      scopes_supported: scopes_supported.to_vec(),
      refresh_expires_at: response
        .extra_fields()
        .refresh_token_expires_in
        .map(|seconds| jiff::Timestamp::now().as_second() + seconds),
    },
    workspace_slug: response.extra_fields().workspace_slug.clone(),
    workspace_name: response.extra_fields().workspace_name.clone(),
    user_name: response.extra_fields().user_name.clone(),
    user_email: response.extra_fields().user_email.clone(),
  }
}

fn get_token_error<E: std::error::Error + 'static>(
  error: RequestTokenError<E, BasicErrorResponse>,
) -> Error {
  match error {
    RequestTokenError::ServerResponse(response) => {
      let description = response.error_description().cloned();
      Error::auth(
        "oauth_error",
        description.unwrap_or_else(|| response.error().to_string()),
      )
    }
    error => Error::failed(
      "oauth_error",
      format!("The login request failed: {}", describe(&error)),
    ),
  }
}

fn describe(error: &dyn std::error::Error) -> String {
  let mut text = error.to_string();
  let mut source = error.source();
  while let Some(cause) = source {
    text.push_str(": ");
    text.push_str(&cause.to_string());
    source = cause.source();
  }
  text
}

fn parse_url<T>(url: std::result::Result<T, oauth2::url::ParseError>) -> Result<T> {
  url.map_err(|error| {
    Error::failed("oauth_error", format!("The login URL is not valid: {error}"))
  })
}

#[cfg(test)]
mod login_code_tests {
  use super::create_login_code;

  #[test]
  fn makes_two_groups_of_four_consonants() {
    let code = create_login_code().unwrap();
    assert_eq!(code.len(), 9);
    assert_eq!(&code[4..5], "-");
    assert!(
      code
        .chars()
        .filter(|letter| *letter != '-')
        .all(|letter| "BCDFGHJKLMNPQRSTVWXZ".contains(letter))
    );
  }
}

#[cfg(test)]
mod tests {
  use std::collections::HashMap;

  use reqwest::Url;
  use serde_json::json;
  use wiremock::matchers::{body_string_contains, method, path};
  use wiremock::{Mock, MockServer, ResponseTemplate};

  use oauth2::CsrfToken;

  use super::{
    CLIENT_ID, Discovery, authorize, build_http, get_device_name, read_login_code,
  };

  #[test]
  fn ignores_an_error_without_the_login_state() {
    let state = CsrfToken::new("expected".to_owned());
    let forged = HashMap::from([
      ("error".to_owned(), "server_error".to_owned()),
      ("error_description".to_owned(), "\u{1b}]0;pwned\u{7}".to_owned()),
    ]);

    let error = read_login_code(&forged, &state).expect_err("the login fails");

    assert_eq!(error.message, "The browser answered for another login.");
  }

  #[tokio::test]
  async fn authorizes_as_the_first_party_client() {
    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(method("POST"))
      .and(path("/v1/oauth/token"))
      .and(body_string_contains("client_id=inboxjot-cli"))
      .respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "access_token": "at_1", "token_type": "bearer", "expires_in": 3600,
        "refresh_token": "rt_1", "scope": "domains:read", "workspace_slug": "acme"
      })))
      .expect(1)
      .mount(&server)
      .await;
    let discovery = Discovery {
      authorization_endpoint: format!("{uri}/oauth/authorize"),
      token_endpoint: format!("{uri}/v1/oauth/token"),
      revocation_endpoint: format!("{uri}/v1/oauth/revoke"),
      scopes_supported: vec!["domains:read".to_owned(), "domains:write".to_owned()],
    };
    let mut params = HashMap::new();

    let grant = authorize(&build_http().unwrap(), &discovery, &uri, "WDJB-MJHT", |url| {
      params = Url::parse(url).unwrap().query_pairs().into_owned().collect();
      let callback =
        format!("{}?code=c_1&state={}", params["redirect_uri"], params["state"]);
      tokio::spawn(reqwest::get(callback));
    })
    .await
    .unwrap();

    assert_eq!(params["client_id"], CLIENT_ID);
    assert_eq!(params["device_name"], get_device_name());
    assert_eq!(params["login_code"], "WDJB-MJHT");
    assert!(!params["device_name"].ends_with(".local"));
    assert_eq!(grant.tokens.client_id, CLIENT_ID);
    assert_eq!(grant.tokens.scopes, ["domains:read"]);
    assert_eq!(grant.tokens.scopes_supported, discovery.scopes_supported);
    assert_eq!(grant.workspace_slug.as_deref(), Some("acme"));
    assert!(
      server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| { request.url.path() != "/v1/oauth/register" })
    );
  }
}
