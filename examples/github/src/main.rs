//! GitHub example: drives the generated client against
//! `https://api.github.com`, exercising the response shapes the
//! upstream spec exposes:
//!
//! * `application/json` — `GET /meta` (`ApiOverview` struct) and
//!   `GET /users/{username}` (a discriminator-tagged `oneOf`).
//! * `text/plain`       — `GET /zen` (raw string body).
//! * status-only        — `GET /octocat` (200 with no body decode).
//!
//! The endpoints picked here are all public (`SECURITY: &[]` in the
//! generated code), so the demo runs without an API token. GitHub's
//! API does require a non-empty `User-Agent`; we wire one through
//! `tower_http::set_header::SetRequestHeaderLayer` so it lands on
//! every outbound request without touching the operation types.
//!
//! Set `GITHUB_USERNAME` to override the user the demo profiles —
//! defaults to `octocat`, which is a public account that the API
//! always exposes.
//!
//! `GITHUB_API_URL` overrides the base URL for proxies / mocks.

use std::env;

use github_example::{
    components::{ApiOverview, PrivateUser, PublicUser},
    operations::{
        meta::get as get_meta, octocat::get as get_octocat, users::by_username::get as get_user,
        zen::get as get_zen,
    },
};
use http::HeaderValue;
use toac::ApiClient;
use tower::{Service, ServiceBuilder};
use tower_http::set_header::SetRequestHeaderLayer;
use tracing::{info, warn};

/// GitHub's REST API root. Matches `servers[0].url` in the spec but
/// must still be passed explicitly to [`ApiClient::new`].
const GITHUB_BASE_URL: &str = "https://api.github.com";

/// Override the base URL (proxies, mocks, GitHub Enterprise).
const API_URL_ENV: &str = "GITHUB_API_URL";

/// Override the username the user-profile demo fetches. Defaults to
/// `octocat`, an account the public API always exposes.
const USERNAME_ENV: &str = "GITHUB_USERNAME";
const DEFAULT_USERNAME: &str = "octocat";

/// GitHub rejects requests without a `User-Agent`; this is a generic
/// label that identifies the demo as a `toac` smoke test.
const USER_AGENT: &str = "toac-github-example";

/// Cap on the number of `meta.api` IP ranges printed — the real list
/// is several dozen entries long and would dominate the log.
const META_PREVIEW_LIMIT: usize = 5;

type HttpClient = client_util::client::HyperHttpsClient<toac::body::Body>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let base_url = env::var(API_URL_ENV).ok().unwrap_or_else(|| {
        info!("using default API URL: {GITHUB_BASE_URL}");
        GITHUB_BASE_URL.to_string()
    });
    let username = env::var(USERNAME_ENV).unwrap_or_else(|_| DEFAULT_USERNAME.to_string());

    let http: HttpClient = client_util::client::build_https_client::<toac::body::Body>()?;
    // Layer in the static User-Agent header. The codegen never
    // touches request headers other than the ones it auto-emits
    // (Accept, Authorization), so a tower layer is the natural place
    // for cross-cutting headers like this one.
    let transport = ServiceBuilder::new()
        .layer(SetRequestHeaderLayer::if_not_present(
            http::header::USER_AGENT,
            HeaderValue::from_static(USER_AGENT),
        ))
        .service(http);
    let mut client = ApiClient::new(transport, base_url);

    demo_zen(&mut client).await;
    demo_octocat(&mut client).await;
    demo_meta(&mut client).await;
    demo_user(&mut client, &username).await;
    Ok(())
}

/// `GET /zen` — text/plain body. The simplest sanity check that
/// transport + content negotiation are wired up correctly.
async fn demo_zen<S>(client: &mut ApiClient<S>)
where
    ApiClient<S>: Service<get_zen::Request, Response = get_zen::Response>,
    <ApiClient<S> as Service<get_zen::Request>>::Error: std::fmt::Display,
{
    info!("GET /zen");
    match client.call(get_zen::Request {}).await {
        Ok(get_zen::Response::Status200(quote)) => info!(quote = %quote, "zen"),
        Err(err) => warn!(error = %err, "zen call failed"),
    }
}

