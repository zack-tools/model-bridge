//! Conservative offline inspection of Responses SSE. This does not prove which
//! account handled a request and must not by itself mark a model live-verified.
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

const MAX_BYTES: usize = 1024 * 1024;

#[derive(PartialEq, Eq, Serialize)]
pub struct StreamEvidence {
    #[serde(skip)]
    pub function_calls: Vec<ControlledCall>,
    #[serde(skip)]
    pub output_text: String,
    #[serde(skip)]
    output_items: Vec<Value>,
    pub completed: bool,
    pub saw_stream_delta: bool,
    pub has_text: bool,
    pub has_function_call: bool,
    /// Correlation hint only; never sufficient for live account verification.
    pub account_trace_matches: Option<bool>,
}

#[derive(PartialEq, Eq, Serialize)]
pub struct ControlledCall {
    #[serde(skip)]
    pub name: String,
    #[serde(skip)]
    pub call_id: String,
    #[serde(skip)]
    pub arguments: Value,
}

impl std::fmt::Debug for StreamEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamEvidence").field("completed", &self.completed)
            .field("saw_stream_delta", &self.saw_stream_delta).field("has_text", &self.has_text)
            .field("has_function_call", &self.has_function_call)
            .field("account_trace_matches", &self.account_trace_matches).finish()
    }
}
impl std::fmt::Debug for ControlledCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ControlledCall([redacted])")
    }
}

pub struct StreamInspector {
    expected_model: String,
    bytes: Vec<u8>,
}

impl StreamInspector {
    pub fn new(expected_model: &str) -> Result<Self, &'static str> {
        if expected_model.trim().is_empty() { return Err("model is required"); }
        Ok(Self { expected_model: expected_model.into(), bytes: Vec::new() })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<(), &'static str> {
        if self.bytes.len().saturating_add(chunk.len()) > MAX_BYTES { return Err("stream exceeds limit"); }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    pub fn finish(self) -> Result<StreamEvidence, &'static str> {
        let raw = std::str::from_utf8(&self.bytes).map_err(|_| "invalid UTF-8")?;
        let normalized = raw.replace("\r\n", "\n");
        if !normalized.ends_with("\n\n") { return Err("incomplete event frame"); }
        let mut text_delta: HashMap<String, String> = HashMap::new();
        let mut call_delta: HashMap<String, String> = HashMap::new();
        let mut saw_done = false;
        let mut completed: Option<Value> = None;
        for frame in normalized.split("\n\n").filter(|s| !s.is_empty()) {
            let mut event = None;
            let mut data = Vec::new();
            for line in frame.lines() {
                if line.starts_with(':') { continue; }
                if let Some(v) = line.strip_prefix("event:") { event = Some(v.trim()); }
                if let Some(v) = line.strip_prefix("data:") { data.push(v.strip_prefix(' ').unwrap_or(v)); }
            }
            if data.is_empty() { continue; }
            let data = data.join("\n");
            if saw_done { return Err("event after stream terminator"); }
            if data == "[DONE]" {
                if completed.is_none() { return Err("stream terminator before completion"); }
                saw_done = true;
                continue;
            }
            let value: Value = serde_json::from_str(&data).map_err(|_| "invalid event JSON")?;
            let kind = value.get("type").and_then(Value::as_str).ok_or("event type missing")?;
            if event.is_some_and(|v| v != kind) { return Err("event type mismatch"); }
            if completed.is_some() { return Err("event after completion"); }
            match kind {
                "error" | "response.failed" | "response.incomplete" => return Err("upstream response did not complete"),
                "response.output_text.delta" | "response.function_call_arguments.delta" => {
                    let id = value.get("item_id").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or("delta item id missing")?;
                    let delta = value.get("delta").and_then(Value::as_str).ok_or("delta missing")?;
                    if kind == "response.output_text.delta" { text_delta.entry(id.to_string()).or_default().push_str(delta); }
                    else { call_delta.entry(id.to_string()).or_default().push_str(delta); }
                }
                "response.completed" => completed = Some(value),
                _ => {}
            }
        }
        let completed = completed.ok_or("completion event missing")?;
        let response = completed.get("response").ok_or("response missing")?;
        if response.get("status").and_then(Value::as_str) != Some("completed") { return Err("response not completed"); }
        if response.get("model").and_then(Value::as_str) != Some(&self.expected_model) { return Err("response model differs"); }
        let items = response.get("output").and_then(Value::as_array).ok_or("output missing")?;
        let mut has_text = false;
        let mut has_function_call = false;
        let mut function_calls = Vec::new();
        let mut output_text = String::new();
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if item.get("role").and_then(Value::as_str) != Some("assistant") { continue; }
                    if let Some(parts) = item.get("content").and_then(Value::as_array) {
                        let final_text: String = parts.iter().filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
                            .filter_map(|part| part.get("text").and_then(Value::as_str)).collect();
                        if !final_text.is_empty() {
                            let id = item.get("id").and_then(Value::as_str).ok_or("message id missing")?;
                            if text_delta.remove(id).as_deref() != Some(final_text.as_str()) {
                                return Err("message text does not match streamed delta");
                            }
                            has_text |= !final_text.trim().is_empty();
                            output_text.push_str(&final_text);
                        }
                    }
                }
                Some("function_call") => {
                    let present = |key| item.get(key).and_then(Value::as_str).is_some_and(|v| !v.is_empty());
                    let arguments = item.get("arguments").and_then(Value::as_str);
                    let valid_args = arguments.and_then(|v| serde_json::from_str::<Value>(v).ok()).is_some_and(|v| v.is_object());
                    let id = item.get("id").and_then(Value::as_str).ok_or("function call id missing")?;
                    if !present("name") || !present("call_id") || !valid_args
                        || call_delta.remove(id).as_deref() != arguments {
                        return Err("function call does not match streamed delta");
                    }
                    has_function_call = true;
                    function_calls.push(ControlledCall { name: item["name"].as_str().unwrap().to_owned(), call_id: item["call_id"].as_str().unwrap().to_owned(), arguments: serde_json::from_str(arguments.unwrap()).unwrap() });
                }
                _ => {}
            }
        }
        if !text_delta.is_empty() || !call_delta.is_empty() { return Err("unmatched streamed delta"); }
        if !(has_text || has_function_call) { return Err("no streamed usable output"); }
        Ok(StreamEvidence { function_calls, output_text, output_items: items.clone(), completed: true, saw_stream_delta: true, has_text, has_function_call, account_trace_matches: None })
    }
}

