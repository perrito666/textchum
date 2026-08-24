//! `file://` URI conversion, since LSP addresses documents by URI.

use std::path::{Path, PathBuf};

/// Bytes that survive un-escaped in a URI path (RFC 3986 unreserved plus
/// the path separator).
fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

/// `/some path/x.rs` → `file:///some%20path/x.rs`
pub fn path_to_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.to_string_lossy().as_bytes() {
        if is_unreserved(byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// `file:///some%20path/x.rs` → `/some path/x.rs`; None for non-file URIs.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let mut chars = rest.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let high = chars.next()?;
            let low = chars.next()?;
            let hex = [high, low];
            let hex = std::str::from_utf8(&hex).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_plain_and_spicy_paths() {
        for path in ["/a/b/c.rs", "/with space/ünïcode/f.py", "/q?/#frag.c"] {
            let uri = path_to_uri(Path::new(path));
            assert!(uri.starts_with("file:///"));
            assert!(!uri.contains(' '), "spaces must be escaped: {uri}");
            assert_eq!(uri_to_path(&uri), Some(PathBuf::from(path)));
        }
    }

    #[test]
    fn rejects_non_file_uris() {
        assert_eq!(uri_to_path("https://example.com/x"), None);
    }
}