/// `GET /octocat` — status-only response. The endpoint actually
/// returns ASCII art in the body, but the spec models it as a bare
/// 200 with no schema, so the response enum is unit-only.
async fn demo_octocat<S>(client: &mut ApiClient<S>)
where
    ApiClient<S>: Service<get_octocat::Request, Response = get_octocat::Response>,
    <ApiClient<S> as Service<get_octocat::Request>>::Error: std::fmt::Display,
{
    info!("GET /octocat");
    let request = get_octocat::Request {
        s: Some("hello from toac".to_string()),
    };
    match client.call(request).await {
        Ok(get_octocat::Response::Status200) => info!("octocat returned 200"),
        Err(err) => warn!(error = %err, "octocat call failed"),
    }
}

/// `GET /meta` — JSON body decoded into a typed `ApiOverview`. The
/// struct has dozens of optional vec fields; we sample a handful to
/// confirm the JSON codec round-tripped end-to-end.
async fn demo_meta<S>(client: &mut ApiClient<S>)
where
    ApiClient<S>: Service<get_meta::Request, Response = get_meta::Response>,
    <ApiClient<S> as Service<get_meta::Request>>::Error: std::fmt::Display,
{
    info!("GET /meta");
    match client.call(get_meta::Request {}).await {
        Ok(get_meta::Response::Status200(overview)) => log_meta(&overview),
        Ok(get_meta::Response::Status304) => info!("meta returned 304 Not Modified"),
        Err(err) => warn!(error = %err, "meta call failed"),
    }
}

/// `GET /users/{username}` — exercises path-parameter rendering
/// (the `{username}` placeholder) and a discriminator-tagged `oneOf`
/// response (`Public` vs `Private` user).
async fn demo_user<S>(client: &mut ApiClient<S>, username: &str)
where
    ApiClient<S>: Service<get_user::Request, Response = get_user::Response>,
    <ApiClient<S> as Service<get_user::Request>>::Error: std::fmt::Display,
{
    info!(username, "GET /users/{{username}}");
    let request = get_user::Request {
        username: username.to_string(),
    };
    match client.call(request).await {
        Ok(get_user::Response::Status200(body)) => {
            // The body's enum name carries a synthetic numeric suffix
            // (`ResponseStatus200Body9789`) that collision-avoidance
            // appends; `try_into` reaches the variant payload without
            // having to spell that ident out.
            if let Ok(public) = PublicUser::try_from(body.clone()) {
                log_public_user(&public);
            } else if let Ok(private) = PrivateUser::try_from(body) {
                log_private_user(&private);
            }
        }
        Ok(get_user::Response::Status404(_)) => warn!(username, "user not found"),
        Err(err) => warn!(error = %err, "user call failed"),
    }
}

/// Logs a small slice of the `ApiOverview` payload — printing the
/// whole struct would dump tens of kilobytes of IP ranges.
fn log_meta(overview: &ApiOverview) {
    if let Some(api) = overview.api.as_ref() {
        let preview: Vec<&str> = api
            .iter()
            .take(META_PREVIEW_LIMIT)
            .map(String::as_str)
            .collect();
        info!(
            total = api.len(),
            preview = ?preview,
            "meta.api IP ranges",
        );
    }
    if let Some(domains) = overview.domains.as_ref()
        && let Some(website) = domains.website.as_ref()
    {
        info!(count = website.len(), "meta.domains.website");
    }
    info!(
        verifiable_password_authentication = overview.verifiable_password_authentication,
        "meta flags",
    );
}

fn log_public_user(user: &PublicUser) {
    info!(
        login = %user.login,
        id = user.id,
        followers = user.followers,
        "public user",
    );
}

fn log_private_user(user: &PrivateUser) {
    info!(
        login = %user.login,
        id = user.id,
        "private user (response carried private fields)",
    );
}