/// Execute a controlled two-turn probe. The supplied result is fixture data; no
/// arbitrary tool is executed. A successful probe is not account attribution.
pub fn inspect_controlled_tool_roundtrip(
    port: u16, key: &str, request: &Value, model: &str,
    timeout: std::time::Duration, expected_name: &str, expected_arguments: &Value,
    tool_result: &str, expected_reply: &str,
) -> Result<(), String> {
    inspect_controlled_tool_roundtrip_with_index(
        port, key, request, model, timeout, expected_name, expected_arguments,
        tool_result, expected_reply, None,
    )
}

pub fn inspect_controlled_tool_roundtrip_with_index(
    port: u16, key: &str, request: &Value, model: &str,
    timeout: std::time::Duration, expected_name: &str, expected_arguments: &Value,
    tool_result: &str, expected_reply: &str, expected_index: Option<&str>,
) -> Result<(), String> {
    if expected_name.is_empty() || !expected_arguments.is_object() || expected_reply.is_empty() {
        return Err("invalid controlled probe".into());
    }
    let start = std::time::Instant::now();
    let first = inspect_loopback_stream_with_index(port, key, request, model, timeout, expected_index)?;
    if expected_index.is_some() && first.account_trace_matches != Some(true) {
        return Err("first turn account trace attribution failed".into());
    }
    if first.function_calls.len() != 1 { return Err("expected exactly one tool call".into()); }
    let call = &first.function_calls[0];
    if call.name != expected_name || &call.arguments != expected_arguments {
        return Err("unexpected tool call".into());
    }
    let mut second = request.clone();
    let input = second.get_mut("input").and_then(Value::as_array_mut).ok_or("input must be an array")?;
    input.extend(first.output_items.iter().cloned());
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|_| "challenge generation failed")?;
    let challenge: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let result = format!("{tool_result}\nChallenge: {challenge}\nReply with exactly: {expected_reply}{challenge}");
    input.push(serde_json::json!({"type":"function_call_output", "call_id":call.call_id, "output":result}));
    let remaining = timeout.checked_sub(start.elapsed()).filter(|d| !d.is_zero()).ok_or("response deadline exceeded")?;
    let reply = inspect_loopback_stream_with_index(port, key, &second, model, remaining, expected_index)?;
    if expected_index.is_some() && reply.account_trace_matches != Some(true) {
        return Err("second turn account trace attribution failed".into());
    }
    if !reply.function_calls.is_empty() || reply.output_text != format!("{expected_reply}{challenge}") {
        return Err("unexpected tool roundtrip reply".into());
    }
    Ok(())
}

