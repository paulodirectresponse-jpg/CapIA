use crate::PortError;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

/// JSON-RPC mínimo entre o EnginePort e o host da AgentSurface.
pub trait Transport: Send + Sync + core::fmt::Debug {
    /// Chama `method` com `params`; devolve `result` ou erro estruturado.
    fn call(&self, method: &str, params: Value) -> Result<Value, PortError>;
}

fn unwrap_reply(reply: &Value) -> Result<Value, PortError> {
    if reply["ok"].as_bool() == Some(true) {
        return Ok(reply["result"].clone());
    }
    let e = &reply["error"];
    Err(PortError::new(
        e["code"].as_str().unwrap_or("TRANSPORT"),
        e["message"].as_str().unwrap_or("rpc failed"),
    ))
}

/// Sidecar (Bun/Node) falando JSON-lines por stdin/stdout.
pub struct StdioTransport {
    io: Mutex<(ChildStdin, BufReader<ChildStdout>, u64)>,
    child: Mutex<Child>,
}

impl core::fmt::Debug for StdioTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StdioTransport")
    }
}

impl StdioTransport {
    pub fn spawn(command: &mut Command) -> Result<Self, PortError> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| PortError::new("SPAWN", e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PortError::new("SPAWN", "no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PortError::new("SPAWN", "no stdout"))?;
        Ok(Self {
            io: Mutex::new((stdin, BufReader::new(stdout), 0)),
            child: Mutex::new(child),
        })
    }
}

impl Transport for StdioTransport {
    fn call(&self, method: &str, params: Value) -> Result<Value, PortError> {
        let mut guard = self
            .io
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (stdin, stdout, next_id) = &mut *guard;
        *next_id += 1;
        let id = *next_id;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        writeln!(stdin, "{line}").map_err(|e| PortError::new("TRANSPORT", e.to_string()))?;
        stdin
            .flush()
            .map_err(|e| PortError::new("TRANSPORT", e.to_string()))?;
        let mut reply = String::new();
        let n = stdout
            .read_line(&mut reply)
            .map_err(|e| PortError::new("TRANSPORT", e.to_string()))?;
        if n == 0 {
            return Err(PortError::new("TRANSPORT", "host closed the pipe"));
        }
        let value: Value = serde_json::from_str(&reply)
            .map_err(|e| PortError::new("TRANSPORT", format!("bad reply: {e}")))?;
        unwrap_reply(&value)
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// HTTP/1.1 (`POST /rpc`, corpo JSON) para um host que repassa à página viva do editor
/// (`window.__capiaAgent.call`). Sem dependências de rede: `std::net` apenas, só loopback.
#[derive(Debug)]
pub struct HttpTransport {
    addr: String,
}

impl HttpTransport {
    /// `addr` = `127.0.0.1:PORT`. Endereços não-loopback são recusados.
    pub fn new(addr: impl Into<String>) -> Result<Self, PortError> {
        let addr = addr.into();
        if !(addr.starts_with("127.0.0.1:")
            || addr.starts_with("localhost:")
            || addr.starts_with("[::1]:"))
        {
            return Err(PortError::new(
                "NOT_LOOPBACK",
                "the agent bridge is loopback only",
            ));
        }
        Ok(Self { addr })
    }
}

impl Transport for HttpTransport {
    fn call(&self, method: &str, params: Value) -> Result<Value, PortError> {
        let body = json!({"method": method, "params": params}).to_string();
        let mut stream = TcpStream::connect(&self.addr)
            .map_err(|e| PortError::new("TRANSPORT", e.to_string()))?;
        let request = format!(
            "POST /rpc HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.addr,
            body.len(),
            body
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| PortError::new("TRANSPORT", e.to_string()))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| PortError::new("TRANSPORT", e.to_string()))?;
        let text = String::from_utf8_lossy(&raw);
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, b)| b)
            .ok_or_else(|| PortError::new("TRANSPORT", "malformed http reply"))?;
        let value: Value = serde_json::from_str(body)
            .map_err(|e| PortError::new("TRANSPORT", format!("bad reply: {e}")))?;
        unwrap_reply(&value)
    }
}
