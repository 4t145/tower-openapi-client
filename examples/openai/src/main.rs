//! OpenAI example: drives generated operation types against
//! `https://api.openai.com/v1`, exercising the three response shapes
//! the upstream spec exposes:
//!
//! * `application/json`     — `GET /models` and the non-streaming branch
//!                            of `POST /chat/completions`
//! * `multipart/form-data`  — `POST /audio/transcriptions`
//! * `text/event-stream`    — the streaming branch of
//!                            `POST /chat/completions`
//!
//! Authentication is the spec's `ApiKeyAuth` (HTTP Bearer); the
//! generator emits an `AuthConfig` with an `api_key_auth` setter that
//! `ApiClient::with_auth` consumes. The token is read from
//! `OPENAI_API_KEY`.

use std::{env, path::PathBuf};

use bytes::Bytes;
use futures_util::StreamExt;
use http::HeaderValue;
use openai_example::{
    components::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessage,
        ChatCompletionRequestUserMessageRole, ChatCompletionResponseMessageContent,
        CreateChatCompletionResponse, CreateTranscriptionResponseJson,
    },
    operations::{
        audio::transcriptions::post as create_transcription,
        chat::completions::post as create_chat_completion, models::get as list_models,
    },
    security::AuthConfig,
};
use toac::{
    ApiClient,
    body::codec::multipart::{MultipartForm, Part},
};
use tower::Service;
use tracing::{error, info, warn};

/// `Accept` value used by the streaming chat completion demo. The chat
/// completion endpoint declares both `application/json` and
/// `text/event-stream` under the same status, so the codegen exposes
/// two response variants — picking SSE on the wire is what makes the
/// server reply with the streaming branch.
const SSE_ACCEPT: &str = "text/event-stream";

/// OpenAI's public API root. The spec's `servers[0].url` already points
/// here, but `ApiClient::new` wants it explicit.
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// Environment variable carrying the bearer credential.
const API_KEY_ENV: &str = "OPENAI_API_KEY";
/// Optional base-URL override for proxies / mocks.
const API_URL_ENV: &str = "OPENAI_API_URL";
/// Optional path to an audio file used by the transcription demo. When
/// unset the demo is skipped — multipart still has to put real bytes on
/// the wire, so a stub file would just produce a server-side error.
const AUDIO_FILE_ENV: &str = "OPENAI_AUDIO_FILE";

/// Default model used by the chat-completion demos. Cheap, widely
/// available, and supports streaming.
const CHAT_MODEL: &str = "gpt-4o-mini";

/// Transcription model that supports the standard `json` response
/// format used below.
const TRANSCRIPTION_MODEL: &str = "gpt-4o-transcribe";

/// Cap on streamed deltas printed to the log so the SSE demo doesn't
/// flood stdout for long completions.
const MAX_STREAM_EVENTS: usize = 32;

type HttpClient = client_util::client::HyperHttpsClient<toac::body::Body>;
type OpenAiClient = ApiClient<HttpClient>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let Ok(token) = env::var(API_KEY_ENV) else {
        error!("set {API_KEY_ENV} to an OpenAI API key before running this example");
        return Ok(());
    };

    let base_url = env::var(API_URL_ENV).ok().unwrap_or_else(|| {
        info!("using default API URL: {OPENAI_BASE_URL}");
        OPENAI_BASE_URL.to_string()
    });

    let auth = AuthConfig::builder().api_key_auth(token).build();
    let http = client_util::client::build_https_client::<toac::body::Body>()?;
    let mut client: OpenAiClient = ApiClient::new(http, base_url).with_auth(auth);

    demo_list_models(&mut client).await;
    demo_chat_completion_json(&mut client).await;
    demo_chat_completion_stream(&mut client).await;
    demo_transcription(&mut client).await;
    Ok(())
}

