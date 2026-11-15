//! AI Terminal's MCP server: the tools agent sessions get to work with the app itself (list
//! sessions and projects, rename or open a session, read the selection, …).
//!
//! Transport: MCP "streamable HTTP" on 127.0.0.1 (random port), JSON responses, bearer-token
//! auth. Each session is told the URL / token through `--mcp-config` (Claude) or `-c
//! mcp_servers…` (Codex), plus a header naming the calling session, so tools can act on "this
//! session". The HTTP thread hands tool calls to the UI thread (which owns the sessions) and
//! waits for the answer.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// Header carrying the calling session's key (set in the MCP config of each session).
pub const SESSION_HEADER: &str = "x-ai-terminal-session";
pub const SERVER_NAME: &str = "ai-terminal";

/// A tool call for the UI thread.
pub struct Call {
    pub tool: String,
    pub args: Value,
    /// The calling session's hook key, when the request carried it.
    pub session: Option<String>,
    pub reply: Sender<Result<Value, String>>,
}

pub struct Server {
    pub url: String,
    pub token: String,
    pub calls: Receiver<Call>,
}

/// Tool definitions (name, description, input schema), set by the app.
pub type Tools = Arc<Mutex<Vec<Value>>>;

fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..4u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i ^ std::process::id() as u64);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// Starts the server; `ctx` is woken when a call arrives.
pub fn start(tools: Tools, ctx: egui::Context) -> std::io::Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let token = random_token();
    let (tx, rx) = channel::<Call>();
    let tok = token.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, tools, tok, ctx) = (tx.clone(), tools.clone(), tok.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = handle(stream, &tx, &tools, &tok, &ctx);
            });
        }
    });
    Ok(Server { url: format!("http://127.0.0.1:{port}/mcp"), token, calls: rx })
}

fn respond(mut s: &TcpStream, status: &str, body: &str, extra: &str) -> std::io::Result<()> {
    write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}", body.len())?;
    s.flush()
}

fn handle(stream: TcpStream, tx: &Sender<Call>, tools: &Tools, token: &str, ctx: &egui::Context) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut r = BufReader::new(&stream);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut headers = std::collections::HashMap::new();
    loop {
        let mut h = String::new();
        r.read_line(&mut h)?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    if !path.starts_with("/mcp") {
        return respond(&stream, "404 Not Found", "{}", "");
    }
    if headers.get("authorization").map(String::as_str) != Some(&format!("Bearer {token}")) {
        return respond(&stream, "401 Unauthorized", r#"{"error":"unauthorized"}"#, "");
    }
    match method {
        "POST" => {}
        // No server-initiated stream; session teardown is a no-op.
        "GET" => return respond(&stream, "405 Method Not Allowed", "{}", "Allow: POST\r\n"),
        "DELETE" => return respond(&stream, "200 OK", "{}", ""),
        _ => return respond(&stream, "405 Method Not Allowed", "{}", "Allow: POST\r\n"),
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0u8; len.min(8 << 20)];
    r.read_exact(&mut body)?;
    let session = headers.get(SESSION_HEADER).cloned();
    let Ok(req) = serde_json::from_slice::<Value>(&body) else {
        return respond(&stream, "400 Bad Request", &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}).to_string(), "");
    };
    // Batches are allowed by the spec; answer each request in order.
    let reqs: Vec<Value> = if let Value::Array(a) = req { a } else { vec![req] };
    let mut answers = Vec::new();
    for q in reqs {
        if let Some(a) = answer(&q, tx, tools, session.clone(), ctx) {
            answers.push(a);
        }
    }
    match answers.len() {
        0 => respond(&stream, "202 Accepted", "", ""),
        1 => respond(&stream, "200 OK", &answers[0].to_string(), "Mcp-Session-Id: ai-terminal\r\n"),
        _ => respond(&stream, "200 OK", &Value::Array(answers).to_string(), "Mcp-Session-Id: ai-terminal\r\n"),
    }
}

/// One JSON-RPC message → its response (None for notifications).
fn answer(q: &Value, tx: &Sender<Call>, tools: &Tools, session: Option<String>, ctx: &egui::Context) -> Option<Value> {
    let id = q.get("id").cloned()?; // notifications (no id) get no response
    let method = q["method"].as_str().unwrap_or("");
    let ok = |result: Value| json!({"jsonrpc":"2.0","id":id,"result":result});
    let err = |code: i64, msg: &str| json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":msg}});
    Some(match method {
        "initialize" => {
            let asked = q["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26").to_string();
            ok(json!({
                "protocolVersion": asked,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": SERVER_NAME, "title": "AI Terminal", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Tools for the AI Terminal app this session runs in: list and rename sessions, open projects and sessions, read the user's selection. 'This session' means the session making the call."
            }))
        }
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({"tools": tools.lock().unwrap().clone()})),
        "tools/call" => {
            let name = q["params"]["name"].as_str().unwrap_or("").to_string();
            let args = q["params"]["arguments"].clone();
            let (rtx, rrx) = channel();
            if tx.send(Call { tool: name, args, session, reply: rtx }).is_err() {
                return Some(err(-32603, "AI Terminal is shutting down"));
            }
            ctx.request_repaint();
            match rrx.recv_timeout(Duration::from_secs(20)) {
                Ok(Ok(v)) => ok(json!({"content":[{"type":"text","text": if v.is_string() { v.as_str().unwrap().to_string() } else { serde_json::to_string_pretty(&v).unwrap_or_default() }}], "isError": false})),
                Ok(Err(e)) => ok(json!({"content":[{"type":"text","text": e}], "isError": true})),
                Err(_) => ok(json!({"content":[{"type":"text","text":"AI Terminal didn't answer in time (is the window responding?)"}], "isError": true})),
            }
        }
        m if m.starts_with("notifications/") => return None,
        _ => err(-32601, &format!("Method not found: {method}")),
    })
}

/// Reads the JSON body of a raw HTTP response (tests).
#[cfg(test)]
fn http_post(url: &str, token: &str, body: &str) -> (u16, String) {
    let addr = url.trim_start_matches("http://").split('/').next().unwrap().to_string();
    let mut s = TcpStream::connect(addr).unwrap();
    write!(s, "POST /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n{SESSION_HEADER}: key-1\r\n\r\n{body}", body.len()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, out.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_round_trip() {
        let tools: Tools = Arc::new(Mutex::new(vec![json!({"name":"echo","description":"d","inputSchema":{"type":"object"}})]));
        let srv = start(tools, egui::Context::default()).unwrap();
        let (url, tok) = (srv.url.clone(), srv.token.clone());
        // Answer calls like the UI thread would.
        std::thread::spawn(move || {
            while let Ok(c) = srv.calls.recv() {
                let _ = c.reply.send(Ok(json!({"tool": c.tool, "session": c.session, "args": c.args})));
            }
        });
        let (st, body) = http_post(&url, &tok, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#);
        assert_eq!(st, 200);
        assert!(body.contains("\"protocolVersion\":\"2025-06-18\""), "{body}");
        let (st, _) = http_post(&url, &tok, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert_eq!(st, 202);
        let (_, body) = http_post(&url, &tok, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        assert!(body.contains("\"echo\""));
        let (_, body) = http_post(&url, &tok, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"x":1}}}"#);
        assert!(body.contains("key-1") && body.contains("echo"), "{body}");
        let (st, _) = http_post(&url, "wrong", r#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#);
        assert_eq!(st, 401);
    }
}