/// Execute a real model two-turn tool roundtrip with strict account attribution checking.
/// Requires explicit user consent before calling due to potential quota consumption.
pub fn inspect_real_model_tool_roundtrip(
    port: u16, key: &str, request: &Value, model: &str,
    timeout: std::time::Duration, expected_tool_name: &str, expected_index: &str,
    check_current: impl Fn() -> Result<(), String>,
) -> Result<String, String> {
    if expected_tool_name.is_empty() || expected_index.is_empty() {
        return Err("invalid tool probe parameters".into());
    }
    check_current()?;
    let start = std::time::Instant::now();
    let first = inspect_loopback_stream_with_index(port, key, request, model, timeout, Some(expected_index))?;
    if first.account_trace_matches != Some(true) {
        return Err("第一回合缺少或不符合帳號歸屬追蹤標頭 (X-CPA-TRACE-ID)".into());
    }
    if first.function_calls.len() != 1 {
        return Err(format!("第一回合預期單一工具呼叫，但收到 {} 個", first.function_calls.len()));
    }
    let call = &first.function_calls[0];
    if call.name != expected_tool_name {
        return Err(format!("呼叫工具名稱不符（收到 {}，預期 {}）", call.name, expected_tool_name));
    }

    let mut second = request.clone();
    let input = second.get_mut("input").and_then(Value::as_array_mut).ok_or("input must be an array")?;
    input.extend(first.output_items.iter().cloned());
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|_| "隨機挑戰生成失敗")?;
    let challenge: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let expected_token = format!("probe-verified-{challenge}");
    let tool_output = format!("Status: operational. Verification challenge: {challenge}. Reply with exactly: {expected_token}");
    input.push(serde_json::json!({"type":"function_call_output", "call_id":call.call_id, "output":tool_output}));

    check_current()?;
    let remaining = timeout.checked_sub(start.elapsed()).filter(|d| !d.is_zero()).ok_or("已超過回應時限")?;
    let reply = inspect_loopback_stream_with_index(port, key, &second, model, remaining, Some(expected_index))?;
    check_current()?;
    if reply.account_trace_matches != Some(true) {
        return Err("第二回合缺少或不符合帳號歸屬追蹤標頭 (X-CPA-TRACE-ID)".into());
    }
    if !reply.function_calls.is_empty() {
        return Err("第二回合不應再產生額外工具呼叫".into());
    }
    if !reply.output_text.contains(&expected_token) {
        return Err("第二回合模型回覆未包含預期之隨機驗證字串".into());
    }
    Ok(format!("模型 {model} 雙回合工具串流驗證通過（第一回合與第二回合帳號追蹤皆匹配，隨機挑戰已核對）"))
}

struct DeadlineStream {
    stream: std::net::TcpStream,
    deadline: std::time::Instant,
}

impl std::io::Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.deadline.checked_duration_since(std::time::Instant::now())
            .filter(|d| !d.is_zero()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "response deadline exceeded"))?;
        self.stream.set_read_timeout(Some(left))?;
        std::io::Read::read(&mut self.stream, buf)
    }
}

