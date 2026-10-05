//! Shared reqwest plumbing for the reqwest-backed platform adapters.
//!
//! Auth header *schemes* differ per platform (`Authorization: token …` for
//! Gitea, `PRIVATE-TOKEN` for GitLab), so request construction deliberately
//! stays inside each adapter — only transport helpers live here.
//!
//! Also hosts the per-host [`cached_client`] pool: the app rebuilds its
//! platform client on every refresh, but the underlying `reqwest::Client`
//! (connection pool + TLS session cache) is created once per host.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use reqwest::{Client, RequestBuilder};
use serde_json::Value;

use crate::platform::error::PlatformError;

/// Per-host client cache, lazily initialized once per process.
static CLIENTS: OnceLock<Mutex<HashMap<String, Arc<Client>>>> = OnceLock::new();

fn client_store() -> &'static Mutex<HashMap<String, Arc<Client>>> {
    CLIENTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Lock the client cache, recovering from a poisoned mutex.
fn lock_clients() -> MutexGuard<'static, HashMap<String, Arc<Client>>> {
    client_store().lock().unwrap_or_else(|e| e.into_inner())
}

/// The cached `reqwest::Client` for `host`, built on first use.
///
/// Repeated calls for the same host return the *same* client (see the unit
/// test), so connection pooling survives app refreshes.
pub(crate) fn cached_client(
    platform: &'static str,
    host: &str,
) -> Result<Arc<Client>, PlatformError> {
    let mut store = lock_clients();
    if let Some(client) = store.get(host) {
        return Ok(client.clone());
    }
    let client = Client::builder()
        .user_agent("gitpulse")
        .build()
        .map_err(|e| PlatformError::Other {
            platform,
            message: e.to_string(),
        })?;
    let client = Arc::new(client);
    store.insert(host.to_string(), client.clone());
    Ok(client)
}

/// A decoded JSON body plus the pagination cursor GitLab reports in
/// `x-next-page` (empty on the last page).
pub(crate) struct JsonPage {
    /// Decoded JSON body.
    pub body: Value,
    /// Raw `x-next-page` value; `None` when absent or empty.
    pub next_page: Option<String>,
}

/// Execute `req`, returning the JSON body plus `x-next-page`, or mapping the
/// failure to [`PlatformError`] tagged with `platform`.
pub(crate) async fn fetch_page(
    platform: &'static str,
    req: RequestBuilder,
) -> Result<JsonPage, PlatformError> {
    let resp = req
        .send()
        .await
        .map_err(|e| reqwest_to_platform(platform, e))?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let message = resp.text().await.unwrap_or_default();
        return Err(PlatformError::Http {
            status,
            message,
            rate_limited: status == 429,
            platform,
        });
    }
    let next_page = resp
        .headers()
        .get("x-next-page")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    let body = resp
        .json::<Value>()
        .await
        .map_err(|e| PlatformError::Other {
            platform,
            message: e.to_string(),
        })?;
    Ok(JsonPage { body, next_page })
}

/// Execute `req`, returning just the JSON body.
pub(crate) async fn fetch_json(
    platform: &'static str,
    req: RequestBuilder,
) -> Result<Value, PlatformError> {
    fetch_page(platform, req).await.map(|p| p.body)
}