/// GET /models — plain JSON list, the simplest sanity check that auth
/// + transport are wired correctly.
async fn demo_list_models(client: &mut OpenAiClient) {
    info!("GET /models");
    match client.call(list_models::Request {}).await {
        Ok(resp) => match resp.body {
            list_models::ResponseBody::Status200(list) => {
                info!(count = list.data.len(), "models returned");
                for model in list.data.iter().take(5) {
                    info!(id = %model.id, owned_by = %model.owned_by, "model");
                }
                if list.data.len() > 5 {
                    info!(omitted = list.data.len() - 5, "… remaining models elided");
                }
            }
        },
        Err(err) => report_call_error("listModels", &err),
    }
}

/// POST /chat/completions — JSON request, JSON response. `stream` is
/// left unset so the server picks the non-streaming branch.
async fn demo_chat_completion_json(client: &mut OpenAiClient) {
    info!("POST /chat/completions (json)");
    let request = create_chat_completion::Request {
        body: chat_request_body(false),
    };

    match client.call(request).await {
        Ok(resp) => match resp.body {
            create_chat_completion::ResponseBody::Status200Json(payload) => {
                log_chat_response(&payload)
            }
            create_chat_completion::ResponseBody::Status200Sse(_) => {
                info!("server returned SSE despite the json default");
            }
        },
        Err(err) => report_call_error("createChatCompletion", &err),
    }
}

/// POST /chat/completions — same operation, but with `stream=true`. The
/// server replies with `text/event-stream`; the generator selects the
/// SSE codec and decodes the body into an `SseEventStream`.
///
/// The op declares both `application/json` and `text/event-stream` for
/// `200`, so the auto-emitted `Accept` header lists both. We use
/// [`Request::with_accept`] to narrow it to `text/event-stream` and
/// steer content negotiation onto the streaming branch.
async fn demo_chat_completion_stream(client: &mut OpenAiClient) {
    info!("POST /chat/completions (sse)");
    let request = create_chat_completion::Request {
        body: chat_request_body(true),
    }
    .with_accept(HeaderValue::from_static(SSE_ACCEPT));

    let response = match client.call(request).await {
        Ok(resp) => resp,
        Err(err) => {
            report_call_error("createChatCompletion (stream)", &err);
            return;
        }
    };

    match response.body {
        create_chat_completion::ResponseBody::Status200Sse(stream) => {
            log_sse_stream(stream).await;
        }
        create_chat_completion::ResponseBody::Status200Json(_) => {
            info!("server fell back to the json branch despite Accept: {SSE_ACCEPT}");
        }
    }
}

/// Pulls a bounded number of events off the SSE stream and logs each
/// one's data field. Stops at [`MAX_STREAM_EVENTS`] so the demo doesn't
/// flood stdout for long completions.
async fn log_sse_stream(mut stream: toac::body::codec::sse::SseEventStream) {
    let mut printed = 0usize;
    while let Some(event) = stream.next().await {
        if printed >= MAX_STREAM_EVENTS {
            info!(printed, "stream cap reached, stopping");
            break;
        }
        match event {
            Ok(sse) => {
                if let Some(data) = sse.data.as_deref() {
                    info!(seq = printed, data, "sse event");
                }
            }
            Err(err) => {
                warn!(error = %err, "sse event decode error");
                break;
            }
        }
        printed += 1;
    }
}

/// POST /audio/transcriptions — exercises the multipart codec. The
/// generator emits `body: MultipartForm`; the caller assembles parts
/// directly so heterogeneous content (file bytes + plain text fields)
/// can coexist in one request.
async fn demo_transcription(client: &mut OpenAiClient) {
    let Ok(audio_path) = env::var(AUDIO_FILE_ENV) else {
        info!("{AUDIO_FILE_ENV} unset; skipping multipart transcription demo");
        return;
    };
    let path = PathBuf::from(&audio_path);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => Bytes::from(bytes),
        Err(err) => {
            warn!(path = %path.display(), error = %err, "could not read audio file");
            return;
        }
    };
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("audio.bin")
        .to_string();
    let content_type = guess_audio_mime(&path);

    info!(file = %path.display(), "POST /audio/transcriptions");
    let body = MultipartForm::builder()
        .part(Part::file("file", filename, content_type, bytes))
        .text("model", TRANSCRIPTION_MODEL)
        .text("response_format", "json")
        .build();

    let request = create_transcription::Request { body };
    match client.call(request).await {
        Ok(resp) => match resp.body {
            create_transcription::ResponseBody::Status200Json(payload) => {
                log_transcription_json(&payload)
            }
            create_transcription::ResponseBody::Status200Sse(_) => {
                info!("server picked the streaming branch; not handled in this demo");
            }
        },
        Err(err) => report_call_error("createTranscription", &err),
    }
}

