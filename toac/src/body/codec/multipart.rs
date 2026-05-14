//! `multipart/form-data` codec. Request-body only — the wire format
//! is meant for uploads, no mainstream API returns it.
//!
//! The payload type is a hand-rolled [`MultipartForm`] rather than
//! something derived from `Serialize`: OAS multipart bodies carry
//! heterogeneous parts (JSON blob alongside a binary upload alongside
//! a plain field), each with its own `Content-Type`, so a flat
//! serde-shaped encoding can't capture everything. Callers (or the
//! generator) assemble parts explicitly, then the encoder writes the
//! canonical RFC 7578 byte sequence.
//!
//! Boundary generation uses a process-wide counter; it's not
//! cryptographic, just distinct enough that "legitimate payload bytes
//! match our boundary" is astronomically unlikely. Tests and recorded
//! integrations that need a stable value can pass their own through
//! [`MultipartEncoder::with_boundary`].

use std::sync::atomic::{AtomicU64, Ordering};

use bytes::{BufMut, Bytes, BytesMut};
use http::HeaderValue;
use http_body_util::Full;

use crate::body::{
    Body,
    codec::{BodyContentType, BodyEncoder},
};

/// One part of a multipart/form-data body.
#[derive(Clone, Debug)]
pub struct Part {
    /// `name=` value in the part's `Content-Disposition` header.
    pub name: String,
    /// Optional `filename=` value. Present when the part represents a
    /// file upload; absent for plain form fields.
    pub filename: Option<String>,
    /// `Content-Type` header for this part (e.g. `application/json`,
    /// `image/png`, `text/plain; charset=utf-8`). `None` means no
    /// header is emitted — RFC 7578 treats that as `text/plain`.
    pub content_type: Option<HeaderValue>,
    /// The part's raw bytes.
    pub body: Bytes,
}

impl Part {
    /// Creates a text-valued form field (no filename, `text/plain`).
    pub fn text<N, V>(name: N, value: V) -> Self
    where
        N: Into<String>,
        V: Into<String>,
    {
        Self {
            name: name.into(),
            filename: None,
            content_type: Some(HeaderValue::from_static("text/plain; charset=utf-8")),
            body: Bytes::from(value.into()),
        }
    }

    /// Creates a file upload part. `filename` is what the server sees
    /// in `Content-Disposition`; `content_type` is the file's MIME.
    pub fn file<N, F>(
        name: N,
        filename: F,
        content_type: HeaderValue,
        body: impl Into<Bytes>,
    ) -> Self
    where
        N: Into<String>,
        F: Into<String>,
    {
        Self {
            name: name.into(),
            filename: Some(filename.into()),
            content_type: Some(content_type),
            body: body.into(),
        }
    }

    /// Creates a part whose body is already encoded (e.g. a JSON blob).
    pub fn raw<N>(name: N, content_type: HeaderValue, body: impl Into<Bytes>) -> Self
    where
        N: Into<String>,
    {
        Self {
            name: name.into(),
            filename: None,
            content_type: Some(content_type),
            body: body.into(),
        }
    }
}

/// A sequence of parts ready to be encoded. Construct through
/// [`MultipartForm::builder`] or [`MultipartForm::from_parts`].
#[derive(Clone, Debug, Default)]
pub struct MultipartForm {
    parts: Vec<Part>,
}

impl MultipartForm {
    /// Fluent entry point.
    pub fn builder() -> MultipartFormBuilder {
        MultipartFormBuilder::default()
    }

    /// Direct constructor when callers already have the parts list.
    pub fn from_parts(parts: Vec<Part>) -> Self {
        Self { parts }
    }

    /// Read-only access to the parts.
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }
}

/// Builder that accumulates parts before freezing into a
/// [`MultipartForm`].
#[derive(Clone, Debug, Default)]
pub struct MultipartFormBuilder {
    parts: Vec<Part>,
}

impl MultipartFormBuilder {
    pub fn part(mut self, part: Part) -> Self {
        self.parts.push(part);
        self
    }

