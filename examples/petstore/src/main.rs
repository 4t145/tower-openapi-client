//! Petstore example: shows how to drive generated operation types
//! through [`ApiClient`] with a tower transport.
//!
//! Running this binary doesn't touch the network — the transport is a
//! hand-written stub that returns canned JSON, because the example is
//! primarily about *the generated code's shape and ergonomics*. To
//! point at a real server, swap `FakeTransport` for something like a
//! `reqwest` adapter and pass the real base URL.
//!
//! Not every operation in the Petstore 3.1 spec is exercised here — a
//! couple of fields are driven by JSON Schema `$id`/`$anchor` pointers
//! that `oas3` 0.21 does not expose, and the generator falls those
//! back to `serde_json::Value`. The ops we demo (`addPet`,
//! `getPetById`, `updatePet`) touch only well-typed parts of the spec.

use std::{
    convert::Infallible,
    task::{Context, Poll},
};

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{Empty, Full};
use tower::{Service, ServiceExt};
// `Service` is only needed for the `impl Service for FakeTransport`
// blocks below; the call sites use `ServiceExt::oneshot` exclusively.
use toac::{ApiClient, CallError, IntoHttpRequest};

use petstore_example::{
    components::{Category, Pet, PetStatus},
    operations::{
        AddPetRequest, AddPetResponse, GetPetByIdRequest, GetPetByIdResponse, UpdatePetRequest,
    },
};

/// Tower transport that hands out canned responses keyed by request
/// path. Used instead of an HTTP client so the example stays self-
/// contained.
#[derive(Clone)]
struct FakeTransport;

impl Service<Request<Empty<Bytes>>> for FakeTransport {
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Empty<Bytes>>) -> Self::Future {
        let path = req.uri().path().to_owned();
        Box::pin(async move {
            // The test server sits under `/api/v3`, so match on the
            // trailing portion after stripping that prefix.
            let suffix = path.rsplit_once("/pet/").map(|(_, tail)| tail);
            let (status, body) = match suffix {
                Some("1") => (
                    200u16,
                    r#"{"id":1,"name":"Buddy","photoUrls":["https://example.com/1.jpg"],"status":"available"}"#,
                ),
                Some("404") => (404, ""),
                _ => (200, r#"{"id":999,"name":"fallback","photoUrls":[]}"#),
            };
            Ok(Response::builder()
                .status(status)
                .body(Full::new(Bytes::from(body)))
                .expect("canned response"))
        })
    }
}

/// Sibling transport for operations whose `IntoHttpRequest<Full<Bytes>>`
/// emits a JSON body. The only thing that changes from `FakeTransport`
/// is the request body type parameter.
#[derive(Clone)]
struct FakeJsonTransport;

impl Service<Request<Full<Bytes>>> for FakeJsonTransport {
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Full<Bytes>>) -> Self::Future {
        let (parts, body) = req.into_parts();
        let path = parts.uri.path().to_owned();
        let method = parts.method.clone();
        Box::pin(async move {
            use http_body_util::BodyExt;
            let body_bytes = body.collect().await.expect("collect fake body").to_bytes();
            println!(
                "  [fake server] received {method} {path} body={}",
                String::from_utf8_lossy(&body_bytes),
            );
            // Echo the body back at 200, i.e. "created/updated".
            Ok(Response::builder()
                .status(200)
                .body(Full::new(body_bytes))
                .expect("canned response"))
        })
    }
}

fn main() {
    futures_executor::block_on(run_demo());
}

async fn run_demo() {
    let base_url = "https://petstore3.swagger.io/api/v3";

    demo_inspect_request(base_url).await;
    demo_get_pet(base_url).await;
    demo_get_pet_missing(base_url).await;
    demo_add_pet(base_url).await;
    demo_update_pet(base_url).await;
}

