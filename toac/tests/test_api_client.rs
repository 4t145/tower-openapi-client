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
    fn make_request(self) -> impl ::std::future::Future<Output = Request> + Send {
        async move {
            let uri = format!("/pets/{}", self.id);
            ::http::Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .expect("valid request")
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
        B: ::http_body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<BoxError>,
    {
        async move {
            let (parts, body) = response.into_parts();
            match parts.status.as_u16() {
                200 => {
                    let bytes = BodyExt::collect(body)
                        .await
                        .map_err(|e| DecodeError::BodyRead(e.into()))?
                        .to_bytes();
                    let pet = ::serde_json::from_slice(bytes.as_ref())?;
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
        CallError::Decode(_) => panic!("expected transport error"),
    }
}
