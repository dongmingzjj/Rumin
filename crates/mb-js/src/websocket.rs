//! WebSocket implementation using tokio-tungstenite for real network connections.
//!
//! Uses a thread-local registry of WebSocket connections, with native functions
//! called from JS to create, send, close, and poll connections.
//!
//! SSRF protection: connections to private/internal IP addresses are rejected.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::IpAddr;
use anyhow::{anyhow, Result};
use rquickjs::function::Rest;
use rquickjs::{Function, Value};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

/// Check if an IP address string belongs to a private/internal network range.
fn is_private_ip(ip_str: &str) -> bool {
    let ip_str = ip_str.trim();
    let stripped;
    let ip_str = if ip_str.starts_with('[') && ip_str.ends_with(']') {
        stripped = &ip_str[1..ip_str.len() - 1];
        stripped
    } else {
        ip_str
    };
    let ip: IpAddr = match ip_str.parse() {
        Ok(ip) => ip,
        Err(_) => return false,
    };
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || {
                    let octets = v6.octets();
                    (octets[0] & 0xfe) == 0xfc
                }
                || v6.is_unicast_link_local()
        }
    }
}

/// Check if a URL targets a private/internal IP address.
fn url_targets_private_ip(url_str: &str) -> bool {
    if let Ok(parsed) = url::Url::parse(url_str) {
        if let Some(host) = parsed.host_str() {
            return is_private_ip(host);
        }
    }
    false
}

/// State of a single WebSocket connection
struct WsState {
    #[allow(dead_code)]
    url: String,
    ready_state: u16, // 0=CONNECTING, 1=OPEN, 2=CLOSING, 3=CLOSED
    /// Channel to send text messages to the WS task
    tx: Option<mpsc::UnboundedSender<String>>,
    /// Channel to receive messages from the WS task
    rx: Option<mpsc::UnboundedReceiver<String>>,
    /// Buffered messages from polling
    buffered_messages: Vec<String>,
    /// Whether we received a close signal
    closed: bool,
    close_code: u16,
    close_reason: String,
    error: Option<String>,
}

// Thread-local storage for WS instances and runtime handle
thread_local! {
    static WS_RUNTIME: RefCell<Option<Handle>> = RefCell::new(None);
    static WS_INSTANCES: RefCell<HashMap<u64, WsState>> = RefCell::new(HashMap::new());
    static WS_NEXT_ID: RefCell<u64> = RefCell::new(1);
}

fn next_ws_id() -> u64 {
    WS_NEXT_ID.with(|cell| {
        let mut id = cell.borrow_mut();
        let current = *id;
        *id += 1;
        current
    })
}

