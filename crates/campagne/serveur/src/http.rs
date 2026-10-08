//! The HTTP surface: `GET /health`, on the loopback interface, and nothing else.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};

use axum::Router;
use axum::routing::get;
use tokio::net::TcpListener;

/// `GET /health` answers a constant `ok`. Any other path is 404, any other
/// method on `/health` is 405.
pub fn router() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}

/// Binds `127.0.0.1:port`. There is no address parameter: the server never
/// listens beyond loopback.
pub async fn bind_loopback(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
}

pub async fn serve(
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router())
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
