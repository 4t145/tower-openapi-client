//! Daytona example: drives generated operation types against the real
//! Daytona API over a hyper HTTPS client, authenticating with a Bearer
//! token supplied via `DAYTONA_API_KEY`.
//!
//! The Daytona public instance (`https://app.daytona.io/api`) gates
//! most endpoints behind the `bearer` security scheme. The generator
//! now emits a per-spec `AuthConfig` (plus builder) wired into the
//! runtime's `AuthSelector`, so the demo just calls
//! `AuthConfig::builder().bearer(token).build()` and hands that to
//! `ApiClient::with_auth`.

use std::env;

use daytona_example::{
    components::Organization, operations::organizations::get as list_orgs,
    operations::sandbox::get as list_sandboxes, security::AuthConfig,
};
use toac::ApiClient;
use tower::Service;
use tracing::{error, info, warn};

/// Daytona's public API root.
const DAYTONA_BASE_URL: &str = "https://app.daytona.io/api";

/// Environment variable used to supply the bearer token. Keep out of
/// source; create a personal API key from the Daytona dashboard.
const API_KEY_ENV: &str = "DAYTONA_API_KEY";
/// Optional environment variable to override the API URL. If not set, defaults to `DAYTONA_BASE_URL`.
/// Useful for testing against a local Daytona instance.
const API_URL_ENV: &str = "DAYTONA_API_URL";

type HttpClient = client_util::client::HyperHttpsClient<toac::body::Body>;
type DaytonaClient = ApiClient<HttpClient>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let Ok(token) = env::var(API_KEY_ENV) else {
        error!("set {API_KEY_ENV} to a Daytona API key before running this example");
        return Ok(());
    };

    let base_url = env::var(API_URL_ENV).ok().unwrap_or_else(|| {
        info!("using default API URL: {DAYTONA_BASE_URL}");
        DAYTONA_BASE_URL.to_string()
    });

    let auth = AuthConfig::builder().bearer(token).build();
    let http = client_util::client::build_https_client::<toac::body::Body>()?;
    let mut client: DaytonaClient = ApiClient::new(http, base_url).with_auth(auth);

    demo_list_organizations(&mut client).await;
    demo_list_sandboxes(&mut client).await;
    Ok(())
}

/// GET /organizations — lists every organization the token can see.
async fn demo_list_organizations(client: &mut DaytonaClient) {
    info!("GET /organizations");
    match client.call(list_orgs::Request {}).await {
        Ok(resp) => match resp.body {
            list_orgs::ResponseBody::Status200(orgs) => {
                info!(count = orgs.len(), "organizations returned");
                for org in orgs.iter().take(5) {
                    log_org(org);
                }
                if orgs.len() > 5 {
                    info!(omitted = orgs.len() - 5, "… remaining organizations elided");
                }
            }
        },
        Err(err) => report_call_error("listOrganizations", &err),
    }
}

async fn demo_list_sandboxes(client: &mut DaytonaClient) {
    info!("GET /sandboxes");
    match client
        .call(list_sandboxes::Request {
            x_daytona_organization_id: std::env::var("DAYTONA_ORGANIZATION_ID").ok(),
            verbose: None,
            labels: None,
            include_errored_deleted: None,
        })
        .await
    {
        Ok(resp) => match resp.body {
            list_sandboxes::ResponseBody::Status200(sandboxes) => {
                info!(count = sandboxes.len(), "sandboxes returned");
                for sb in sandboxes.iter().take(5) {
                    info!(id = %sb.id, name = %sb.name, "sandbox");
                }
                if sandboxes.len() > 5 {
                    info!(
                        omitted = sandboxes.len() - 5,
                        "… remaining sandboxes elided"
                    );
                }
            }
        },
        Err(err) => report_call_error("listSandboxes", &err),
    }
}

/// Minimal summary log. `Organization` carries a lot of fields; this
/// picks the two the Daytona UI also leads with.
fn log_org(org: &Organization) {
    info!(id = %org.id, name = %org.name, "organization");
}

/// Logs a [`toac::CallError`] at `warn`.
fn report_call_error<E: std::fmt::Display>(op: &str, err: &toac::CallError<E>) {
    match err {
        toac::CallError::Encode(e) => warn!(op, error = %e, "encode error"),
        toac::CallError::Auth(e) => warn!(op, error = %e, "auth error"),
        toac::CallError::Transport(e) => warn!(op, error = %e, "transport error"),
        toac::CallError::Decode(e) => warn!(op, error = %e, "decode error"),
    }
}
