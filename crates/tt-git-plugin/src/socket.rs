//! The tt-git API socket: newline-delimited JSON over a local socket (a Unix
//! domain socket, or a named pipe on Windows).
//!
//! ```text
//! → {"id":"req_1","method":"ui.notify","params":{"message":"hi"}}
//! ← {"id":"req_1","result":{}}
//! ```
//!
//! `events.subscribe` answers once and then streams one event envelope per
//! line until the client disconnects.

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, Stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, code: &str, message: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(ErrorBody {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

/// What the app sends back for a request.
pub enum Reply {
    Once(Response),
    /// Send `first`, then forward every line from `events`.
    Stream {
        first: Response,
        events: Receiver<EventLine>,
    },
}

/// A value as one NDJSON line, serialized once however many connections it
/// goes to.
#[derive(Debug, Clone)]
pub struct EventLine(Arc<[u8]>);

impl EventLine {
    pub fn new(value: &Value) -> serde_json::Result<Self> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        Ok(Self(line.into()))
    }

    /// The line, newline included.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Handle for answering one request; cheap to clone so it can travel inside
/// an app action. Only the first reply is delivered.
#[derive(Clone)]
pub struct Replier(Arc<Mutex<Option<Sender<Reply>>>>);

impl std::fmt::Debug for Replier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Replier")
    }
}

impl Replier {
    pub fn send(&self, reply: Reply) {
        if let Some(tx) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(reply);
        }
    }

    pub fn respond(&self, response: Response) {
        self.send(Reply::Once(response));
    }
}

/// A request that arrived on the socket, waiting for the app to answer.
#[derive(Debug, Clone)]
pub struct Call {
    pub request: Request,
    pub replier: Replier,
}

/// A fresh, per-instance socket address.
pub fn new_address() -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    if cfg!(windows) {
        format!(r"\\.\pipe\tt-git-{pid}-{nonce:x}")
    } else {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!("tt-git-{pid}-{nonce:x}.sock"))
            .to_string_lossy()
            .into_owned()
    }
}

/// The listening socket. Accepting runs on a background thread for the life
/// of the process; each connection gets its own thread.
pub struct Server {
    address: String,
}

impl Server {
    /// Listens on `address`; `on_call` is invoked (on connection threads) for
    /// every request and must eventually reply through the `Replier`.
    pub fn start(address: &str, on_call: impl Fn(Call) + Send + Sync + 'static) -> std::io::Result<Self> {
        let name = address.to_fs_name::<GenericFilePath>()?;
        let listener = ListenerOptions::new().name(name).try_overwrite(true).create_sync()?;
        let on_call = Arc::new(on_call);
        std::thread::Builder::new()
            .name("tt-git-socket".into())
            .stack_size(crate::IO_THREAD_STACK)
            .spawn(move || {
                for conn in listener.incoming() {
                    let Ok(conn) = conn else { continue };
                    let on_call = on_call.clone();
                    let _ = std::thread::Builder::new()
                        .name("tt-git-socket-conn".into())
                        .spawn(move || serve(conn, &*on_call));
                }
            })?;
        Ok(Self {
            address: address.to_string(),
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if !cfg!(windows) {
            let _ = std::fs::remove_file(&self.address);
        }
    }
}

fn write_line(w: &mut impl Write, value: &impl Serialize) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()
}

fn serve(conn: Stream, on_call: &(dyn Fn(Call) + Send + Sync)) {
    let mut reader = BufReader::new(conn);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::err(Value::Null, "invalid_request", e.to_string());
                if write_line(reader.get_mut(), &resp).is_err() {
                    return;
                }
                continue;
            }
        };
        let id = request.id.clone();
        let (tx, rx) = mpsc::channel();
        on_call(Call {
            request,
            replier: Replier(Arc::new(Mutex::new(Some(tx)))),
        });
        match rx.recv() {
            Ok(Reply::Once(resp)) => {
                if write_line(reader.get_mut(), &resp).is_err() {
                    return;
                }
            }
            Ok(Reply::Stream { first, events }) => {
                if write_line(reader.get_mut(), &first).is_err() {
                    return;
                }
                for ev in events {
                    let w = reader.get_mut();
                    if w.write_all(ev.as_bytes()).and_then(|()| w.flush()).is_err() {
                        return;
                    }
                }
                return;
            }
            Err(_) => {
                let resp = Response::err(id, "server_shutting_down", "tt-git did not answer");
                let _ = write_line(reader.get_mut(), &resp);
                return;
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("could not connect to tt-git at {address}: {source}")]
    Connect { address: String, source: std::io::Error },
    #[error("socket error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad response: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{code}: {message}")]
    Remote { code: String, message: String },
}

