//! End-to-end exercise of [`ApiClient`]: wraps a hand-written
//! `tower::Service` that speaks [`toac::Request`] / [`toac::Response`],
//! then drives it through typed operation values.
//!
//! The trait signatures need `impl Future + Send` bounds that `async fn`
//! can't spell on its own.

#![allow(clippy::manual_async_fn)]

use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use ::bytes::Bytes;
use ::http::Method;
use ::http_body_util::{BodyExt, Full};
use ::toac::{
    ApiClient, BoxError, CallError, DecodeError, MakeRequest, Operation, ParseResponse, Request,
    Response, body::Body,
};
use ::tower::{Service, ServiceExt};

// ---------------------------------------------------------------------------
// Minimal hand-written "generated" types.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct GetPetRequest {
    id: String,
}

impl MakeRequest for GetPetRequest {
    type Error = Infallible;

    fn make_request(
        self,
    ) -> impl ::std::future::Future<Output = Result<Request, Self::Error>> + Send {
        async move {
            let uri = format!("/pets/{}", self.id);
            Ok(::http::Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .expect("valid request"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, ::serde::Deserialize)]
struct Pet {
    id: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq)]
enum GetPetResponse {
    Status200(Pet),
    Status404,
}

impl ParseResponse for GetPetResponse {
    type Error = DecodeError;

    fn parse_response<B>(
        response: ::http::Response<B>,
    ) -> impl ::std::future::Future<Output = Result<Self, Self::Error>> + Send
    where
        B: ::http_body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        async move {
            let (parts, body) = response.into_parts();
            match parts.status.as_u16() {
                200 => {
                    let bytes = BodyExt::collect(body)
                        .await
                        .map_err(|e| DecodeError::Codec(e.into()))?
                        .to_bytes();
                    let pet = ::serde_json::from_slice(bytes.as_ref())
                        .map_err(|e| DecodeError::Codec(Box::new(e)))?;
                    Ok(Self::Status200(pet))
                }
                404 => Ok(Self::Status404),
                _ => Err(DecodeError::UnexpectedStatus(parts.status)),
            }
        }
    }
}

impl Operation for GetPetRequest {
    type Response = GetPetResponse;
}

// ---------------------------------------------------------------------------
// Transport: a Service that records the last URI it saw and answers with
// a canned response.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct RecordingTransport {
    last_uri: Arc<Mutex<Option<::http::Uri>>>,
    canned_status: u16,
    canned_body: Bytes,
}

impl RecordingTransport {
    fn new(status: u16, body: impl Into<Bytes>) -> Self {
        Self {
            last_uri: Arc::new(Mutex::new(None)),
            canned_status: status,
            canned_body: body.into(),
        }
    }
}

