//! Whether Ollama is on this machine, and which models it has.
//!
//! Only when asked (the Models view's refresh, or listing providers for a launch),
//! never at startup. Two plain HTTP/1.0 requests to Ollama's documented API on the
//! loopback address, `GET /api/version` and `GET /api/tags`, with short timeouts.
//! Nothing is installed, started or downloaded: a model the user wants is pulled
//! by the user, with Ollama.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use x8ai_core::model::{LocalAvailability, is_model_id};

/// Where Ollama listens by default. The provider definition's endpoints point
/// here too; a server moved elsewhere with `OLLAMA_HOST` is not found.
pub const ADDRESS: ([u8; 4], u16) = ([127, 0, 0, 1], 11434);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const IO_TIMEOUT: Duration = Duration::from_secs(2);
/// Largest response read. A model list is a few kilobytes per model.
const MAX_RESPONSE: u64 = 4 << 20;
const MAX_MODELS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub availability: LocalAvailability,
    /// Models the server has, by the id Ollama uses (`qwen3-coder:30b`).
    pub models: Vec<String>,
}

/// Checks the installation, then the server at `address`. `path` is the user's
/// login `PATH`.
pub fn detect(path: Option<&OsStr>, address: SocketAddr) -> Detection {
    detect_under(path, Path::new("/"), address)
}

fn detect_under(path: Option<&OsStr>, root: &Path, address: SocketAddr) -> Detection {
    match probe(address) {
        Some((version, models)) => Detection {
            availability: LocalAvailability::Available { version },
            models,
        },
        None => Detection {
            availability: if installed(path, root) {
                LocalAvailability::Installed
            } else {
                LocalAvailability::Unavailable
            },
            models: Vec::new(),
        },
    }
}

/// Whether the `ollama` command is on `path`, or the app is in `Applications`
/// (under `root`, which tests replace).
pub fn installed(path: Option<&OsStr>, root: &Path) -> bool {
    let on_path = path.is_some_and(|path| {
        std::env::split_paths(path)
            .any(|dir| dir.is_absolute() && is_executable(&dir.join("ollama")))
    });
    let mut apps = vec![root.join("Applications/Ollama.app")];
    if let Some(home) = std::env::home_dir() {
        apps.push(
            root.join(home.strip_prefix("/").unwrap_or(&home))
                .join("Applications/Ollama.app"),
        );
    }
    on_path || apps.iter().any(|app: &PathBuf| app.is_dir())
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The server's version and models, if it answers as Ollama does.
fn probe(address: SocketAddr) -> Option<(Option<String>, Vec<String>)> {
    #[derive(Deserialize)]
    struct Version {
        version: Option<String>,
    }
    #[derive(Deserialize)]
    struct Tags {
        #[serde(default)]
        models: Vec<Tag>,
    }
    #[derive(Deserialize)]
    struct Tag {
        name: String,
    }

    let version: Version = serde_json::from_slice(&get(address, "/api/version")?).ok()?;
    let version = version
        .version
        .filter(|v| v.len() <= 64 && v.chars().all(|c| c.is_ascii_graphic()));
    // Answering but not listing models is still a server; the list is then empty.
    let mut models: Vec<String> = get(address, "/api/tags")
        .and_then(|body| serde_json::from_slice::<Tags>(&body).ok())
        .map(|tags| {
            tags.models
                .into_iter()
                .map(|t| t.name)
                .filter(|n| is_model_id(n))
                .collect()
        })
        .unwrap_or_default();
    models.sort();
    models.dedup();
    models.truncate(MAX_MODELS);
    Some((version, models))
}

/// The body of a `200 OK` response to `GET path`, or `None`.
fn get(address: SocketAddr, path: &str) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;
    let request =
        format!("GET {path} HTTP/1.0\r\nHost: {address}\r\nAccept: application/json\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    stream.take(MAX_RESPONSE).read_to_end(&mut response).ok()?;
    let split = response.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&response[..split]).ok()?;
    let body = &response[split + 4..];
    let mut lines = head.split("\r\n");
    let status = lines.next()?;
    if !(status.starts_with("HTTP/1.") && status.split(' ').nth(1) == Some("200")) {
        return None;
    }
    let chunked = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value.trim().eq_ignore_ascii_case("chunked")
        })
    });
    if chunked {
        dechunk(body)
    } else {
        Some(body.to_vec())
    }
}

fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = body.windows(2).position(|w| w == b"\r\n")?;
        let size = std::str::from_utf8(&body[..end]).ok()?;
        let size = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
        body = &body[end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(body.get(..size)?);
        body = body.get(size + 2..)?;
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    /// A server that answers each connection with the next canned response.
    fn serve(responses: Vec<&'static str>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 1024];
                let n = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..n]).to_string();
                assert!(request.starts_with("GET /api/"), "{request}");
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        address
    }

    fn closed_port() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    }

    #[test]
    fn a_running_server_is_available_with_its_models() {
        let address = serve(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"version\":\"0.34.4\"}",
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n\
             {\"models\":[{\"name\":\"qwen3-coder:30b\",\"size\":1},{\"name\":\"llama3.2:latest\"},{\"name\":\"-bad id\"}]}",
        ]);
        let detection = detect(None, address);
        assert_eq!(
            detection.availability,
            LocalAvailability::Available {
                version: Some("0.34.4".into())
            }
        );
        assert_eq!(detection.models, ["llama3.2:latest", "qwen3-coder:30b"]);
    }

    #[test]
    fn chunked_responses_are_read() {
        let address = serve(vec![
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n8\r\n{\"versio\r\n\
             b\r\nn\":\"0.1.0\"}\r\n0\r\n\r\n",
            "HTTP/1.1 500 Internal Server Error\r\n\r\n",
        ]);
        let detection = detect(None, address);
        assert_eq!(
            detection.availability,
            LocalAvailability::Available {
                version: Some("0.1.0".into())
            }
        );
        assert!(detection.models.is_empty());
    }

    #[test]
    fn something_else_on_the_port_is_not_ollama() {
        let address = serve(vec!["HTTP/1.1 404 Not Found\r\n\r\nnope"]);
        assert_eq!(
            detect(None, address).availability,
            LocalAvailability::Unavailable
        );
        let address = serve(vec!["HTTP/1.1 200 OK\r\n\r\n<html>hello</html>"]);
        assert_eq!(
            detect(None, address).availability,
            LocalAvailability::Unavailable
        );
    }

    #[test]
    fn installed_but_not_running_is_installed() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let path = std::env::join_paths([&bin]).unwrap();
        let detect = |path| detect_under(Some(path), temp.path(), closed_port()).availability;
        assert_eq!(detect(&path), LocalAvailability::Unavailable);

        let ollama = bin.join("ollama");
        std::fs::write(&ollama, "#!/bin/sh\n").unwrap();
        assert!(!installed(Some(&path), temp.path()), "not executable");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ollama, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(detect(&path), LocalAvailability::Installed);
    }

    #[test]
    fn the_app_bundle_counts_as_installed() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!installed(None, temp.path()));
        std::fs::create_dir_all(temp.path().join("Applications/Ollama.app")).unwrap();
        assert!(installed(None, temp.path()));
    }

    #[test]
    fn relative_path_entries_are_ignored() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!installed(Some(OsStr::new("relative/bin:.")), temp.path()));
    }
}