fn connect(address: &str) -> Result<BufReader<Stream>, ClientError> {
    let name = address.to_fs_name::<GenericFilePath>()?;
    let conn = Stream::connect(name).map_err(|source| ClientError::Connect {
        address: address.to_string(),
        source,
    })?;
    Ok(BufReader::new(conn))
}

fn send_request(conn: &mut BufReader<Stream>, method: &str, params: Value) -> Result<Value, ClientError> {
    let req = Request {
        id: Value::String("req_1".into()),
        method: method.to_string(),
        params,
    };
    write_line(conn.get_mut(), &req)?;
    let mut line = String::new();
    conn.read_line(&mut line)?;
    let resp: Response = serde_json::from_str(&line)?;
    match (resp.result, resp.error) {
        (_, Some(e)) => Err(ClientError::Remote {
            code: e.code,
            message: e.message,
        }),
        (Some(v), None) => Ok(v),
        (None, None) => Ok(Value::Null),
    }
}

/// Sends one request and returns its result.
pub fn call(address: &str, method: &str, params: Value) -> Result<Value, ClientError> {
    let mut conn = connect(address)?;
    send_request(&mut conn, method, params)
}

/// Calls `events.subscribe` and returns an iterator over streamed events.
pub fn subscribe(address: &str, params: Value) -> Result<impl Iterator<Item = Value>, ClientError> {
    let mut conn = connect(address)?;
    send_request(&mut conn, "events.subscribe", params)?;
    Ok(conn
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str(&l).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trip_and_errors() {
        let addr = new_address();
        let _server = Server::start(&addr, |call| {
            let Call { request, replier } = call;
            match request.method.as_str() {
                "ping" => replier.respond(Response::ok(request.id, json!({"pong": true}))),
                "echo" => replier.respond(Response::ok(request.id, request.params)),
                "events.subscribe" => {
                    let (tx, rx) = mpsc::channel();
                    for i in 0..3 {
                        tx.send(EventLine::new(&json!({"event": "tick", "data": i})).unwrap())
                            .unwrap();
                    }
                    drop(tx);
                    replier.send(Reply::Stream {
                        first: Response::ok(request.id, json!({})),
                        events: rx,
                    });
                }
                _ => replier.respond(Response::err(request.id, "unknown_method", request.method)),
            }
        })
        .unwrap();

        assert_eq!(call(&addr, "ping", Value::Null).unwrap(), json!({"pong": true}));
        assert_eq!(call(&addr, "echo", json!({"a": [1, 2]})).unwrap(), json!({"a": [1, 2]}));
        let err = call(&addr, "nope", Value::Null).unwrap_err();
        assert!(
            matches!(err, ClientError::Remote { ref code, .. } if code == "unknown_method"),
            "{err}"
        );

        let events: Vec<Value> = subscribe(&addr, Value::Null).unwrap().collect();
        assert_eq!(events.len(), 3);
        assert_eq!(events[2]["data"], 2);
    }

    #[test]
    fn connecting_to_nothing_fails_cleanly() {
        let err = call(&new_address(), "ping", Value::Null).unwrap_err();
        assert!(matches!(err, ClientError::Connect { .. }), "{err}");
    }
}