/// Constructs a minimal valid `CreateChatCompletionRequest`. The spec
/// only requires `model` + `messages`; everything else is `Option<_>`
/// and is set to `None` here. The function is shared by the JSON and
/// SSE demos — only the `stream` flag differs.
#[allow(deprecated)]
fn chat_request_body(stream: bool) -> openai_example::components::CreateChatCompletionRequest {
    use openai_example::components as c;

    let user_msg = ChatCompletionRequestMessage::ChatCompletionRequestUserMessage(
        ChatCompletionRequestUserMessage {
            content: "Say hi in one short sentence.".to_string().into(),
            name: None,
            role: ChatCompletionRequestUserMessageRole::User,
        },
    );

    c::CreateChatCompletionRequest {
        audio: None,
        frequency_penalty: None,
        function_call: None,
        functions: None,
        logit_bias: None,
        logprobs: None,
        max_completion_tokens: None,
        max_tokens: None,
        messages: vec![user_msg],
        metadata: None,
        modalities: None,
        model: CHAT_MODEL.to_string().into(),
        n: None,
        parallel_tool_calls: None,
        prediction: None,
        presence_penalty: None,
        prompt_cache_key: None,
        prompt_cache_retention: None,
        reasoning_effort: None,
        response_format: None,
        safety_identifier: None,
        seed: None,
        service_tier: None,
        stop: None,
        store: None,
        stream: Some(stream),
        stream_options: None,
        temperature: None,
        tool_choice: None,
        tools: None,
        top_logprobs: None,
        top_p: None,
        user: None,
        verbosity: None,
        web_search_options: None,
    }
}

/// Logs the first choice's text for the JSON branch.
fn log_chat_response(resp: &CreateChatCompletionResponse) {
    let Some(first) = resp.choices.first() else {
        info!("model returned no choices");
        return;
    };
    match &first.message.content {
        ChatCompletionResponseMessageContent::ChatCompletionResponseMessageContentVariant0(
            text,
        ) => {
            info!(model = %resp.model, content = %text, "chat reply");
        }
        ChatCompletionResponseMessageContent::ChatCompletionResponseMessageContentVariant1(_) => {
            info!(model = %resp.model, "chat reply (non-text content)");
        }
    }
}

fn log_transcription_json(body: &create_transcription::ResponseStatus200JsonBody) {
    use create_transcription::ResponseStatus200JsonBody as B;
    match body {
        B::CreateTranscriptionResponseJson(plain) => log_transcription(plain),
        B::CreateTranscriptionResponseDiarizedJson(diarized) => {
            info!(
                segments = diarized.segments.len(),
                "transcription (diarized)"
            );
        }
        B::CreateTranscriptionResponseVerboseJson(verbose) => {
            info!(text = %verbose.text, "transcription (verbose)");
        }
    }
}

fn log_transcription(resp: &CreateTranscriptionResponseJson) {
    info!(text = %resp.text, "transcription");
}

/// Picks a coarse audio MIME from the file extension. Wrong guesses
/// don't break the request — the server inspects the bytes — but the
/// header still goes on the wire, so do something reasonable.
fn guess_audio_mime(path: &std::path::Path) -> HeaderValue {
    const FALLBACK: &str = "application/octet-stream";
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    let mime = match ext.as_deref() {
        Some("mp3") => "audio/mpeg",
        Some("mp4" | "m4a") => "audio/mp4",
        Some("wav") => "audio/wav",
        Some("flac") => "audio/flac",
        Some("ogg") => "audio/ogg",
        Some("webm") => "audio/webm",
        _ => FALLBACK,
    };
    HeaderValue::from_static(mime)
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
