//! Petstore example: drives generated operation types against the real
//! Swagger Petstore API (`https://petstore3.swagger.io/api/v3`) over a
//! hyper HTTPS client built by [`client_util::client::build_https_client`].
//!
//! The transport is a `Service<Request, Response = http::Response<Incoming>>`
//! — `ApiClient` now accepts that directly without a body-adapter
//! layer.

use petstore_example::{
    components::{Category, Pet, PetStatus},
    operations::pet::{by_pet_id::get as get_pet, post as add_pet, put as update_pet},
};
use toac::ApiClient;
use tower::ServiceExt;
use tracing::{info, warn};

// Local aliases keep the call sites below readable while delegating to
// the path-module types the generator now emits.
type GetPetByIdRequest = get_pet::Request;
type GetPetByIdResponseBody = get_pet::ResponseBody;
type AddPetRequest = add_pet::Request;
type AddPetResponseBody = add_pet::ResponseBody;
type UpdatePetRequest = update_pet::Request;
type UpdatePetResponseBody = update_pet::ResponseBody;

/// Base URL used by every call below.
const PETSTORE_BASE_URL: &str = "https://petstore3.swagger.io/api/v3";

/// Pet identifier used by the GET demo. Petstore seeds a handful of
/// sample pets; `1` is the one their own UI uses.
const SAMPLE_PET_ID: i64 = 1;

/// ID that Petstore reliably answers with 404 — well outside the seeded
/// range, so the server reports "pet not found".
const MISSING_PET_ID: i64 = 999_999_999;

/// HTTPS client built by `client_util`; concrete so the demo helpers
/// don't need a trait-bound tangle.
type HttpClient = client_util::client::HyperHttpsClient<toac::body::Body>;
type PetstoreClient = ApiClient<HttpClient>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let http = client_util::client::build_https_client::<toac::body::Body>()?;
    let client: PetstoreClient = ApiClient::new(http, PETSTORE_BASE_URL);

    demo_get_pet(client.clone()).await;
    demo_get_pet_missing(client.clone()).await;
    demo_add_pet(client.clone()).await;
    demo_update_pet(client).await;

    Ok(())
}

/// GET /pet/{id} — typed round-trip for an existing pet.
async fn demo_get_pet(client: PetstoreClient) {
    info!(pet_id = SAMPLE_PET_ID, "GET /pet/{{id}}");
    match client
        .oneshot(GetPetByIdRequest {
            pet_id: SAMPLE_PET_ID,
        })
        .await
    {
        Ok(resp) => match resp.body {
            GetPetByIdResponseBody::Status200(pet) => {
                info!(id = ?pet.id, name = %pet.name, status = ?pet.status, "pet fetched");
            }
            GetPetByIdResponseBody::Status400 => info!("server reported 400 invalid id"),
            GetPetByIdResponseBody::Status404 => info!("pet not found"),
            GetPetByIdResponseBody::Default => {
                info!("server returned unspecified status (default variant)");
            }
        },
        Err(err) => report_call_error("getPetById", &err),
    }
}

/// GET /pet/{id} against an id outside the seeded range — expect 404.
async fn demo_get_pet_missing(client: PetstoreClient) {
    info!(pet_id = MISSING_PET_ID, "GET /pet/{{id}} (expecting 404)");
    match client
        .oneshot(GetPetByIdRequest {
            pet_id: MISSING_PET_ID,
        })
        .await
    {
        Ok(resp) => match resp.body {
            GetPetByIdResponseBody::Status200(pet) => {
                warn!(id = ?pet.id, "unexpected 200 for missing id");
            }
            GetPetByIdResponseBody::Status400 => info!("server reported 400 invalid id"),
            GetPetByIdResponseBody::Status404 => info!("observed 404 as expected"),
            GetPetByIdResponseBody::Default => {
                info!("server returned unspecified status (default variant)");
            }
        },
        Err(err) => report_call_error("getPetById", &err),
    }
}

/// POST /pet — create a new pet. Petstore usually answers 200 with the
/// created pet echoed back.
async fn demo_add_pet(client: PetstoreClient) {
    info!("POST /pet");
    let request = AddPetRequest {
        body: Pet {
            id: None,
            name: "Milo".into(),
            photo_urls: vec!["https://example.com/milo.jpg".into()],
            status: Some(PetStatus::Available),
            category: Some(Category {
                id: Some(1),
                name: Some("Cats".into()),
            }),
            tags: None,
            available_instances: None,
            pet_details: None,
            pet_details_id: None,
        },
    };

    match client.oneshot(request).await {
        Ok(resp) => match resp.body {
            AddPetResponseBody::Status200(pet) => {
                info!(id = ?pet.id, name = %pet.name, "pet created");
            }
            AddPetResponseBody::Status405 => info!("server reported 405 invalid input"),
            AddPetResponseBody::Default => {
                info!("server returned unspecified status (default variant)");
            }
        },
        Err(err) => report_call_error("addPet", &err),
    }
}

/// PUT /pet — mirrors addPet but distinct operation + response enum.
async fn demo_update_pet(client: PetstoreClient) {
    info!("PUT /pet");
    let request = UpdatePetRequest {
        body: Pet {
            id: Some(SAMPLE_PET_ID),
            name: "Milo-updated".into(),
            photo_urls: vec![],
            status: Some(PetStatus::Sold),
            category: None,
            tags: None,
            available_instances: None,
            pet_details: None,
            pet_details_id: None,
        },
    };

    match client.oneshot(request).await {
        Ok(resp) => match resp.body {
            UpdatePetResponseBody::Status200(pet) => {
                info!(id = ?pet.id, name = %pet.name, status = ?pet.status, "pet updated");
            }
            UpdatePetResponseBody::Status400 => info!("server reported 400 invalid id"),
            UpdatePetResponseBody::Status404 => info!("pet not found"),
            UpdatePetResponseBody::Status405 => info!("server reported 405 validation exception"),
            UpdatePetResponseBody::Default => {
                info!("server returned unspecified status (default variant)");
            }
        },
        Err(err) => report_call_error("updatePet", &err),
    }
}

/// Logs a [`toac::CallError`] at `warn`. Transport failures (DNS, TLS,
/// timeouts) and decode failures (unexpected status / bad payload) are
/// genuine errors the caller should surface — unlike the declared
/// non-2xx statuses, which are routed through the typed response enum.
fn report_call_error<E: std::fmt::Display>(op: &str, err: &toac::CallError<E>) {
    match err {
        toac::CallError::Encode(e) => warn!(op, error = %e, "encode error"),
        toac::CallError::Auth(e) => warn!(op, error = %e, "auth error"),
        toac::CallError::Transport(e) => warn!(op, error = %e, "transport error"),
        toac::CallError::Decode(e) => warn!(op, error = %e, "decode error"),
    }
}