impl Service<Request> for RecordingTransport {
    type Response = Response;
    type Error = Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        *self.last_uri.lock().unwrap() = Some(req.uri().clone());
        let status = self.canned_status;
        let body = self.canned_body.clone();
        Box::pin(async move {
            Ok(::http::Response::builder()
                .status(status)
                .body(Body::new(Full::new(body)))
                .expect("valid response"))
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn api_client_prefixes_base_url_and_decodes_ok() {
    let transport = RecordingTransport::new(200, Bytes::from(r#"{"id":"abc","name":"rex"}"#));
    let uri_tap = transport.last_uri.clone();
    let client = ApiClient::new(transport, "https://api.example.com");

    let req = GetPetRequest { id: "abc".into() };
    let resp = futures_executor::block_on(client.oneshot(req)).expect("call ok");

    match resp {
        GetPetResponse::Status200(pet) => {
            assert_eq!(pet.id, "abc");
            assert_eq!(pet.name, "rex");
        }
        other => panic!("unexpected response {other:?}"),
    }

    let uri = uri_tap.lock().unwrap().clone().expect("uri recorded");
    assert_eq!(uri.to_string(), "https://api.example.com/pets/abc");
}

#[test]
fn api_client_trims_trailing_slash_in_base_url() {
    let transport = RecordingTransport::new(404, Bytes::new());
    let uri_tap = transport.last_uri.clone();
    // base URL with trailing slash — must not double up.
    let client = ApiClient::new(transport, "https://api.example.com/");

    let req = GetPetRequest { id: "xyz".into() };
    let resp = futures_executor::block_on(client.oneshot(req)).expect("call ok");

    assert!(matches!(resp, GetPetResponse::Status404));
    assert_eq!(
        uri_tap.lock().unwrap().as_ref().unwrap().to_string(),
        "https://api.example.com/pets/xyz",
    );
}

#[test]
fn transport_error_is_wrapped() {
    // A transport that always fails with a string error.
    #[derive(Clone)]
    struct AlwaysFails;
    impl Service<Request> for AlwaysFails {
        type Response = Response;
        type Error = &'static str;
        type Future = std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
        >;
        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, _req: Request) -> Self::Future {
            Box::pin(async { Err("boom") })
        }
    }

    let client = ApiClient::new(AlwaysFails, "https://x");
    let err = futures_executor::block_on(client.oneshot(GetPetRequest { id: "x".into() }))
        .expect_err("transport should fail");

    match err {
        CallError::Transport(msg) => assert_eq!(msg, "boom"),
        CallError::Encode(_) => panic!("expected transport error"),
        CallError::Auth(_) => panic!("expected transport error"),
        CallError::Decode(_) => panic!("expected transport error"),
    }
}

// ---------------------------------------------------------------------------
// Auth integration: an op that declares a security requirement through
// `http::Extensions`, verifying `ApiClient::with_auth` injects the
// credential and that the default `NoAuth` rejects it.
// ---------------------------------------------------------------------------

use ::toac::{
    AuthSelector, OperationSecurity, SecurityCredential,
    security::{AuthFuture, BearerCredential},
};

#[derive(Debug, Clone)]
struct ProtectedRequest;

impl MakeRequest for ProtectedRequest {
    type Error = Infallible;

    fn make_request(
        self,
    ) -> impl ::std::future::Future<Output = Result<Request, Self::Error>> + Send {
        async move {
            let mut req = ::http::Request::builder()
                .method(Method::GET)
                .uri("/protected")
                .body(Body::empty())
                .expect("valid request");
            req.extensions_mut()
                .insert(OperationSecurity(&[&["bearer_scheme"]]));
            Ok(req)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ProtectedResponse {
    Status200,
}

impl ParseResponse for ProtectedResponse {
    type Error = DecodeError;

    fn parse_response<B>(
        response: ::http::Response<B>,
    ) -> impl ::std::future::Future<Output = Result<Self, Self::Error>> + Send
    where
        B: ::http_body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        async move {
            match response.status().as_u16() {
                200 => Ok(Self::Status200),
                other => Err(DecodeError::UnexpectedStatus(
                    ::http::StatusCode::from_u16(other).unwrap(),
                )),
            }
        }
    }
}

impl Operation for ProtectedRequest {
    type Response = ProtectedResponse;
}

/// Stand-in for a generated `AuthConfig` that owns a single bearer
/// credential and recognises its scheme name.
struct SingleBearer {
    scheme: &'static str,
    credential: BearerCredential,
}

impl AuthSelector for SingleBearer {
    fn apply_for(
        &self,
        req: Request,
        requirements: &'static [&'static [&'static str]],
    ) -> AuthFuture<'_> {
        Box::pin(async move {
            // Pick the first alternative where every required scheme
            // matches ours (for this single-scheme stand-in, that's the
            // trivial case of one name).
            for alt in requirements {
                if alt.iter().all(|name| *name == self.scheme) {
                    return self.credential.apply(req).await;
                }
            }
            Err(format!("no alternative matched scheme {}", self.scheme).into())
        })
    }
}

/// Records the Authorization header of the last request it saw.
#[derive(Clone)]
struct AuthHeaderTap {
    last_auth: Arc<Mutex<Option<String>>>,
}

impl AuthHeaderTap {
    fn new() -> Self {
        Self {
            last_auth: Arc::new(Mutex::new(None)),
        }
    }
}

impl Service<Request> for AuthHeaderTap {
    type Response = Response;
    type Error = Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        *self.last_auth.lock().unwrap() = req
            .headers()
            .get(::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        Box::pin(async move {
            Ok(::http::Response::builder()
                .status(200)
                .body(Body::empty())
                .expect("valid response"))
        })
    }
}

#[test]
fn with_auth_injects_credential_for_protected_op() {
    let transport = AuthHeaderTap::new();
    let tap = transport.last_auth.clone();
    let client = ApiClient::new(transport, "https://api.example.com").with_auth(SingleBearer {
        scheme: "bearer_scheme",
        credential: BearerCredential {
            token: "abc".into(),
        },
    });

    let resp = futures_executor::block_on(client.oneshot(ProtectedRequest)).expect("call ok");
    assert!(matches!(resp, ProtectedResponse::Status200));
    assert_eq!(tap.lock().unwrap().as_deref(), Some("Bearer abc"));
}

#[test]
fn default_no_auth_rejects_protected_op() {
    // No `.with_auth(...)` — the default `NoAuth` selector should fail
    // loudly rather than silently sending an unauthenticated request.
    let transport = AuthHeaderTap::new();
    let client = ApiClient::new(transport, "https://api.example.com");

    let err = futures_executor::block_on(client.oneshot(ProtectedRequest))
        .expect_err("protected op without auth should fail");
    assert!(matches!(err, CallError::Auth(_)), "got {err:?}");
}