/// Convert a `reqwest::Error` into a `PlatformError`.
///
/// Connect/timeout/build errors → `Network`; everything else (including decode
/// and JSON errors) → `Other`.
pub(crate) fn reqwest_to_platform(platform: &'static str, e: reqwest::Error) -> PlatformError {
    if e.is_connect() || e.is_timeout() || e.is_builder() {
        PlatformError::Network(e.to_string())
    } else {
        PlatformError::Other {
            platform,
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve exactly one canned HTTP response on a random localhost port and
    /// return the origin (`http://127.0.0.1:<port>`).
    fn spawn_server(response: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost");
        let addr = listener.local_addr().expect("local addr");
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf);
                let _ = sock.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// A minimal `200 OK` JSON response.
    fn ok_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
            .block_on(f)
    }

    #[test]
    fn cached_client_reuses_the_same_instance_per_host() {
        let a = cached_client("Gitea", "cache-test.example").expect("client");
        let b = cached_client("Gitea", "cache-test.example").expect("client");
        assert!(Arc::ptr_eq(&a, &b), "same host must hit the cache");

        let other = cached_client("GitLab", "other-cache-test.example").expect("client");
        assert!(
            !Arc::ptr_eq(&a, &other),
            "different hosts must not share a client"
        );
    }

    #[test]
    fn fetch_json_decodes_success_body() {
        let origin = spawn_server(ok_response(r#"{"hello":"world"}"#));
        let client = Client::new();
        let value = block_on(fetch_json(
            "Gitea",
            client.get(format!("{origin}/api/v1/repos/x/y")),
        ))
        .expect("json");
        assert_eq!(value["hello"], "world");
    }

    #[test]
    fn fetch_json_maps_http_error_status() {
        let origin = spawn_server(
            "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found"
                .to_string(),
        );
        let client = Client::new();
        let err = block_on(fetch_json(
            "GitLab",
            client.get(format!("{origin}/missing")),
        ))
        .expect_err("404 must be an error");
        match err {
            PlatformError::Http {
                status,
                platform,
                message,
                rate_limited,
            } => {
                assert_eq!(status, 404);
                assert_eq!(platform, "GitLab");
                assert_eq!(message, "not found");
                assert!(!rate_limited);
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn fetch_json_maps_429_to_rate_limited() {
        let origin = spawn_server(
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 4\r\nConnection: close\r\n\r\nslow"
                .to_string(),
        );
        let client = Client::new();
        let err = block_on(fetch_json(
            "GitLab",
            client.get(format!("{origin}/limited")),
        ))
        .unwrap_err();
        assert!(matches!(
            err,
            PlatformError::Http {
                status: 429,
                rate_limited: true,
                ..
            }
        ));
    }

    #[test]
    fn fetch_json_maps_invalid_body_to_other() {
        let origin = spawn_server(ok_response("this is not json"));
        let client = Client::new();
        let err =
            block_on(fetch_json("Gitea", client.get(format!("{origin}/api/v1")))).unwrap_err();
        match err {
            PlatformError::Other { platform, .. } => assert_eq!(platform, "Gitea"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn fetch_page_exposes_the_next_page_cursor() {
        let origin = spawn_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nx-next-page: 2\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]"
                .to_string(),
        );
        let client = Client::new();
        let page = block_on(fetch_page(
            "GitLab",
            client.get(format!("{origin}/api/v4/todos?page=1")),
        ))
        .expect("json page");
        assert!(page.body.as_array().is_some());
        assert_eq!(page.next_page.as_deref(), Some("2"));
    }

    #[test]
    fn fetch_page_treats_empty_or_missing_cursor_as_last_page() {
        let empty_cursor = spawn_server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nx-next-page: \r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]"
                .to_string(),
        );
        let page = block_on(fetch_page(
            "GitLab",
            Client::new().get(format!("{empty_cursor}/api/v4/todos")),
        ))
        .expect("json page");
        assert_eq!(page.next_page, None);

        let no_header = spawn_server(ok_response("[]"));
        let page = block_on(fetch_page(
            "GitLab",
            Client::new().get(format!("{no_header}/api/v4/todos")),
        ))
        .expect("json page");
        assert_eq!(page.next_page, None);
    }

    #[test]
    fn fetch_json_maps_connection_refused_to_network() {
        // Bind then drop, so the port is closed: connect must fail fast.
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let client = Client::new();
        let err = block_on(fetch_json(
            "GitLab",
            client.get(format!("http://127.0.0.1:{port}/api/v4")),
        ))
        .unwrap_err();
        assert!(
            matches!(err, PlatformError::Network(_)),
            "expected Network, got {err:?}"
        );
    }
}