/// What a generated request looks like when it hits the wire.
async fn demo_inspect_request(base_url: &str) {
    println!("--- inspect generated GET request ---");
    let req = GetPetByIdRequest { pet_id: 1 };
    let http_req = req.into_http_request().await;
    println!("  method  = {}", http_req.method());
    println!("  path    = {}", http_req.uri());
    println!(
        "  method const from metadata impl: {}",
        GetPetByIdRequest::METHOD
    );
    println!(
        "  path template from metadata impl: {}",
        GetPetByIdRequest::PATH_TEMPLATE,
    );
    // Path is relative; `ApiClient` is what prefixes base_url. Show it
    // for completeness.
    println!("  (ApiClient would dispatch this under {base_url})");
    println!();
}

/// Full round-trip with ApiClient for an existing pet.
async fn demo_get_pet(base_url: &str) {
    println!("--- GET /pet/1 (ApiClient end-to-end) ---");
    let client = ApiClient::new(FakeTransport, base_url);
    // `oneshot` is provided by `tower::ServiceExt`: it does
    // `poll_ready` + `call` in one step and consumes the client, so
    // the request value on its own nails down which `Service<Op>`
    // impl we're using. No turbofish needed.
    let outcome = client.oneshot(GetPetByIdRequest { pet_id: 1 }).await;

    match outcome {
        Ok(GetPetByIdResponse::Status200(pet)) => {
            println!("  decoded 200 Pet: id={:?} name={:?}", pet.id, pet.name);
        }
        Ok(other) => println!("  unexpected response variant: {other:?}"),
        Err(err) => report_call_error("getPetById", &err),
    }
    println!();
}

/// Same op, missing pet — should round-trip to `Status404`.
async fn demo_get_pet_missing(base_url: &str) {
    println!("--- GET /pet/404 (not found) ---");
    let client = ApiClient::new(FakeTransport, base_url);
    let outcome = client.oneshot(GetPetByIdRequest { pet_id: 404 }).await;

    match outcome {
        Ok(GetPetByIdResponse::Status404) => {
            println!("  observed 404 as expected");
        }
        Ok(other) => println!("  unexpected variant: {other:?}"),
        Err(err) => report_call_error("getPetById", &err),
    }
    println!();
}

/// POST with a JSON body, demonstrating request body serialisation.
async fn demo_add_pet(base_url: &str) {
    println!("--- POST /pet (addPet) ---");
    let request = AddPetRequest {
        body: Pet {
            id: Some(42),
            name: "Milo".into(),
            photo_urls: vec!["https://example.com/milo.jpg".into()],
            status: Some(PetStatus::Available),
            category: Some(Category {
                id: Some(1),
                name: Some("Cats".into()),
            }),
            tags: None,
            available_instances: None,
            // The fields below come out as `serde_json::Value` because
            // their source `$ref` uses JSON Schema `$id`, which oas3
            // 0.21 doesn't surface. Leaving them as JSON null keeps the
            // generated type usable without forcing the user to reach
            // for the fallback shape.
            pet_details: None,
            pet_details_id: None,
        },
    };

    let client = ApiClient::new(FakeJsonTransport, base_url);
    let outcome = client.oneshot(request).await;

    match outcome {
        Ok(AddPetResponse::Status200(pet)) => {
            println!("  server echoed back Pet: name={:?}", pet.name);
        }
        Ok(other) => println!("  unexpected variant: {other:?}"),
        Err(err) => report_call_error("addPet", &err),
    }
    println!();
}

/// PUT: near-identical to addPet but distinct operation + response enum.
async fn demo_update_pet(base_url: &str) {
    println!("--- PUT /pet (updatePet) ---");
    let request = UpdatePetRequest {
        body: Pet {
            id: Some(42),
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

    let client = ApiClient::new(FakeJsonTransport, base_url);
    let outcome = client.oneshot(request).await;

    match outcome {
        Ok(resp) => println!("  response variant: {resp:?}"),
        Err(err) => report_call_error("updatePet", &err),
    }
    println!();
}

fn report_call_error<E: std::fmt::Display>(op: &str, err: &CallError<E>) {
    match err {
        CallError::Transport(e) => println!("  {op} transport error: {e}"),
        CallError::Decode(e) => println!("  {op} decode error: {e}"),
    }
}
