//! JSON-RPC message framing, as the Language Server Protocol specifies:
//! `Content-Length: N\r\n\r\n` headers followed by N bytes of JSON.

use std::io::{BufRead, Write};

use serde_json::Value;

/// Writes one framed message.
pub fn write_message(writer: &mut impl Write, message: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

/// Reads one framed message. `Ok(None)` means the stream ended cleanly
/// between messages (the server exited).
pub fn read_message(reader: &mut impl BufRead) -> std::io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        // Headers are case-insensitive; Content-Type is the only other
        // header the spec defines and it carries no information we need.
        if let Some(value) = line
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
            .map(str::trim)
        {
            content_length = Some(value.parse().map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("bad Content-Length: {e}"),
                )
            })?);
        }
    }
    let Some(length) = content_length else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "message without Content-Length",
        ));
    };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_messages() {
        let message = json!({"jsonrpc": "2.0", "method": "x", "params": {"núm": 1}});
        let mut buffer = Vec::new();
        write_message(&mut buffer, &message).unwrap();
        write_message(&mut buffer, &json!({"id": 2})).unwrap();

        let mut reader = std::io::BufReader::new(buffer.as_slice());
        assert_eq!(read_message(&mut reader).unwrap(), Some(message));
        assert_eq!(read_message(&mut reader).unwrap(), Some(json!({"id": 2})));
        assert_eq!(read_message(&mut reader).unwrap(), None, "clean EOF");
    }

    #[test]
    fn frames_count_bytes_not_chars() {
        let message = json!({"m": "héllo 🎉"});
        let mut buffer = Vec::new();
        write_message(&mut buffer, &message).unwrap();
        let text = String::from_utf8(buffer.clone()).unwrap();
        let body_len: usize = text
            .split("\r\n")
            .next()
            .unwrap()
            .trim_start_matches("Content-Length:")
            .trim()
            .parse()
            .unwrap();
        assert_eq!(body_len, serde_json::to_vec(&message).unwrap().len());
        let mut reader = std::io::BufReader::new(buffer.as_slice());
        assert_eq!(read_message(&mut reader).unwrap(), Some(message));
    }

    #[test]
    fn tolerates_extra_headers_and_case() {
        let body = br#"{"ok":true}"#;
        let mut framed = Vec::new();
        framed.extend_from_slice(b"content-type: application/vscode-jsonrpc\r\n");
        framed.extend_from_slice(format!("CONTENT-LENGTH: {}\r\n\r\n", body.len()).as_bytes());
        framed.extend_from_slice(body);
        let mut reader = std::io::BufReader::new(framed.as_slice());
        assert_eq!(
            read_message(&mut reader).unwrap(),
            Some(json!({"ok": true}))
        );
    }
}
