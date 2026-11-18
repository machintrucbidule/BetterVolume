//! A small HTTP client for the update check and its downloads (FR-132), through WinHTTP,
//! the HTTP stack Windows itself uses: Windows' certificate store and the system proxy
//! settings, no dependency. Blocking: it runs on a thread of its own, never on the UI
//! thread.
//!
//! Only `GET`. Redirections are followed (release files are served from another host),
//! never from an encrypted address to a plain one. No cookie is kept or sent, and nothing
//! is sent beyond the address and the headers the caller gives.

use windows::core::HSTRING;
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
    WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_COOKIES,
    WINHTTP_FLAG_SECURE, WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_OPTION_DISABLE_FEATURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

/// How long name resolution, connection, sending and each wait for data may take, in
/// milliseconds.
const TIMEOUT_MS: i32 = 30_000;

/// Why a request failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// The address is not an `http://` or `https://` address this client can use.
    Address,
    /// No answer: no network, the server could not be reached, the connection broke, or
    /// its certificate was refused.
    Network,
    /// The server answered with this status instead of 200.
    Status(u16),
    /// The answer is longer than the caller accepts.
    TooLarge,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Address => f.write_str("unusable address"),
            HttpError::Network => f.write_str("no answer"),
            HttpError::Status(code) => write!(f, "status {code}"),
            HttpError::TooLarge => f.write_str("answer too long"),
        }
    }
}

/// The parts of an address.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Address {
    secure: bool,
    host: String,
    port: u16,
    /// Path and query, starting with `/`.
    path: String,
}

/// Splits `http[s]://host[:port][/path][?query]` (no user name, no fragment).
fn address(url: &str) -> Option<Address> {
    let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else {
        (false, url.strip_prefix("http://")?)
    };
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    if authority.is_empty() || authority.contains('@') || path.contains('#') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok().filter(|p| *p > 0)?),
        _ => (authority, if secure { 443 } else { 80 }),
    };
    if host.is_empty() || host.contains(['[', ']']) || !host.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let path = match path {
        "" => "/".to_owned(),
        p if p.starts_with('?') => format!("/{p}"),
        p => p.to_owned(),
    };
    if !path.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    Some(Address { secure, host: host.to_owned(), port, path })
}

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut core::ffi::c_void);

