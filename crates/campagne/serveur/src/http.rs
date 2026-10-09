//! The HTTP surface, on the loopback interface only:
//!
//! - `GET /health` answers a constant `ok`;
//! - `POST /commands` queues a command and answers at once (202), never
//!   waiting for it to apply;
//! - `GET /commands/{commandId}` answers `{"entry", "messages"}` while the
//!   command is pending (contracts G and L), `{"result"}` once it is settled
//!   (contract H);
//! - `GET /data-version` is a server-sent event stream of the global
//!   `dataVersion`: the current one first, then each greater one.
//!
//! An error answers a stable id, never a driver's or a parser's text.

use std::convert::Infallible;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use dataguard::{CommandId, Engine, Submission, SubmitError};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;
use tracing::warn;

/// `GET /health` answers a constant `ok`. Any other path is 404, any other
/// method on `/health` is 405.
pub fn router() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    /// The global `dataVersion`, fed by `LISTEN/NOTIFY`.
    pub versions: watch::Receiver<i64>,
}

/// `/health` and the engine's three routes.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/commands", post(submit))
        .route("/commands/{command}", get(lookup))
        .route("/data-version", get(data_version))
        .with_state(state)
        .merge(router())
}

fn error(status: StatusCode, id: &str) -> Response {
    (status, Json(json!({ "error": id }))).into_response()
}

async fn submit(State(state): State<AppState>, body: Bytes) -> Response {
    let Ok(submission) = serde_json::from_slice::<Submission>(&body) else {
        return error(StatusCode::BAD_REQUEST, "malformed-submission");
    };
    match state.engine.submit(submission).await {
        Ok(submitted) => (StatusCode::ACCEPTED, Json(submitted)).into_response(),
        Err(SubmitError::Internal(code)) => {
            warn!(sqlstate = %code, "submission failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
        Err(refused) => error(StatusCode::BAD_REQUEST, refused.id()),
    }
}

async fn lookup(State(state): State<AppState>, Path(command): Path<String>) -> Response {
    let Ok(command) = command.parse::<CommandId>() else {
        return error(StatusCode::NOT_FOUND, "not-found");
    };
    match state.engine.lookup(command).await {
        Ok(Some(found)) => Json(found).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "not-found"),
        Err(e) => {
            warn!(error = %e, "lookup failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

/// The current version first, then every greater one; never row data.
async fn data_version(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream = WatchStream::new(state.versions).map(|v| {
        Ok(Event::default()
            .event("dataVersion")
            .data(json!({ "dataVersion": v }).to_string()))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Binds `127.0.0.1:port`. There is no address parameter: the server never
/// listens beyond loopback.
pub async fn bind_loopback(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
}

pub async fn serve(
    listener: TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(method: Method, path: &str) -> (StatusCode, String, Option<String>) {
        let response = router()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_owned());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            String::from_utf8(body.to_vec()).unwrap(),
            content_type,
        )
    }

    #[tokio::test]
    async fn health_is_ok_and_constant() {
        let (status, body, content_type) = call(Method::GET, "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");
        assert!(content_type.unwrap().starts_with("text/plain"));
    }

    #[tokio::test]
    async fn other_paths_are_not_found() {
        for path in ["/", "/api", "/health/x", "/healthz"] {
            assert_eq!(
                call(Method::GET, path).await.0,
                StatusCode::NOT_FOUND,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn other_methods_on_health_are_not_allowed() {
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            assert_eq!(
                call(method.clone(), "/health").await.0,
                StatusCode::METHOD_NOT_ALLOWED,
                "{method}"
            );
        }
    }

    #[tokio::test]
    async fn binds_loopback_only() {
        let listener = bind_loopback(0).await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert_eq!(addr.ip(), std::net::IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[tokio::test]
    async fn port_in_use_is_an_error() {
        let held = bind_loopback(0).await.unwrap();
        let port = held.local_addr().unwrap().port();
        let err = bind_loopback(port).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    }
}
