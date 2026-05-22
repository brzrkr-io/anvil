//! Caldera API client: AI agent transport layer.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub use anvil_agent::{
    CurrentSessionResponse, HealthResponse, SessionStartRequest, SessionStartResponse,
};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct CalderaClient {
    base_url: String,
    timeout: Duration,
}

#[derive(Debug, Error)]
pub enum CalderaError {
    #[error("unsupported Caldera URL: {0}")]
    UnsupportedUrl(String),
    #[error("Caldera URL is missing host")]
    MissingHost,
    #[error("invalid Caldera port: {0}")]
    InvalidPort(String),
    #[error("Caldera I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Caldera returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("Caldera response was not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Caldera response was malformed")]
    MalformedResponse,
}

impl CalderaClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            timeout: Duration::from_secs(5),
        }
    }

    pub fn localhost() -> Self {
        Self::new("http://127.0.0.1:4175")
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn health(&self) -> Result<HealthResponse, CalderaError> {
        self.get_json("/health")
    }

    pub fn current_session(&self) -> Result<CurrentSessionResponse, CalderaError> {
        self.get_json("/api/sessions/current")
    }

    pub fn start_session(
        &self,
        request: &SessionStartRequest,
    ) -> Result<SessionStartResponse, CalderaError> {
        let body = serde_json::to_string(request)?;
        self.post_json("/api/sessions/start", &body)
    }

    fn get_json<T>(&self, path: &str) -> Result<T, CalderaError>
    where
        T: serde::de::DeserializeOwned,
    {
        let body = self.request("GET", path, "")?;
        Ok(serde_json::from_str(&body)?)
    }

    fn post_json<T>(&self, path: &str, body: &str) -> Result<T, CalderaError>
    where
        T: serde::de::DeserializeOwned,
    {
        let body = self.request("POST", path, body)?;
        Ok(serde_json::from_str(&body)?)
    }

    fn request(&self, method: &str, path: &str, body: &str) -> Result<String, CalderaError> {
        let endpoint = Endpoint::parse(&self.base_url)?;
        let request_path = endpoint.path(path);
        let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port))?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        let request = format!(
            "{method} {request_path} HTTP/1.1\r\nHost: {}:{}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            endpoint.host,
            endpoint.port,
            body.len(),
            body
        );
        stream.write_all(request.as_bytes())?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        parse_response(&response)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Endpoint {
    host: String,
    port: u16,
    prefix: String,
}

impl Endpoint {
    fn parse(base_url: &str) -> Result<Self, CalderaError> {
        let Some(rest) = base_url.strip_prefix("http://") else {
            return Err(CalderaError::UnsupportedUrl(base_url.to_string()));
        };
        let (authority, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() {
            return Err(CalderaError::MissingHost);
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let port = port
                    .parse::<u16>()
                    .map_err(|_| CalderaError::InvalidPort(port.to_string()))?;
                (host.to_string(), port)
            }
            None => (authority.to_string(), 80),
        };
        if host.is_empty() {
            return Err(CalderaError::MissingHost);
        }
        Ok(Self {
            host,
            port,
            prefix: normalize_prefix(prefix),
        })
    }

    fn path(&self, path: &str) -> String {
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        if self.prefix.is_empty() {
            path
        } else {
            format!("{}{}", self.prefix, path)
        }
    }
}

fn normalize_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}

fn parse_response(response: &str) -> Result<String, CalderaError> {
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or(CalderaError::MalformedResponse)?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(CalderaError::MalformedResponse)?;
    if !(200..300).contains(&status) {
        return Err(CalderaError::Http {
            status,
            body: body.to_string(),
        });
    }
    Ok(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn parses_endpoint_with_optional_prefix() {
        let endpoint = Endpoint::parse("http://127.0.0.1:4175/caldera").unwrap();

        assert_eq!(
            endpoint,
            Endpoint {
                host: "127.0.0.1".to_string(),
                port: 4175,
                prefix: "/caldera".to_string()
            }
        );
        assert_eq!(
            endpoint.path("/api/sessions/start"),
            "/caldera/api/sessions/start"
        );
    }

    #[test]
    fn start_session_posts_to_caldera() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("POST /api/sessions/start HTTP/1.1"));
            assert!(request.contains("\"task\":\"Review repo\""));

            let body = r##"{
              "schema_version": "caldera.session_start.v0",
              "session_id": "agent_1",
              "task": "Review repo",
              "agent": "lmstudio",
              "status": "prepared",
              "handoff_path": ".forge/agent-run-handoffs/agent_1.md",
              "handoff_markdown": "# Caldera AI Session",
              "context_cache": {
                "status": "ready",
                "source_state": "current",
                "refresh_reason": "test",
                "estimated_tokens_saved": 42
              },
              "launch": {
                "mode": "prepared",
                "command": ["caldera-local", "ask"],
                "shell_command": "caldera-local ask",
                "working_directory": "/repo",
                "handoff_file": "/repo/.forge/agent-run-handoffs/agent_1.md",
                "environment": {},
                "instructions": "Run it"
              },
              "run": {
                "schema_version": "caldera.agent_run.v0",
                "run_id": "agent_1",
                "task": "Review repo",
                "agent": "lmstudio",
                "status": "prepared",
                "workspace_root": "/repo",
                "created_at": "2026-05-22T00:00:00Z",
                "finished_at": ""
              },
              "next": []
            }"##;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let client = CalderaClient::new(format!("http://{}", addr));
        let response = client
            .start_session(&SessionStartRequest::new("Review repo", "lmstudio"))
            .unwrap();

        assert!(response.is_prepared());
        assert_eq!(response.context_cache.estimated_tokens_saved, 42);
        server.join().unwrap();
    }
}
