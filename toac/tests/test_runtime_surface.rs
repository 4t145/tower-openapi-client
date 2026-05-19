//! Exercises the runtime traits against a hand-written mirror of what
//! the generator emits. This confirms `MakeRequest` / `ParseResponse`
//! are usable in isolation — the shape tests in
//! `test_runtime_codegen.rs` cover the generator's output form.
//!
//! The `manual_async_fn` lint is intentionally silenced: the trait
//! signatures use `impl Future + Send`, which `async fn` would not
//! produce — the extra `+ Send` bound is load-bearing.

#![allow(clippy::manual_async_fn)]

use ::bytes::Bytes;
use ::http_body_util::{BodyExt, Full};
use ::toac::{BoxError, DecodeError, MakeRequest, ParseResponse, Request, body::Body};

// Hand-written mirror of a GET with one path param, one optional query
// param, and one optional header.
#[derive(Debug, Clone, PartialEq)]
pub struct GetPetRequest {
    pub id: String,
    pub limit: Option<i64>,
    pub x_trace: Option<String>,
}

impl MakeRequest for GetPetRequest {
    type Error = ::std::convert::Infallible;

    fn make_request(
        self,
    ) -> impl ::std::future::Future<Output = Result<Request, Self::Error>> + Send {
        async move {
            let mut path = String::new();
            path.push_str("/pets/");
            path.push_str(&ToString::to_string(&self.id));
            let mut query_first = true;
            if let Some(v) = &self.limit {
                let sep = if query_first { '?' } else { '&' };
                query_first = false;
                path.push(sep);
                path.push_str("limit");
                path.push('=');
                path.push_str(&ToString::to_string(v));
            }
            let mut builder = ::http::Request::builder()
                .method(::http::Method::GET)
                .uri(path);
            if let Some(v) = &self.x_trace {
                builder = builder.header("X-Trace", ToString::to_string(v));
            }
            let _ = query_first;
            Ok(builder.body(Body::empty()).expect("valid request"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, ::serde::Deserialize)]
pub struct Pet {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GetPetResponse {
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
            let bytes = BodyExt::collect(body)
                .await
                .map_err(|e| DecodeError::Codec(e.into()))?
                .to_bytes();
            match parts.status.as_u16() {
                200 => {
                    let v = ::serde_json::from_slice(bytes.as_ref())
                        .map_err(|e| DecodeError::Codec(Box::new(e)))?;
                    Ok(Self::Status200(v))
                }
                404 => Ok(Self::Status404),
                _ => Err(DecodeError::UnexpectedStatus(parts.status)),
            }
        }
    }
}

// --- Body-carrying variant ---

#[derive(Debug, Clone, PartialEq, ::serde::Serialize)]
pub struct NewPet {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreatePetRequest {
    pub body: NewPet,
}

impl MakeRequest for CreatePetRequest {
    type Error = ::serde_json::Error;

    fn make_request(
        self,
    ) -> impl ::std::future::Future<Output = Result<Request, Self::Error>> + Send {
        async move {
            let bytes = ::serde_json::to_vec(&self.body)?;
            Ok(::http::Request::builder()
                .method(::http::Method::POST)
                .uri("/pets")
                .body(Body::new(Full::new(Bytes::from(bytes))))
                .expect("valid request"))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn request_with_path_query_header() {
    let req = GetPetRequest {
        id: "abc".into(),
        limit: Some(10),
        x_trace: Some("t1".into()),
    };
    let http_req = futures_executor::block_on(req.make_request()).expect("make_request");
    assert_eq!(http_req.method(), ::http::Method::GET);
    let uri = http_req.uri().to_string();
    assert_eq!(uri, "/pets/abc?limit=10");
    assert_eq!(
        http_req
            .headers()
            .get("X-Trace")
            .map(|v| v.to_str().unwrap()),
        Some("t1"),
    );
}

#[test]
fn request_body_serialises_to_json() {
    let req = CreatePetRequest {
        body: NewPet { name: "rex".into() },
    };
    let http_req = futures_executor::block_on(req.make_request()).expect("make_request");
    let (parts, body) = http_req.into_parts();
    assert_eq!(parts.method, ::http::Method::POST);
    let collected = futures_executor::block_on(body.collect())
        .expect("collect body")
        .to_bytes();
    let parsed: ::serde_json::Value = ::serde_json::from_slice(&collected).unwrap();
    assert_eq!(parsed["name"], "rex");
}

#[test]
fn response_decodes_known_statuses() {
    let ok = ::http::Response::builder()
        .status(200)
        .body(Body::new(Full::new(Bytes::from(
            r#"{"id":"abc","name":"rex"}"#,
        ))))
        .unwrap();
    let decoded = futures_executor::block_on(GetPetResponse::parse_response(ok)).expect("ok");
    match decoded {
        GetPetResponse::Status200(p) => assert_eq!(p.name, "rex"),
        other => panic!("unexpected {other:?}"),
    }

    let not_found = ::http::Response::builder()
        .status(404)
        .body(Body::empty())
        .unwrap();
    let decoded = futures_executor::block_on(GetPetResponse::parse_response(not_found));
    assert!(matches!(decoded, Ok(GetPetResponse::Status404)));
}

#[test]
fn response_unknown_status_errors() {
    let resp = ::http::Response::builder()
        .status(500)
        .body(Body::empty())
        .unwrap();
    let decoded = futures_executor::block_on(GetPetResponse::parse_response(resp));
    assert!(matches!(decoded, Err(DecodeError::UnexpectedStatus(_))));
}
