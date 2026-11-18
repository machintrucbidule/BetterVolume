//! A local imitation of the release service, for tests and manual checks: it
//! answers like GitHub's release list, on the loopback address only, from the release
//! files found in a folder. Never shipped.
//!
//! ```text
//! release-server --port <port> --folder <folder> [--preview]
//! ```
//!
//! Every pair `BetterVolume-<version>-setup.exe` / `-portable.exe` in the folder is one
//! release (tag `v<version>`, a preview when the version has a suffix, or with
//! `--preview`). Answers:
//!
//! - `GET /repos/<owner>/<name>/releases...`: the JSON list, newest first, with the size
//!   and the SHA-256 digest of each file, as GitHub gives them;
//! - `GET /download/<file>`: the file;
//! - `GET /<owner>/<name>/releases/tag/<tag>`: a small page standing for the release page.
//!
//! The first line written is `listening on <port>`; each request then gets a line.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

use bv_core::package::Version;

struct Release {
    version: Version,
    files: Vec<(String, u64, String)>,
}

fn releases(folder: &Path) -> Vec<Release> {
    let mut found: Vec<Release> = Vec::new();
    let Ok(entries) = std::fs::read_dir(folder) else { return found };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix("BetterVolume-") else { continue };
        let Some(version) =
            rest.strip_suffix("-setup.exe").or_else(|| rest.strip_suffix("-portable.exe"))
        else {
            continue;
        };
        let Some(version) = Version::parse(version) else { continue };
        let Ok(bytes) = std::fs::read(entry.path()) else { continue };
        let digest = bv_core::encoding::hex(&bv_core::encoding::sha256(&bytes));
        let file = (name, bytes.len() as u64, digest);
        match found.iter_mut().find(|r| r.version == version) {
            Some(release) => release.files.push(file),
            None => found.push(Release { version, files: vec![file] }),
        }
    }
    found.sort_by(|a, b| b.version.cmp(&a.version));
    found
}

fn quote(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn list(folder: &Path, base: &str, repository: &str, preview: bool) -> String {
    let items: Vec<String> = releases(folder)
        .iter()
        .map(|r| {
            let tag = format!("v{}", r.version);
            let assets: Vec<String> = r
                .files
                .iter()
                .map(|(name, size, digest)| {
                    format!(
                        r#"{{"name":"{}","size":{size},"digest":"sha256:{digest}","browser_download_url":"{base}/download/{}"}}"#,
                        quote(name),
                        quote(name)
                    )
                })
                .collect();
            format!(
                r#"{{"tag_name":"{tag}","name":"BetterVolume {}","draft":false,"prerelease":{},"html_url":"{base}/{repository}/releases/tag/{tag}","body":"Local imitation","assets":[{}]}}"#,
                r.version,
                preview || r.version.is_pre_release(),
                assets.join(",")
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

fn answer(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body));
}

fn serve(mut stream: TcpStream, folder: &Path, base: &str, preview: bool, out: &mut impl Write) {
    let mut line = String::new();
    {
        let mut reader = BufReader::new(&stream);
        if reader.read_line(&mut line).is_err() {
            return;
        }
        // The headers are not needed: read them to the empty line.
        let mut header = String::new();
        while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
            header.clear();
        }
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let _ = writeln!(out, "GET {path}");
    let _ = out.flush();
    let parts: Vec<&str> = path.split('?').next().unwrap_or("").split('/').collect();
    match parts.as_slice() {
        ["", "repos", owner, name, "releases"] => {
            let body = list(folder, base, &format!("{owner}/{name}"), preview);
            answer(&mut stream, "200 OK", "application/json; charset=utf-8", body.as_bytes());
        }
        ["", "download", file] if !file.contains("..") && !file.contains('\\') => {
            match std::fs::read(folder.join(file)) {
                Ok(bytes) => answer(&mut stream, "200 OK", "application/octet-stream", &bytes),
                Err(_) => answer(&mut stream, "404 Not Found", "text/plain", b"not found"),
            }
        }
        ["", _, _, "releases", "tag", tag] => {
            let page = format!(
                "<!doctype html><meta charset=utf-8><title>{tag}</title><h1>BetterVolume {tag}</h1><p>Local imitation of the release page.</p>"
            );
            answer(&mut stream, "200 OK", "text/html; charset=utf-8", page.as_bytes());
        }
        _ => answer(&mut stream, "404 Not Found", "text/plain", b"not found"),
    }
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let port: u16 = value("--port").and_then(|p| p.parse().ok()).unwrap_or(0);
    let folder = PathBuf::from(value("--folder").map_or(".", String::as_str));
    let preview = args.iter().any(|a| a == "--preview");
    let mut out = std::io::stdout();
    let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) else {
        let _ = writeln!(out, "cannot listen on port {port}");
        return std::process::ExitCode::FAILURE;
    };
    let port = listener.local_addr().map_or(port, |a| a.port());
    let base = format!("http://127.0.0.1:{port}");
    let _ = writeln!(out, "listening on {port}");
    let _ = out.flush();
    for stream in listener.incoming().flatten() {
        serve(stream, &folder, &base, preview, &mut out);
    }
    std::process::ExitCode::SUCCESS
}
