use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy)]
struct HttpTimeouts {
    connect: Duration,
    read: Duration,
    total: Duration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct SidecarHealth {
    pub engine: String,
    pub status: String,
}

#[derive(Clone)]
pub(crate) struct SidecarHttpClient {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl SidecarHttpClient {
    pub(crate) fn new(base_url: &str, token: &str) -> Result<Self, String> {
        Self::with_timeouts(
            base_url,
            token,
            HttpTimeouts {
                connect: CONNECT_TIMEOUT,
                read: READ_TIMEOUT,
                total: TOTAL_TIMEOUT,
            },
        )
    }

    fn with_timeouts(base_url: &str, token: &str, timeouts: HttpTimeouts) -> Result<Self, String> {
        let parsed =
            reqwest::Url::parse(base_url).map_err(|_| "SIDECAR_HTTP_URL_INVALID".to_string())?;
        if parsed.scheme() != "http"
            || parsed.host_str() != Some("127.0.0.1")
            || parsed.port().is_none()
            || parsed.path() != "/"
        {
            return Err("SIDECAR_HTTP_URL_INVALID".to_string());
        }
        if token.is_empty() {
            return Err("SIDECAR_HTTP_TOKEN_REQUIRED".to_string());
        }
        let client = reqwest::Client::builder()
            .http1_only()
            .connect_timeout(timeouts.connect)
            .read_timeout(timeouts.read)
            .timeout(timeouts.total)
            .build()
            .map_err(|error| format!("SIDECAR_HTTP_CLIENT:{error}"))?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        })
    }

    pub(crate) async fn health(&self) -> Result<SidecarHealth, String> {
        self.request_json("/health", false).await
    }

    pub(crate) async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.request_json(path, true).await
    }

    pub(crate) async fn post_json<I: Serialize, O: DeserializeOwned>(
        &self,
        path: &str,
        body: &I,
    ) -> Result<O, String> {
        if !path.starts_with('/') || path.starts_with("//") {
            return Err("SIDECAR_HTTP_PATH_INVALID".to_string());
        }
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    "SIDECAR_HTTP_TIMEOUT".to_string()
                } else {
                    format!("SIDECAR_HTTP_REQUEST:{error}")
                }
            })?;
        if !response.status().is_success() {
            return Err(format!(
                "SIDECAR_HTTP_STATUS_{}",
                response.status().as_u16()
            ));
        }
        response
            .json::<O>()
            .await
            .map_err(|error| format!("SIDECAR_HTTP_JSON:{error}"))
    }

    async fn request_json<T: DeserializeOwned>(
        &self,
        path: &str,
        authenticated: bool,
    ) -> Result<T, String> {
        if !path.starts_with('/') || path.starts_with("//") {
            return Err("SIDECAR_HTTP_PATH_INVALID".to_string());
        }
        let url = format!("{}{}", self.base_url, path);
        let mut request = self.client.get(url);
        if authenticated {
            request = request.bearer_auth(&self.token);
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                "SIDECAR_HTTP_TIMEOUT".to_string()
            } else {
                format!("SIDECAR_HTTP_REQUEST:{error}")
            }
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "SIDECAR_HTTP_STATUS_{}",
                response.status().as_u16()
            ));
        }
        response
            .json::<T>()
            .await
            .map_err(|error| format!("SIDECAR_HTTP_JSON:{error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{HttpTimeouts, SidecarHealth, SidecarHttpClient};
    use std::time::Duration;
    use tauri::async_runtime::block_on;
    use tiny_http::{Response, Server};

    #[test]
    fn client_requires_a_loopback_http_origin() {
        assert!(SidecarHttpClient::new("https://example.invalid", "token").is_err());
        assert!(SidecarHttpClient::new("http://127.0.0.1:12345", "token").is_ok());
    }

    #[test]
    fn health_is_unauthenticated_but_json_requests_send_bearer() {
        let server = Server::http("127.0.0.1:0").expect("test server must bind");
        let address = server
            .server_addr()
            .to_ip()
            .expect("server must have an IP");
        let origin = format!("http://127.0.0.1:{}", address.port());
        let thread = std::thread::spawn(move || {
            let health = server.recv().expect("health request must arrive");
            assert!(health
                .headers()
                .iter()
                .all(|header| !header.field.equiv("Authorization")));
            health
                .respond(Response::from_string(
                    r#"{"engine":"podcast","status":"ok"}"#,
                ))
                .expect("health response must write");
            let protected = server.recv().expect("protected request must arrive");
            let authorization = protected
                .headers()
                .iter()
                .find(|header| header.field.equiv("Authorization"))
                .expect("protected request must have authorization");
            assert_eq!(authorization.value.as_str(), "Bearer secret-token");
            protected
                .respond(Response::from_string(r#"{"status":"ready"}"#))
                .expect("protected response must write");
        });
        let client = SidecarHttpClient::new(&origin, "secret-token").expect("client must build");
        let health = block_on(client.health()).expect("health must decode");
        assert_eq!(
            health,
            SidecarHealth {
                engine: "podcast".to_string(),
                status: "ok".to_string(),
            }
        );
        let status: serde_json::Value =
            block_on(client.get_json("/status")).expect("status must decode");
        assert_eq!(status["status"], "ready");
        thread.join().expect("server thread must finish");
    }

    #[test]
    fn read_and_total_timeouts_fail_closed() {
        let server = Server::http("127.0.0.1:0").expect("test server must bind");
        let address = server
            .server_addr()
            .to_ip()
            .expect("server must have an IP");
        let origin = format!("http://127.0.0.1:{}", address.port());
        let thread = std::thread::spawn(move || {
            let request = server.recv().expect("request must arrive");
            std::thread::sleep(Duration::from_millis(100));
            let _ = request.respond(Response::from_string("{}"));
        });
        let client = SidecarHttpClient::with_timeouts(
            &origin,
            "secret-token",
            HttpTimeouts {
                connect: Duration::from_millis(50),
                read: Duration::from_millis(20),
                total: Duration::from_millis(40),
            },
        )
        .expect("client must build");
        let error = block_on(client.health()).expect_err("slow response must time out");
        assert_eq!(error, "SIDECAR_HTTP_TIMEOUT");
        thread.join().expect("server thread must finish");
    }
}