impl Handle {
    fn new(raw: *mut core::ffi::c_void) -> Result<Handle, HttpError> {
        if raw.is_null() {
            Err(HttpError::Network)
        } else {
            Ok(Handle(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle WinHTTP returned, closed once (children before parents, as the
        // handles are declared in that order and dropped in reverse).
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// Fetches `url` with `agent` as the user agent and the extra `headers`, reading at most
/// `limit` bytes. `progress` is told the bytes received so far and the length the server
/// announced, if any.
pub fn get(
    url: &str,
    agent: &str,
    headers: &[(&str, &str)],
    limit: usize,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, HttpError> {
    let target = address(url).ok_or(HttpError::Address)?;
    if headers.iter().any(|(n, v)| n.contains([':', '\r', '\n']) || v.contains(['\r', '\n'])) {
        return Err(HttpError::Address);
    }
    // SAFETY: valid NUL-terminated strings for the duration of the call; no proxy names
    // with the automatic proxy.
    let session = Handle::new(unsafe {
        WinHttpOpen(
            &HSTRING::from(agent),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            0,
        )
    })?;
    // SAFETY: a live session handle.
    unsafe { WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS) }
        .map_err(|_| HttpError::Network)?;
    // SAFETY: a live session handle and a valid host string.
    let connection = Handle::new(unsafe {
        WinHttpConnect(session.0, &HSTRING::from(&target.host), target.port, 0)
    })?;
    let flags = if target.secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
    // SAFETY: a live connection handle, valid strings, no accept-type list (null).
    let request = Handle::new(unsafe {
        WinHttpOpenRequest(
            connection.0,
            &HSTRING::from("GET"),
            &HSTRING::from(&target.path),
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            std::ptr::null(),
            flags,
        )
    })?;
    let no_cookies = WINHTTP_DISABLE_COOKIES.to_ne_bytes();
    // SAFETY: a live request handle; the option takes a 32-bit flag word.
    let _ = unsafe {
        WinHttpSetOption(Some(request.0), WINHTTP_OPTION_DISABLE_FEATURE, Some(&no_cookies))
    };
    let lines: String =
        headers.iter().map(|(name, value)| format!("{name}: {value}\r\n")).collect();
    let wide: Vec<u16> = lines.encode_utf16().collect();
    // SAFETY: a live request handle; the header buffer outlives the call; no body.
    unsafe {
        WinHttpSendRequest(request.0, (!wide.is_empty()).then_some(wide.as_slice()), None, 0, 0, 0)
    }
    .map_err(|_| HttpError::Network)?;
    // SAFETY: a live request handle whose request was sent.
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }
        .map_err(|_| HttpError::Network)?;
    let status = number_header(&request, WINHTTP_QUERY_STATUS_CODE).ok_or(HttpError::Network)?;
    if status != 200 {
        return Err(HttpError::Status(u16::try_from(status).unwrap_or(0)));
    }
    let announced =
        number_header(&request, windows::Win32::Networking::WinHttp::WINHTTP_QUERY_CONTENT_LENGTH)
            .map(u64::from);
    if announced.is_some_and(|n| n > limit as u64) {
        return Err(HttpError::TooLarge);
    }
    let mut body = Vec::with_capacity(announced.map_or(0, |n| n as usize));
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        // SAFETY: a live request handle; the buffer is writable for its whole length.
        unsafe {
            WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read)
        }
        .map_err(|_| HttpError::Network)?;
        if read == 0 {
            break;
        }
        let read = read as usize;
        if body.len() + read > limit {
            return Err(HttpError::TooLarge);
        }
        body.extend_from_slice(&chunk[..read]);
        progress(body.len() as u64, announced);
    }
    if announced.is_some_and(|n| n != body.len() as u64) {
        return Err(HttpError::Network);
    }
    Ok(body)
}

/// A numeric header of the answer (status code, length).
fn number_header(request: &Handle, which: u32) -> Option<u32> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: a live request handle with a received answer; the buffer holds one `u32`.
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            which | WINHTTP_QUERY_FLAG_NUMBER,
            windows::core::PCWSTR::null(),
            Some((&mut value as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
    }
    .ok()?;
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn addresses_are_split() {
        let a = address("https://api.github.com/repos/o/B/releases?per_page=30").unwrap();
        assert_eq!(
            a,
            Address {
                secure: true,
                host: "api.github.com".into(),
                port: 443,
                path: "/repos/o/B/releases?per_page=30".into()
            }
        );
        let b = address("http://127.0.0.1:8123").unwrap();
        assert_eq!(
            (b.secure, b.host.as_str(), b.port, b.path.as_str()),
            (false, "127.0.0.1", 8123, "/")
        );
        assert_eq!(address("http://h?x=1").unwrap().path, "/?x=1");
        for bad in [
            "ftp://h/x",
            "https://",
            "https://user@h/",
            "https://h:0/",
            "https://h:99999/",
            "https://h/a b",
            "https://h/#x",
            "h/x",
        ] {
            assert_eq!(address(bad), None, "{bad}");
        }
    }

    /// Serves one answer on the loopback address and returns the request it read.
    fn serve(answer: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buffer).expect("read");
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
            }
            stream.write_all(&answer).expect("write");
            String::from_utf8_lossy(&request).into_owned()
        });
        (format!("http://127.0.0.1:{port}"), thread)
    }

    fn ok(body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn fetches_an_answer_with_its_headers_and_progress() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let (base, server) = serve(ok(&body));
        let mut last = (0, None);
        let fetched = get(
            &format!("{base}/repos/o/B/releases?per_page=30"),
            "BetterVolume/1.2.3",
            &[("Accept", "application/vnd.github+json")],
            1 << 20,
            &mut |received, total| last = (received, total),
        )
        .expect("answer");
        assert_eq!(fetched, body);
        assert_eq!(last, (body.len() as u64, Some(body.len() as u64)));
        let request = server.join().unwrap();
        assert!(
            request.starts_with("GET /repos/o/B/releases?per_page=30 HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(request.contains("User-Agent: BetterVolume/1.2.3\r\n"), "{request}");
        assert!(request.contains("Accept: application/vnd.github+json\r\n"), "{request}");
        assert!(!request.to_ascii_lowercase().contains("cookie"), "{request}");
    }

    #[test]
    fn other_statuses_and_long_answers_fail() {
        let (base, server) = serve(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        );
        assert_eq!(get(&base, "a", &[], 100, &mut |_, _| {}), Err(HttpError::Status(404)));
        server.join().unwrap();
        let (base, server) = serve(ok(&[7u8; 500]));
        assert_eq!(get(&base, "a", &[], 100, &mut |_, _| {}), Err(HttpError::TooLarge));
        server.join().unwrap();
        // An answer without a length, longer than accepted, is cut short too.
        let mut chunked = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        chunked.extend_from_slice(&[1u8; 500]);
        let (base, server) = serve(chunked);
        assert_eq!(get(&base, "a", &[], 100, &mut |_, _| {}), Err(HttpError::TooLarge));
        server.join().unwrap();
    }

    #[test]
    fn a_closed_port_or_a_bad_address_fails_cleanly() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(
            get(&format!("http://127.0.0.1:{port}/"), "a", &[], 100, &mut |_, _| {}),
            Err(HttpError::Network)
        );
        assert_eq!(get("file:///c:/x", "a", &[], 100, &mut |_, _| {}), Err(HttpError::Address));
        assert_eq!(
            get("http://127.0.0.1/", "a", &[("X", "a\r\nInjected: 1")], 100, &mut |_, _| {}),
            Err(HttpError::Address)
        );
    }
}
