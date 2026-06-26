//! Small loopback HTTP client for an attached `cortex serve` process.
//!
//! The TUI stays a terminal app and does not depend on a generated SDK. When the user opts into an
//! attached local server (`cortex tui --server http://127.0.0.1:32NN` or `CORTEX_SERVER_URL`), this
//! module posts typed turn events to the server's `/event` bridge and asks the server to create
//! loopback shares. Only 127.0.0.1/localhost ports in 3200-3299 are accepted.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use events::Event;

const MIN_PORT: u16 = 3200;
const MAX_PORT: u16 = 3299;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteServer {
    origin: String,
    authority: String,
    addr: SocketAddr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteShare {
    pub share_id: String,
    pub url: String,
}

impl RemoteServer {
    pub fn from_url(url: &str) -> Result<Self, String> {
        let trimmed = url.trim().trim_end_matches('/');
        let rest = trimmed
            .strip_prefix("http://")
            .ok_or_else(|| "server URL must use http:// loopback".to_owned())?;
        if rest.is_empty() || rest.contains('/') {
            return Err("server URL must be an origin like http://127.0.0.1:3200".to_owned());
        }
        let (host, port) = rest.rsplit_once(':').ok_or_else(|| {
            "server URL must include a port in the reserved 3200-3299 range".to_owned()
        })?;
        if !(host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost")) {
            return Err("server URL host must be loopback (127.0.0.1 or localhost)".to_owned());
        }
        let port = port
            .parse::<u16>()
            .map_err(|_| "server URL port must be numeric".to_owned())?;
        if !(MIN_PORT..=MAX_PORT).contains(&port) {
            return Err(format!(
                "server URL port {port} is outside the reserved 3200-3299 range"
            ));
        }
        let authority = format!("127.0.0.1:{port}");
        Ok(Self {
            origin: format!("http://{authority}"),
            authority,
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn publish_event(&self, event: &Event) -> Result<(), String> {
        let body = serde_json::to_string(event).map_err(|err| err.to_string())?;
        let response = self.post_json("/event", &body)?;
        if (200..300).contains(&response.status) {
            Ok(())
        } else {
            Err(format!(
                "server event publish failed with HTTP {}: {}",
                response.status, response.body
            ))
        }
    }

    pub fn share_session(&self, session_id: &str) -> Result<RemoteShare, String> {
        let path = format!("/session/{session_id}/share");
        let response = self.post_json(&path, "")?;
        if !(200..300).contains(&response.status) {
            return Err(format!(
                "server share failed with HTTP {}: {}",
                response.status, response.body
            ));
        }
        let value: serde_json::Value =
            serde_json::from_str(&response.body).map_err(|err| err.to_string())?;
        let share_id = value
            .get("share_id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "server share response did not include share_id".to_owned())?
            .to_owned();
        let url = value
            .get("url")
            .and_then(serde_json::Value::as_str)
            .filter(|url| {
                url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")
            })
            .ok_or_else(|| "server share response did not include a loopback URL".to_owned())?
            .to_owned();
        Ok(RemoteShare { share_id, url })
    }

    pub fn unshare_session(&self, session_id: &str) -> Result<bool, String> {
        let path = format!("/session/{session_id}/unshare");
        let response = self.post_json(&path, "")?;
        if !(200..300).contains(&response.status) {
            return Err(format!(
                "server unshare failed with HTTP {}: {}",
                response.status, response.body
            ));
        }
        let value: serde_json::Value =
            serde_json::from_str(&response.body).map_err(|err| err.to_string())?;
        Ok(value
            .get("revoked")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false))
    }

    fn post_json(&self, path: &str, body: &str) -> Result<HttpResponse, String> {
        let mut stream =
            TcpStream::connect_timeout(&self.addr, Duration::from_secs(2)).map_err(|err| {
                format!(
                    "could not connect to attached server {}: {err}",
                    self.origin
                )
            })?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        let request = format!(
            "POST {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             Content-Type: application/json\r\n\
             Accept: application/json\r\n\
             Content-Length: {len}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            host = self.authority,
            len = body.len(),
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|err| format!("failed to write request to attached server: {err}"))?;
        let mut raw = String::new();
        stream
            .read_to_string(&mut raw)
            .map_err(|err| format!("failed to read attached server response: {err}"))?;
        parse_response(&raw)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct HttpResponse {
    status: u16,
    body: String,
}

fn parse_response(raw: &str) -> Result<HttpResponse, String> {
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| "attached server returned a malformed HTTP response".to_owned())?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| "attached server response had no HTTP status".to_owned())?;
    Ok(HttpResponse {
        status,
        body: body.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_loopback_ports_in_the_server_range() {
        assert!(RemoteServer::from_url("http://127.0.0.1:3200").is_ok());
        assert!(RemoteServer::from_url("http://localhost:3299").is_ok());
        assert!(RemoteServer::from_url("https://127.0.0.1:3200").is_err());
        assert!(RemoteServer::from_url("http://example.com:3200").is_err());
        assert!(RemoteServer::from_url("http://127.0.0.1:3199").is_err());
        assert!(RemoteServer::from_url("http://127.0.0.1:3300").is_err());
    }

    #[test]
    fn parses_basic_http_response() {
        let parsed = parse_response("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap();
        assert_eq!(
            parsed,
            HttpResponse {
                status: 200,
                body: "{}".to_owned()
            }
        );
    }
}
