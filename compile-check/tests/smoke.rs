//! Smoke tests that exercise the generated types end-to-end. If the
//! generator emits something that happens to parse but then fails to
//! compile, this is the test binary that catches it — the `include!`
//! inside `lib.rs` already ensures the generated code is type-checked
//! to build this very binary.

#![allow(clippy::manual_async_fn)]

use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{Empty, Full};
use tower::{Service, ServiceExt};
// `Service` only participates in the transport impl below; call sites
// use `ServiceExt::oneshot`.
use toac::{ApiClient, CallError, FromHttpResponse, IntoHttpRequest, Operation};
use toac_compile_check::{
    components::{FormatSample, NewPet, Pet},
    operations::{CreatePetRequest, CreatePetResponse, GetPetRequest, GetPetResponse},
};

/// Static assertion: `{Op}Request` really implements `Operation`.
fn assert_is_operation<T: Operation>() {}

#[test]
fn trait_bounds_check() {
    assert_is_operation::<GetPetRequest>();
    assert_is_operation::<CreatePetRequest>();
}

#[test]
fn get_request_renders_uri() {
    let req = GetPetRequest {
        id: "abc".into(),
        limit: Some(10),
        x_trace: Some("t1".into()),
    };
    let http_req = futures_executor::block_on(req.into_http_request());
    assert_eq!(http_req.method(), http::Method::GET);
    assert_eq!(http_req.uri().to_string(), "/pets/abc?limit=10");
    assert_eq!(
        http_req
            .headers()
            .get("X-Trace")
            .map(|v| v.to_str().unwrap()),
        Some("t1"),
    );
}

#[test]
fn post_body_serialises_to_json() {
    let req = CreatePetRequest {
        body: NewPet { name: "rex".into() },
    };
    let http_req = futures_executor::block_on(req.into_http_request());
    assert_eq!(http_req.method(), http::Method::POST);

    use http_body_util::BodyExt;
    let body = futures_executor::block_on(http_req.into_body().collect())
        .expect("collect")
        .to_bytes();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["name"], "rex");
}

#[test]
fn get_response_decodes_200_and_404() {
    let ok = Response::builder()
        .status(200)
        .body(Full::new(Bytes::from(r#"{"id":"abc","name":"rex"}"#)))
        .unwrap();
    let decoded = futures_executor::block_on(GetPetResponse::from_http_response(ok)).expect("ok");
    match decoded {
        GetPetResponse::Status200(pet) => {
            assert_eq!(pet.id, "abc");
            assert_eq!(pet.name, "rex");
        }
        other => panic!("expected 200, got {other:?}"),
    }

    let not_found = Response::builder()
        .status(404)
        .body(Full::new(Bytes::from(r#"{"message":"missing"}"#)))
        .unwrap();
    let decoded =
        futures_executor::block_on(GetPetResponse::from_http_response(not_found)).expect("ok");
    assert!(matches!(decoded, GetPetResponse::Status404(_)));
}

#[test]
fn unknown_status_falls_through_to_default() {
    let resp = Response::builder()
        .status(500)
        .body(Full::new(Bytes::from(r#"{"message":"boom"}"#)))
        .unwrap();
    let decoded = futures_executor::block_on(GetPetResponse::from_http_response(resp)).expect("ok");
    assert!(matches!(decoded, GetPetResponse::Default(_)));
}

// --- ApiClient end-to-end with a canned transport ---

#[derive(Clone)]
struct StaticTransport {
    last_uri: Arc<Mutex<Option<http::Uri>>>,
    status: u16,
    body: Bytes,
}

impl Service<Request<Empty<Bytes>>> for StaticTransport {
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Empty<Bytes>>) -> Self::Future {
        *self.last_uri.lock().unwrap() = Some(req.uri().clone());
        let status = self.status;
        let body = self.body.clone();
        Box::pin(async move {
            Ok(Response::builder()
                .status(status)
                .body(Full::new(body))
                .expect("valid response"))
        })
    }
}

#[test]
fn api_client_end_to_end() {
    let transport = StaticTransport {
        last_uri: Arc::new(Mutex::new(None)),
        status: 200,
        body: Bytes::from(r#"{"id":"42","name":"rex"}"#),
    };
    let uri_tap = transport.last_uri.clone();
    let client = ApiClient::new(transport, "https://api.example.com");

    let resp = futures_executor::block_on(async move {
        client
            .oneshot(GetPetRequest {
                id: "42".into(),
                limit: None,
                x_trace: None,
            })
            .await
    });

    let resp = resp.expect("call ok");
    match resp {
        GetPetResponse::Status200(pet) => assert_eq!(pet.name, "rex"),
        other => panic!("unexpected response {other:?}"),
    }
    assert_eq!(
        uri_tap.lock().unwrap().as_ref().unwrap().to_string(),
        "https://api.example.com/pets/42",
    );
}

/// Just reference the extra generated types so the linker doesn't drop
/// them; this catches bugs where a type is emitted into the module but
/// has a name that can't actually be imported.
#[allow(dead_code)]
fn names_exist() {
    let _: fn() -> Pet = || unreachable!();
    let _: fn() -> CreatePetResponse = || unreachable!();
    let _: fn(CallError<Infallible>) = |_| ();
}

/// `format:` handling: constructing `FormatSample` uses the richer types
/// (`Uuid`, `DateTime<Utc>`, `NaiveDate`, `NaiveTime`, `Base64String`,
/// `Bytes`) rather than raw `String`.
#[test]
fn format_mapping_uses_typed_representations() {
    use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
    use toac::Base64String;
    use uuid::Uuid;

    let sample = FormatSample {
        id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        created_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap(),
        birthday: Some(NaiveDate::from_ymd_opt(1990, 1, 2).unwrap()),
        wake_time: Some(NaiveTime::from_hms_opt(7, 30, 0).unwrap()),
        payload: Base64String::from_bytes(b"hello".as_slice().to_vec()),
        blob: Some(bytes::Bytes::from_static(b"raw")),
    };

    let json = serde_json::to_string(&sample).expect("serialise");
    // Round-trip through serde to verify all typed fields know how to
    // project themselves onto/from JSON.
    let decoded: FormatSample = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(decoded.id, sample.id);
    assert_eq!(decoded.created_at, sample.created_at);
    assert_eq!(decoded.birthday, sample.birthday);
    assert_eq!(decoded.wake_time, sample.wake_time);
    assert_eq!(decoded.payload.as_bytes(), sample.payload.as_bytes());
    assert_eq!(decoded.blob, sample.blob);
    // `payload` goes over the wire as base64, not a byte array.
    assert!(
        json.contains("\"aGVsbG8=\""),
        "expected base64 in JSON: {json}"
    );
}
