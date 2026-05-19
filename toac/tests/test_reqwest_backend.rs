//! End-to-end test for the `reqwest` backend.
//!
//! Wraps a [`reqwest::Client`] in [`toac::reqwest::ReqwestService`],
//! plugs it into [`toac::ApiClient`], and drives a hand-written
//! `Operation` against a [`wiremock`] mock server. Mirrors the
//! shape of code that `toac-build` emits, so this test doubles as a
//! smoke check for the runtime traits over a real HTTP loopback.
//!
//! `ParseResponse` returns `impl Future + Send`, which `async fn` cannot
//! spell on its own.

#![cfg(feature = "reqwest")]
#![allow(clippy::manual_async_fn)]

use std::convert::Infallible;

use ::bytes::Bytes;
use ::http::Method;
use ::toac::{
    ApiClient, BoxError, DecodeError, MakeRequest, Operation, ParseResponse, Request, body::Body,
    compat::reqwest::ReqwestService,
};
use ::tower::ServiceExt;
use ::wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[derive(Debug, Clone, PartialEq, ::serde::Deserialize)]
struct Pet {
    id: String,
    name: String,
}

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
            use ::http_body_util::BodyExt;
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

#[derive(Debug, Clone, ::serde::Serialize)]
struct CreatePetBody {
    name: String,
}

#[derive(Debug, Clone)]
struct CreatePetRequest {
    body: CreatePetBody,
}

impl MakeRequest for CreatePetRequest {
    type Error = ::serde_json::Error;

    fn make_request(
        self,
    ) -> impl ::std::future::Future<Output = Result<Request, Self::Error>> + Send {
        async move {
            let bytes = ::serde_json::to_vec(&self.body)?;
            let body = Body::new(::http_body_util::Full::new(Bytes::from(bytes)));
            Ok(::http::Request::builder()
                .method(Method::POST)
                .uri("/pets")
                .header(::http::header::CONTENT_TYPE, "application/json")
                .body(body)
                .expect("valid request"))
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum CreatePetResponse {
    Status201(Pet),
}

impl ParseResponse for CreatePetResponse {
    type Error = DecodeError;

    fn parse_response<B>(
        response: ::http::Response<B>,
    ) -> impl ::std::future::Future<Output = Result<Self, Self::Error>> + Send
    where
        B: ::http_body::Body<Data = Bytes> + Send + Sync + 'static,
        B::Error: Into<BoxError>,
    {
        async move {
            use ::http_body_util::BodyExt;
            let (parts, body) = response.into_parts();
            match parts.status.as_u16() {
                201 => {
                    let bytes = BodyExt::collect(body)
                        .await
                        .map_err(|e| DecodeError::Codec(e.into()))?
                        .to_bytes();
                    let pet = ::serde_json::from_slice(bytes.as_ref())
                        .map_err(|e| DecodeError::Codec(Box::new(e)))?;
                    Ok(Self::Status201(pet))
                }
                other => Err(DecodeError::UnexpectedStatus(
                    ::http::StatusCode::from_u16(other).expect("status from valid response"),
                )),
            }
        }
    }
}

impl Operation for CreatePetRequest {
    type Response = CreatePetResponse;
}

#[tokio::test(flavor = "multi_thread")]
async fn reqwest_backend_get_returns_decoded_payload() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/pets/abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "abc",
            "name": "rex",
        })))
        .mount(&server)
        .await;

    let transport = ReqwestService::new(reqwest::Client::new());
    let client = ApiClient::new(transport, server.uri());

    let resp = client
        .oneshot(GetPetRequest { id: "abc".into() })
        .await
        .expect("call ok");

    assert_eq!(
        resp,
        GetPetResponse::Status200(Pet {
            id: "abc".into(),
            name: "rex".into(),
        }),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reqwest_backend_404_maps_to_status_variant() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/pets/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    // Exercise the `new_reqwest` shortcut: takes a `reqwest::Client`
    // directly, no manual `ReqwestService` wrap.
    let client = ApiClient::new_reqwest(reqwest::Client::new(), server.uri());

    let resp = client
        .oneshot(GetPetRequest {
            id: "missing".into(),
        })
        .await
        .expect("call ok");

    assert_eq!(resp, GetPetResponse::Status404);
}

#[tokio::test(flavor = "multi_thread")]
async fn reqwest_backend_post_streams_request_body() {
    use wiremock::matchers::{body_json, header};

    let server = MockServer::start().await;
    let echo = serde_json::json!({ "id": "new", "name": "milo" });
    Mock::given(method("POST"))
        .and(path("/pets"))
        .and(header("content-type", "application/json"))
        .and(body_json(serde_json::json!({ "name": "milo" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(echo))
        .mount(&server)
        .await;

    let transport = ReqwestService::new(reqwest::Client::new());
    let client = ApiClient::new(transport, server.uri());

    let resp = client
        .oneshot(CreatePetRequest {
            body: CreatePetBody {
                name: "milo".into(),
            },
        })
        .await
        .expect("call ok");

    assert_eq!(
        resp,
        CreatePetResponse::Status201(Pet {
            id: "new".into(),
            name: "milo".into(),
        }),
    );
}