    pub fn text<N, V>(self, name: N, value: V) -> Self
    where
        N: Into<String>,
        V: Into<String>,
    {
        self.part(Part::text(name, value))
    }

    pub fn file<N, F>(
        self,
        name: N,
        filename: F,
        content_type: HeaderValue,
        body: impl Into<Bytes>,
    ) -> Self
    where
        N: Into<String>,
        F: Into<String>,
    {
        self.part(Part::file(name, filename, content_type, body))
    }

    pub fn build(self) -> MultipartForm {
        MultipartForm { parts: self.parts }
    }
}

/// Encoder for [`MultipartForm`].
///
/// Each encoder carries the boundary as part of its state — both
/// `content_type()` and `encode()` need to agree on the same string,
/// and the trait is `&self`, so we can't generate on the fly. Call
/// [`MultipartEncoder::new`] (auto boundary) or
/// [`MultipartEncoder::with_boundary`] (explicit) to get a one-shot
/// encoder for a specific request.
#[derive(Clone, Debug)]
pub struct MultipartEncoder {
    boundary: String,
}

impl Default for MultipartEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl MultipartEncoder {
    /// Generates a fresh boundary.
    pub fn new() -> Self {
        Self {
            boundary: next_boundary(),
        }
    }

    /// Builds an encoder with a caller-supplied boundary. The caller
    /// must ensure the boundary doesn't appear in any part's body
    /// bytes — this is standard multipart caveat.
    pub fn with_boundary(boundary: impl Into<String>) -> Self {
        Self {
            boundary: boundary.into(),
        }
    }

    /// Returns the boundary this encoder will use on the wire.
    pub fn boundary(&self) -> &str {
        &self.boundary
    }
}

static BOUNDARY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_boundary() -> String {
    let n = BOUNDARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("----toac-boundary-{n:016x}")
}

impl BodyContentType for MultipartEncoder {
    fn content_type(&self) -> HeaderValue {
        let raw = format!("multipart/form-data; boundary={}", self.boundary);
        HeaderValue::try_from(raw).expect("boundary is ASCII by construction")
    }
}

impl BodyEncoder<&MultipartForm> for MultipartEncoder {
    type Error = std::convert::Infallible;

    fn encode(&self, data: &MultipartForm) -> Result<Body, Self::Error> {
        let bytes = render_parts(&self.boundary, data.parts());
        Ok(Body::new(Full::new(bytes)))
    }
}

fn render_parts(boundary: &str, parts: &[Part]) -> Bytes {
    let mut out = BytesMut::with_capacity(256 + parts.iter().map(|p| p.body.len()).sum::<usize>());
    for part in parts {
        out.put(b"--".as_slice());
        out.put(boundary.as_bytes());
        out.put(b"\r\n".as_slice());
        out.put(b"Content-Disposition: form-data; name=\"".as_slice());
        out.put(escape_quoted(&part.name).as_bytes());
        out.put(b"\"".as_slice());
        if let Some(filename) = part.filename.as_deref() {
            out.put(b"; filename=\"".as_slice());
            out.put(escape_quoted(filename).as_bytes());
            out.put(b"\"".as_slice());
        }
        out.put(b"\r\n".as_slice());
        if let Some(ct) = part.content_type.as_ref() {
            out.put(b"Content-Type: ".as_slice());
            out.put(ct.as_bytes());
            out.put(b"\r\n".as_slice());
        }
        out.put(b"\r\n".as_slice());
        out.put(part.body.as_ref());
        out.put(b"\r\n".as_slice());
    }
    out.put(b"--".as_slice());
    out.put(boundary.as_bytes());
    out.put(b"--\r\n".as_slice());
    out.freeze()
}

/// Escapes `"` and CR/LF inside `name=` / `filename=` values so the
/// Content-Disposition header stays parseable. Real-world servers
/// vary in strictness; this is the minimal RFC 7578 interpretation.
fn escape_quoted(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '"' => out.push_str("%22"),
            '\r' => out.push_str("%0D"),
            '\n' => out.push_str("%0A"),
            c => out.push(c),
        }
    }
    out
}