/// Register WebSocket support in the JS context.
/// The tokio runtime handle is stored for spawning connection tasks.
pub fn register_websocket(ctx: &rquickjs::Context, runtime_handle: Handle) -> Result<()> {
    WS_RUNTIME.with(|r| {
        *r.borrow_mut() = Some(runtime_handle);
    });

    ctx.with(|ctx| -> rquickjs::Result<()> {
        let globals = ctx.globals();

        // _ws_create(url) -> id (f64), returns -1 on SSRF block
        let _ws_create = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<f64> {
            let url = args
                .get(0)
                .and_then(|v| {
                    if v.is_string() {
                        v.as_string().and_then(|s| s.to_string().ok())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            if url.is_empty() {
                return Ok(-1.0);
            }

            // SSRF protection
            if url_targets_private_ip(&url) {
                return Ok(-1.0);
            }

            let id = next_ws_id();

            // Create channels for communication between JS and the async WS task
            let (tx_to_ws, rx_from_js) = mpsc::unbounded_channel::<String>();
            let (tx_from_ws, rx_to_js) = mpsc::unbounded_channel::<String>();

            WS_INSTANCES.with(|inst| {
                inst.borrow_mut().insert(
                    id,
                    WsState {
                        url: url.clone(),
                        ready_state: 0, // CONNECTING
                        tx: Some(tx_to_ws),
                        rx: Some(rx_to_js),
                        buffered_messages: Vec::new(),
                        closed: false,
                        close_code: 0,
                        close_reason: String::new(),
                        error: None,
                    },
                );
            });

            // Spawn async task to connect
            let url_clone = url.clone();
            WS_RUNTIME.with(|r| {
                let handle_opt = r.borrow();
                if let Some(handle) = handle_opt.as_ref() {
                    let tx_out = tx_from_ws;
                    let id_clone = id;
                    handle.spawn(async move {
                        match connect_and_run(&url_clone, rx_from_js, &tx_out).await {
                            Ok(()) => {}
                            Err(e) => {
                                let _ = tx_out.send(format!("__error__:{}", e));
                            }
                        }
                        // Send close signal
                        let _ = tx_out.send("__closed__".to_string());
                        // Update ready state
                        WS_INSTANCES.with(|inst| {
                            if let Some(s) = inst.borrow_mut().get_mut(&id_clone) {
                                s.ready_state = 3; // CLOSED
                                s.closed = true;
                            }
                        });
                    });
                }
            });

            Ok(id as f64)
        })?;
        globals.set("_ws_create", _ws_create)?;

        // _ws_send(id, data) -> bool
        let _ws_send = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<bool> {
            let id = args
                .get(0)
                .and_then(|v| v.as_float().or_else(|| v.as_int().map(|i| i as f64)))
                .unwrap_or(0.0) as u64;
            let data = args
                .get(1)
                .and_then(|v| {
                    if v.is_string() {
                        v.as_string().and_then(|s| s.to_string().ok())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            let result = WS_INSTANCES.with(|inst| {
                let map = inst.borrow();
                if let Some(s) = map.get(&id) {
                    if let Some(ref tx) = s.tx {
                        return tx.send(data).is_ok();
                    }
                }
                false
            });
            Ok(result)
        })?;
        globals.set("_ws_send", _ws_send)?;

        // _ws_close(id, code, reason) -> ()
        let _ws_close = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<()> {
            let id = args
                .get(0)
                .and_then(|v| v.as_float().or_else(|| v.as_int().map(|i| i as f64)))
                .unwrap_or(0.0) as u64;
            let code = args
                .get(1)
                .and_then(|v| v.as_float().or_else(|| v.as_int().map(|i| i as f64)))
                .unwrap_or(1000.0) as u16;
            let reason = args
                .get(2)
                .and_then(|v| {
                    if v.is_string() {
                        v.as_string().and_then(|s| s.to_string().ok())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            WS_INSTANCES.with(|inst| {
                let mut map = inst.borrow_mut();
                if let Some(s) = map.get_mut(&id) {
                    s.ready_state = 2; // CLOSING
                    s.close_code = code;
                    s.close_reason = reason;
                    // Drop the sender to signal close
                    s.tx.take();
                }
            });
            Ok(())
        })?;
        globals.set("_ws_close", _ws_close)?;

        // _ws_get_ready_state(id) -> f64
        let _ws_get_ready_state =
            Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<f64> {
                let id = args
                    .get(0)
                    .and_then(|v| v.as_float().or_else(|| v.as_int().map(|i| i as f64)))
                    .unwrap_or(0.0) as u64;

                let state = WS_INSTANCES.with(|inst| {
                    inst.borrow()
                        .get(&id)
                        .map(|s| s.ready_state as f64)
                        .unwrap_or(3.0) // CLOSED by default
                });
                Ok(state)
            })?;
        globals.set("_ws_get_ready_state", _ws_get_ready_state)?;

        // _ws_poll(id) -> JSON string: {messages: [...], error: null|str, closed: bool, code: num, reason: str}
        let _ws_poll = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<String> {
            let id = args
                .get(0)
                .and_then(|v| v.as_float().or_else(|| v.as_int().map(|i| i as f64)))
                .unwrap_or(0.0) as u64;

            let result = WS_INSTANCES.with(|inst| {
                let mut map = inst.borrow_mut();
                if let Some(s) = map.get_mut(&id) {
                    // Drain from the async receiver into buffered messages
                    if let Some(ref mut rx) = s.rx {
                        while let Ok(msg) = rx.try_recv() {
                            if msg == "__closed__" {
                                s.closed = true;
                                s.ready_state = 3;
                            } else if msg.starts_with("__error__:") {
                                s.error = Some(msg[10..].to_string());
                            } else {
                                s.buffered_messages.push(msg);
                            }
                        }
                    }

                    let msgs: Vec<String> = s.buffered_messages.drain(..).collect();
                    let err = s.error.clone();
                    let closed = s.closed;
                    let code = s.close_code;
                    let reason = s.close_reason.clone();

                    serde_json::json!({
                        "messages": msgs,
                        "error": err,
                        "closed": closed,
                        "code": code,
                        "reason": reason
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "messages": [],
                        "error": "Connection not found",
                        "closed": true,
                        "code": 1006,
                        "reason": ""
                    })
                    .to_string()
                }
            });
            Ok(result)
        })?;
        globals.set("_ws_poll", _ws_poll)?;

        // Inject WebSocket class via eval
        let _: () = ctx.eval(
            r#"
        (function() {
            var CONNECTING = 0;
            var OPEN = 1;
            var CLOSING = 2;
            var CLOSED = 3;

            function WebSocket(url, protocols) {
                var self = this;
                this.url = url;
                this.readyState = CONNECTING;
                this.bufferedAmount = 0;
                this.extensions = '';
                this.protocol = '';
                this.binaryType = 'blob';

                this.onopen = null;
                this.onmessage = null;
                this.onerror = null;
                this.onclose = null;

                this._id = _ws_create(url);
                if (this._id < 0) {
                    this.readyState = CLOSED;
                    var self2 = this;
                    setTimeout(function() {
                        if (typeof self2.onerror === 'function') {
                            self2.onerror({ type: 'error', target: self2 });
                        }
                        if (typeof self2.onclose === 'function') {
                            self2.onclose({ code: 1006, reason: 'SSRF blocked', wasClean: false, type: 'close', target: self2 });
                        }
                    }, 0);
                    return;
                }

                // Start polling at 50ms intervals
                var pollInterval = setInterval(function() {
                    var state = _ws_get_ready_state(self._id);
                    self.readyState = state;

                    if (state === CLOSED) {
                        clearInterval(pollInterval);
                        return;
                    }

                    var raw = _ws_poll(self._id);
                    var data;
                    try { data = JSON.parse(raw); } catch(e) { return; }

                    // Check for open
                    if (state === OPEN && self.readyState !== OPEN) {
                        self.readyState = OPEN;
                        if (typeof self.onopen === 'function') {
                            self.onopen({ type: 'open', target: self });
                        }
                    }

                    // Deliver messages
                    if (data.messages) {
                        for (var i = 0; i < data.messages.length; i++) {
                            if (typeof self.onmessage === 'function') {
                                self.onmessage({
                                    data: data.messages[i],
                                    type: 'message',
                                    target: self
                                });
                            }
                        }
                    }

                    // Check for errors
                    if (data.error) {
                        if (typeof self.onerror === 'function') {
                            self.onerror({ type: 'error', message: data.error, target: self });
                        }
                    }

                    // Check for close
                    if (data.closed) {
                        clearInterval(pollInterval);
                        self.readyState = CLOSED;
                        if (typeof self.onclose === 'function') {
                            self.onclose({
                                code: data.code || 1000,
                                reason: data.reason || '',
                                wasClean: data.code === 1000,
                                type: 'close',
                                target: self
                            });
                        }
                    }
                }, 50);

                // Trigger onopen when connection establishes (poll once after a tick)
                var openCheck = setInterval(function() {
                    var state = _ws_get_ready_state(self._id);
                    self.readyState = state;
                    if (state === OPEN) {
                        clearInterval(openCheck);
                        if (typeof self.onopen === 'function') {
                            self.onopen({ type: 'open', target: self });
                        }
                    } else if (state === CLOSED) {
                        clearInterval(openCheck);
                    }
                }, 50);

                this._pollInterval = pollInterval;
                this._openCheck = openCheck;
            }

            WebSocket.prototype.send = function(data) {
                if (this.readyState !== OPEN) {
                    throw new Error('InvalidStateError: WebSocket is not open');
                }
                _ws_send(this._id, String(data));
            };

            WebSocket.prototype.close = function(code, reason) {
                if (this.readyState === CLOSING || this.readyState === CLOSED) return;
                this.readyState = CLOSING;
                _ws_close(this._id, code || 1000, reason || '');
                if (this._pollInterval) clearInterval(this._pollInterval);
                if (this._openCheck) clearInterval(this._openCheck);
            };

            WebSocket.CONNECTING = CONNECTING;
            WebSocket.OPEN = OPEN;
            WebSocket.CLOSING = CLOSING;
            WebSocket.CLOSED = CLOSED;

            globalThis.WebSocket = WebSocket;
        })();
        "#,
        )?;

        Ok(())
    })
    .map_err(|e| anyhow!("Failed to setup WebSocket: {:?}", e))?;

    Ok(())
}

/// Connect to a WebSocket URL and run the message relay loop.
async fn connect_and_run(
    url: &str,
    mut rx: mpsc::UnboundedReceiver<String>,
    tx: &mpsc::UnboundedSender<String>,
) -> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    let (ws_stream, _response) = connect_async(url).await?;

    let (mut write, mut read) = ws_stream.split();

    // Notify that we're open
    // (the ready_state is updated in the polling function based on messages)

    // Spawn a task to forward incoming messages to the channel
    let tx_clone = tx.clone();
    let read_task = tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if tx_clone.send(text.to_string()).is_err() {
                        break;
                    }
                }
                Ok(Message::Binary(bin)) => {
                    if tx_clone
                        .send(String::from_utf8_lossy(&bin).to_string())
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Close(_)) => {
                    break;
                }
                Ok(Message::Ping(data)) => {
                    // Auto-pong is handled by tungstenite
                    let _ = data;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    // Forward outgoing messages from the channel to the WebSocket
    let write_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if write.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
        // Try to close gracefully
        let _ = write.close().await;
    });

    // Wait for either task to complete
    tokio::select! {
        _ = read_task => {}
        _ = write_task => {}
    }

    Ok(())
}
