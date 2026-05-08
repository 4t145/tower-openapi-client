//! Unit tests for the built-in `SecurityCredential` implementations
//! and `NoAuth`'s pass-through / reject behaviour.

#![allow(clippy::manual_async_fn)]

use ::http_body_util::Full;
use ::toac::security::{ApiKeyCredential, ApiKeyLocation};
use ::toac::{
    AuthSelector, BoxError, NoAuth, Request, SecurityCredential,
    body::Body,
    security::{BasicCredential, BearerCredential},
};

fn empty_request(uri: &str) -> Request {
    ::http::Request::builder()
        .method(::http::Method::GET)
        .uri(uri)
        .body(Body::new(Full::<::bytes::Bytes>::new(Default::default())))
        .unwrap()
}

#[test]
fn api_key_header_sets_named_header() {
    let cred = ApiKeyCredential {
        name: "X-API-Key",
        location: ApiKeyLocation::Header,
        value: "sk-secret".into(),
    };
    let req = futures_executor::block_on(cred.apply(empty_request("/pets"))).unwrap();
    assert_eq!(
        req.headers().get("X-API-Key").and_then(|v| v.to_str().ok()),
        Some("sk-secret"),
    );
}

#[test]
fn api_key_query_appends_to_existing_query() {
    let cred = ApiKeyCredential {
        name: "key",
        location: ApiKeyLocation::Query,
        value: "abc 123".into(),
    };
    let req = futures_executor::block_on(cred.apply(empty_request("/pets?limit=10"))).unwrap();
    let uri = req.uri().to_string();
    assert!(uri.starts_with("/pets?limit=10&key="), "uri was {uri}");
    assert!(
        uri.contains("abc%20123"),
        "space not percent-encoded: {uri}"
    );
}

#[test]
fn api_key_query_uses_question_mark_when_no_existing_query() {
    let cred = ApiKeyCredential {
        name: "key",
        location: ApiKeyLocation::Query,
        value: "v".into(),
    };
    let req = futures_executor::block_on(cred.apply(empty_request("/pets"))).unwrap();
    assert_eq!(req.uri().to_string(), "/pets?key=v");
}

#[test]
fn api_key_cookie_merges_with_existing_cookie_header() {
    let cred = ApiKeyCredential {
        name: "session",
        location: ApiKeyLocation::Cookie,
        value: "xyz".into(),
    };
    let mut req = empty_request("/pets");
    req.headers_mut()
        .insert(::http::header::COOKIE, "foo=bar".try_into().unwrap());
    let req = futures_executor::block_on(cred.apply(req)).unwrap();
    assert_eq!(
        req.headers().get(::http::header::COOKIE).unwrap(),
        "foo=bar; session=xyz"
    );
}

#[test]
fn bearer_sets_authorization_header() {
    let cred = BearerCredential {
        token: "token-123".into(),
    };
    let req = futures_executor::block_on(cred.apply(empty_request("/pets"))).unwrap();
    assert_eq!(
        req.headers().get(::http::header::AUTHORIZATION).unwrap(),
        "Bearer token-123"
    );
}

#[test]
fn basic_encodes_credentials_as_base64() {
    let cred = BasicCredential {
        username: "alice".into(),
        password: "wonderland".into(),
    };
    let req = futures_executor::block_on(cred.apply(empty_request("/pets"))).unwrap();
    // alice:wonderland -> YWxpY2U6d29uZGVybGFuZA==
    assert_eq!(
        req.headers().get(::http::header::AUTHORIZATION).unwrap(),
        "Basic YWxpY2U6d29uZGVybGFuZA=="
    );
}

#[test]
fn no_auth_passes_through_public_endpoints() {
    let req = empty_request("/pets");
    let result: Result<Request, BoxError> = futures_executor::block_on(NoAuth.apply_for(req, &[]));
    assert!(result.is_ok(), "public endpoint should pass through");
}

#[test]
fn no_auth_rejects_protected_endpoints() {
    let req = empty_request("/pets");
    let result: Result<Request, BoxError> =
        futures_executor::block_on(NoAuth.apply_for(req, &[&["api_key"]]));
    let err = result.expect_err("NoAuth should reject protected endpoints");
    let msg = err.to_string();
    assert!(
        msg.contains("api_key") && msg.contains("with_auth"),
        "error should mention missing scheme + how to fix: {msg}"
    );
}