/// Compare only the upstream-generated 16-hex index format. Unknown/custom
/// formats are not attributed; a match remains a hint, not live verification.
fn trace_index_matches(trace: &str, expected: &str) -> Option<bool> {
    if expected.len() != 16 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let bytes = trace.as_bytes();
    if bytes.len() < 34 || !bytes[..14].iter().all(u8::is_ascii_digit) || bytes[14] != b'-'
        || bytes[31] != b'-' || !bytes[15..31].iter().all(u8::is_ascii_hexdigit)
        || !bytes[32..].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-') {
        return None;
    }
    Some(bytes[15..31].eq_ignore_ascii_case(expected.as_bytes()))
}

/// Inspect one loopback response without claiming account attribution or live verification.
pub fn inspect_loopback_stream(port: u16, key: &str, request: &Value, model: &str, timeout: std::time::Duration) -> Result<StreamEvidence, String> {
    inspect_loopback_stream_with_index(port, key, request, model, timeout, None)
}

pub fn inspect_loopback_stream_with_index(port: u16, key: &str, request: &Value, model: &str, timeout: std::time::Duration, expected_index: Option<&str>) -> Result<StreamEvidence, String> {
    use std::io::{BufRead, Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};
    if port == 0 || key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("invalid gateway credentials or port".into());
    }
    if request.get("stream").and_then(Value::as_bool) != Some(true) || request.get("model").and_then(Value::as_str) != Some(model) {
        return Err("request must stream the expected model".into());
    }
    let deadline = std::time::Instant::now().checked_add(timeout).ok_or("invalid timeout")?;
    let body = serde_json::to_vec(request).map_err(|_| "invalid request")?;
    if body.len() > MAX_BYTES { return Err("request exceeds limit".into()); }
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, deadline.checked_duration_since(std::time::Instant::now()).filter(|d| !d.is_zero()).ok_or("response deadline exceeded")?).map_err(|_| "loopback connection failed")?;
    stream.set_read_timeout(Some(timeout)).map_err(|_| "read timeout setup failed")?;

    let headers = format!("POST /v1/responses HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {key}\r\nContent-Type: application/json\r\nAccept: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    for mut data in [headers.as_bytes(), body.as_slice()] {
        while !data.is_empty() {
            let left = deadline.checked_duration_since(std::time::Instant::now()).filter(|d| !d.is_zero()).ok_or("response deadline exceeded")?;
            stream.set_write_timeout(Some(left)).map_err(|_| "write timeout setup failed")?;
            let n = stream.write(data).map_err(|_| "request write failed")?;
            if n == 0 { return Err("request write failed".into()); }
            data = &data[n..];
        }
    }
    let mut reader = std::io::BufReader::new(DeadlineStream { stream, deadline });
    let mut header = Vec::new();
    loop {
        let mut line = Vec::new();
        (&mut reader).take(8193).read_until(b'\n', &mut line).map_err(|_| "response header read failed")?;
        if !line.ends_with(b"\r\n") || header.len().saturating_add(line.len()) > 16384 { return Err("invalid response headers".into()); }
        header.extend_from_slice(&line);
        if line == b"\r\n" { break; }
    }
    let header = std::str::from_utf8(&header).map_err(|_| "invalid response headers")?;
    let mut lines = header.split("\r\n");
    let status = lines.next().ok_or("missing status")?;
    if !status.starts_with("HTTP/1.1 200 ") && status != "HTTP/1.1 200" { return Err("response not successful".into()); }
    let mut transfer = None;
    let mut content_type = None;
    let mut trace = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("invalid response header")?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) { return Err("invalid response header".into()); }
        if name.eq_ignore_ascii_case("content-length") { return Err("ambiguous response framing".into()); }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if transfer.replace(value.trim()).is_some() { return Err("duplicate transfer encoding".into()); }
        }
        if name.eq_ignore_ascii_case("content-type") {
            if content_type.replace(value.trim()).is_some() { return Err("duplicate content type".into()); }
        }
        if name.eq_ignore_ascii_case("x-cpa-trace-id") {
            if trace.replace(value.trim()).is_some() { return Err("duplicate trace header".into()); }
        }
    }
    if !transfer.is_some_and(|value| value.eq_ignore_ascii_case("chunked")) { return Err("chunked transfer required".into()); }
    if !content_type.and_then(|s| s.split(';').next()).is_some_and(|s| s.trim().eq_ignore_ascii_case("text/event-stream")) { return Err("event stream required".into()); }
    let mut inspector = StreamInspector::new(model).map_err(str::to_string)?;
    crate::http::decode_chunked(&mut reader, MAX_BYTES, |bytes| inspector.push(bytes).map_err(str::to_string))?;
    let mut evidence = inspector.finish().map_err(str::to_string)?;
    evidence.account_trace_matches = trace.zip(expected_index).and_then(|(value, index)| trace_index_matches(value, index));
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(s: &str) -> Result<StreamEvidence, &'static str> {
        let mut inspector = StreamInspector::new("model-a").unwrap();
        for chunk in s.as_bytes().chunks(7) { inspector.push(chunk)?; }
        inspector.finish()
    }
    const DELTA: &str = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"item-1\",\"delta\":\"hello\"}\n\n";
    const COMPLETE: &str = "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"model\":\"model-a\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"id\":\"item-1\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello\"}]}]}}\n\n";
    #[test]
    fn controlled_roundtrip_sends_matching_call_id_and_result() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            for turn in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") { let mut b = [0]; socket.read_exact(&mut b).unwrap(); headers.push(b[0]); }
                let h = String::from_utf8(headers).unwrap();
                let n: usize = h.lines().find_map(|l| l.strip_prefix("Content-Length: ")).unwrap().parse().unwrap();
                let mut body = vec![0; n]; socket.read_exact(&mut body).unwrap();
                if turn == 1 {
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let input = request["input"].as_array().unwrap();
                    assert_eq!(input[1]["call_id"], "call-1");
                    assert_eq!(input[2]["call_id"], "call-1");
                    assert!(input[2]["output"].as_str().unwrap().starts_with("fixture-result\nChallenge: "));
                }
                let request: Value = serde_json::from_slice(&body).unwrap();
                let events = if turn == 0 {
                    "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"i\",\"delta\":\"{}\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"model\":\"model-a\",\"output\":[{\"type\":\"function_call\",\"id\":\"i\",\"name\":\"check\",\"call_id\":\"call-1\",\"arguments\":\"{}\"}]}}\n\n".to_string()
                } else {
                    let output = request["input"][2]["output"].as_str().unwrap();
                    let challenge = output.lines().nth(1).unwrap().strip_prefix("Challenge: ").unwrap();
                    let reply = format!("hello{challenge}");
                    format!("{}{}", DELTA.replace("hello", &reply), COMPLETE.replace("hello", &reply))
                };
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n", events.len(), events).unwrap();
            }
        });
        let request = serde_json::json!({"model":"model-a","stream":true,"input":[{"role":"user","content":"test"}]});
        assert!(inspect_controlled_tool_roundtrip(port, "fixture", &request, "model-a", std::time::Duration::from_secs(2), "check", &serde_json::json!({}), "fixture-result", "hello").is_ok());
        worker.join().unwrap();
    }

    #[test]
    fn controlled_roundtrip_with_account_attribution_requires_matching_trace() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            for turn in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") { let mut b = [0]; socket.read_exact(&mut b).unwrap(); headers.push(b[0]); }
                let h = String::from_utf8(headers).unwrap();
                let n: usize = h.lines().find_map(|l| l.strip_prefix("Content-Length: ")).unwrap().parse().unwrap();
                let mut body = vec![0; n]; socket.read_exact(&mut body).unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                let events = if turn == 0 {
                    "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"i\",\"delta\":\"{}\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"model\":\"model-a\",\"output\":[{\"type\":\"function_call\",\"id\":\"i\",\"name\":\"check\",\"call_id\":\"call-1\",\"arguments\":\"{}\"}]}}\n\n".to_string()
                } else {
                    let output = request["input"][2]["output"].as_str().unwrap();
                    let challenge = output.lines().nth(1).unwrap().strip_prefix("Challenge: ").unwrap();
                    let reply = format!("hello{challenge}");
                    format!("{}{}", DELTA.replace("hello", &reply), COMPLETE.replace("hello", &reply))
                };
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nX-CPA-TRACE-ID: 20261107123456-0123456789abcdef-12345678\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n", events.len(), events).unwrap();
            }
        });
        let request = serde_json::json!({"model":"model-a","stream":true,"input":[{"role":"user","content":"test"}]});
        // Matching index succeeds
        assert!(inspect_controlled_tool_roundtrip_with_index(port, "fixture", &request, "model-a", std::time::Duration::from_secs(2), "check", &serde_json::json!({}), "fixture-result", "hello", Some("0123456789abcdef")).is_ok());
        worker.join().unwrap();

        // Mismatched index fails
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") { let mut b = [0]; socket.read_exact(&mut b).unwrap(); headers.push(b[0]); }
            let h = String::from_utf8(headers).unwrap();
            let n: usize = h.lines().find_map(|l| l.strip_prefix("Content-Length: ")).unwrap().parse().unwrap();
            let mut body = vec![0; n]; socket.read_exact(&mut body).unwrap();
            let events = "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"i\",\"delta\":\"{}\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"model\":\"model-a\",\"output\":[{\"type\":\"function_call\",\"id\":\"i\",\"name\":\"check\",\"call_id\":\"call-1\",\"arguments\":\"{}\"}]}}\n\n";
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nX-CPA-TRACE-ID: 20261107123456-otherindex12345-12345678\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n", events.len(), events).unwrap();
        });
        let err = inspect_controlled_tool_roundtrip_with_index(port, "fixture", &request, "model-a", std::time::Duration::from_secs(2), "check", &serde_json::json!({}), "fixture-result", "hello", Some("0123456789abcdef")).unwrap_err();
        assert!(err.contains("first turn account trace attribution failed"));
        worker.join().unwrap();
    }

    #[test]
    fn evidence_formatting_omits_sensitive_content() {
        let evidence = check(&format!("{DELTA}{COMPLETE}")).unwrap();
        assert!(!format!("{evidence:?}").contains("hello"));
        assert!(!serde_json::to_string(&evidence).unwrap().contains("hello"));
        let call = ControlledCall { name: "private-name".into(), call_id: "private-id".into(), arguments: serde_json::json!({"secret":"private-value"}) };
        for rendered in [format!("{call:?}"), serde_json::to_string(&call).unwrap()] {
            assert!(!rendered.contains("private"));
        }
    }

    #[test]
    fn trace_matching_is_bounded_and_conservative() {
        let trace = "20261107123456-0123456789abcdef-abcdef12-3456";
        assert_eq!(trace_index_matches(trace, "0123456789abcdef"), Some(true));
        assert_eq!(trace_index_matches(trace, "fedcba9876543210"), Some(false));
        assert_eq!(trace_index_matches(trace, "custom-index"), None);
        assert_eq!(trace_index_matches("20261107123456-0123456789abcdef-", "0123456789abcdef"), None);
        assert_eq!(trace_index_matches("bad-0123456789abcdef-1234", "0123456789abcdef"), None);
    }

    #[test]
    fn loopback_stream_rejects_truncated_chunk() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        fn run(truncated: bool) -> Result<StreamEvidence, String> {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let worker = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                loop {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") { break; }
                }
                let headers = String::from_utf8(request).unwrap();
                let length: usize = headers.lines().find_map(|line| line.strip_prefix("Content-Length: ")).unwrap().parse().unwrap();
                let mut body = vec![0; length];
                socket.read_exact(&mut body).unwrap();
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nX-CPA-TRACE-ID: 20261107123456-0123456789abcdef-12345678\r\n\r\n").unwrap();
                let events = format!("{DELTA}{COMPLETE}");
                if truncated {
                    write!(socket, "{:x}\r\n{}", events.len() + 20, events).unwrap();
                } else {
                    for part in events.as_bytes().chunks(11) {
                        write!(socket, "{:x}\r\n", part.len()).unwrap();
                        socket.write_all(part).unwrap();
                        socket.write_all(b"\r\n").unwrap();
                    }
                    socket.write_all(b"0\r\n\r\n").unwrap();
                }
            });
            let result = inspect_loopback_stream_with_index(port, "fixture-key", &serde_json::json!({"model":"model-a","stream":true}), "model-a", std::time::Duration::from_secs(2), Some("0123456789abcdef"));
            worker.join().unwrap();
            result
        }
        let evidence = run(false).unwrap();
        assert!(evidence.has_text);
        assert_eq!(evidence.account_trace_matches, Some(true));
        assert!(run(true).is_err());
    }

    #[test]
    fn deadline_rejects_slow_stream_even_with_regular_bytes() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf);
            for byte in b"HTTP/1.1 200 OK\r\n" {
                if socket.write_all(&[*byte]).is_err() { break; }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        });
        let start = std::time::Instant::now();
        let result = inspect_loopback_stream(port, "fixture-key", &serde_json::json!({"model":"model-a","stream":true}), "model-a", std::time::Duration::from_millis(100));
        assert!(result.is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        worker.join().unwrap();
    }

    #[test]
    fn unmatched_extra_outputs_and_orphan_deltas_are_rejected() {
        let json = COMPLETE.lines().find_map(|line| line.strip_prefix("data: ")).unwrap();
        let mut event: Value = serde_json::from_str(json).unwrap();
        event["response"]["output"].as_array_mut().unwrap().push(serde_json::json!({
            "type":"message", "role":"assistant", "id":"other",
            "content":[{"type":"output_text", "text":"extra"}]
        }));
        let extra = format!("event: response.completed\ndata: {event}\n\n");
        assert!(check(&format!("{DELTA}{extra}")).is_err());
        let orphan = DELTA.replace("item-1", "orphan");
        assert!(check(&format!("{orphan}{DELTA}{COMPLETE}")).is_err());
    }

    #[test]
    fn malformed_delta_is_not_ignored() {
        let bad = "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"item-1\"}\n\n";
        assert!(check(&format!("{bad}{DELTA}{COMPLETE}")).is_err());
    }

    #[test]
    fn accepts_completed_stream_in_fragments() {
        let e = check(&format!("{DELTA}{COMPLETE}data: [DONE]\n\n")).unwrap();
        assert!(e.completed && e.has_text && !e.has_function_call);
    }
    #[test]
    fn rejects_truncation_failure_and_model_mismatch() {
        assert!(check(DELTA).is_err());
        assert!(check(&format!("{DELTA}{}", COMPLETE.trim_end())).is_err());
        assert!(check(&format!("{DELTA}{}", COMPLETE.replace("model-a", "model-b"))).is_err());
        assert!(check(&format!("{DELTA}data: {{\"type\":\"response.failed\"}}\n\n")).is_err());
        assert!(check(COMPLETE).is_err());
        assert!(check(&format!("{DELTA}{}", COMPLETE.replace("item-1", "other"))).is_err());
        assert!(check(&format!("{DELTA}{}", COMPLETE.replace("assistant", "user"))).is_err());
        assert!(check(&format!("{DELTA}{}", COMPLETE.replace("hello\"}]", "hello world\"}]"))).is_err());
        assert!(check(&format!("data: [DONE]\n\n{DELTA}{COMPLETE}")).is_err());
    }
    #[test]
    fn accepts_valid_tool_call_but_not_invalid_arguments() {
        let delta = "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"item-2\",\"delta\":\"{}\"}\n\n";
        let end = "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"model\":\"model-a\",\"output\":[{\"type\":\"function_call\",\"id\":\"item-2\",\"name\":\"check\",\"call_id\":\"call-1\",\"arguments\":\"{}\"}]}}\n\n";
        assert!(check(&format!("{delta}{end}")).unwrap().has_function_call);
        assert!(check(&format!("{delta}{}", end.replace("\"arguments\":\"{}\"", "\"arguments\":\"bad\""))).is_err());
    }
}
