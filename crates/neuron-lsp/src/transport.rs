//! JSON-RPC framing over a byte stream (`Content-Length` headers).

use crate::Server;
use serde_json::Value;
use std::io::{self, BufRead, Write};

fn read_message<R: BufRead>(input: &mut R) -> io::Result<Option<Value>> {
    let mut len: Option<usize> = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        let lower = t.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            len = v.trim().parse().ok();
        }
    }
    let Some(n) = len else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message without Content-Length",
        ));
    };
    let mut buf = vec![0u8; n];
    input.read_exact(&mut buf)?;
    // A body that is not JSON is ignored, as the protocol asks of a server.
    Ok(Some(serde_json::from_slice(&buf).unwrap_or(Value::Null)))
}

fn write_message<W: Write>(output: &mut W, msg: &Value) -> io::Result<()> {
    let body = serde_json::to_string(msg)?;
    write!(output, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    output.flush()
}

/// Serves one session until the client sends `exit` or closes the stream.
///
/// Returns the exit code the protocol asks for: 0 after `shutdown`, 1 otherwise.
///
/// # Errors
/// Fails when the streams fail or a message has no `Content-Length`.
pub fn serve<R: BufRead, W: Write>(mut input: R, mut output: W) -> io::Result<i32> {
    let mut server = Server::new();
    loop {
        let Some(msg) = read_message(&mut input)? else {
            return Ok(i32::from(!server.was_shut_down()));
        };
        for out in server.handle(&msg) {
            write_message(&mut output, &out)?;
        }
        if server.exit_requested() {
            return Ok(i32::from(!server.was_shut_down()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(v: &Value) -> Vec<u8> {
        let body = v.to_string();
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
    }

    #[test]
    fn a_session_runs_from_initialize_to_exit() {
        let mut input = Vec::new();
        input.extend(frame(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        ));
        input.extend(frame(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}),
        ));
        input.extend(frame(&json!({"jsonrpc": "2.0", "method": "exit"})));
        let mut output = Vec::new();
        let code = serve(io::Cursor::new(input), &mut output).unwrap_or(99);
        assert_eq!(code, 0);
        let text = String::from_utf8_lossy(&output);
        assert_eq!(text.matches("Content-Length:").count(), 2, "{text}");
        assert!(text.contains("\"hoverProvider\":true"), "{text}");
    }

    #[test]
    fn a_stream_that_ends_without_shutdown_is_an_error_exit() {
        let code = serve(io::Cursor::new(Vec::new()), Vec::new()).unwrap_or(99);
        assert_eq!(code, 1);
    }
}
