//! Synchronous XMLHttpRequest implementation for rquickjs
//!
//! Since boa_engine/rquickjs has no async/Promise support, all XHR operations are synchronous.
//! The HttpClient is stored in a thread-local and retrieved by native functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use rquickjs::{Function, Value};
use rquickjs::function::Rest;

use mb_network::client::HttpClient;
use mb_network::request::{HttpRequest, Method};
use tokio::runtime::Handle;

// Thread-local storage for the HTTP client and runtime handle
thread_local! {
    static XHR_CLIENT: RefCell<Option<Arc<HttpClient>>> = RefCell::new(None);
    static XHR_RUNTIME: RefCell<Option<Handle>> = RefCell::new(None);
    static XHR_INSTANCES: RefCell<HashMap<u64, XhrState>> = RefCell::new(HashMap::new());
    static XHR_NEXT_ID: RefCell<u64> = RefCell::new(0);
}

/// State of a single XMLHttpRequest instance
struct XhrState {
    method: String,
    url: String,
    request_headers: HashMap<String, String>,
    body: Vec<u8>,
    ready_state: u8,
    status: u16,
    status_text: String,
    response_text: String,
    response_url: String,
    response_headers: String,
}

impl Default for XhrState {
    fn default() -> Self {
        Self {
            method: String::new(),
            url: String::new(),
            request_headers: HashMap::new(),
            body: Vec::new(),
            ready_state: 0,
            status: 0,
            status_text: String::new(),
            response_text: String::new(),
            response_url: String::new(),
            response_headers: String::new(),
        }
    }
}

fn next_xhr_id() -> u64 {
    XHR_NEXT_ID.with(|cell| {
        let mut id = cell.borrow_mut();
        let current = *id;
        *id += 1;
        current
    })
}

/// Helper to extract f64 from Option<Value> (handles both Int and Float)
fn val_to_f64(v: Option<&Value>) -> Option<f64> {
    v.and_then(|v| v.as_float().or_else(|| v.as_int().map(|i| i as f64)))
}

/// Helper to extract string from Option<Value>
fn val_to_string(v: Option<&Value>) -> Option<String> {
    v.and_then(|v| {
        if v.is_string() {
            v.as_string().and_then(|s| s.to_string().ok())
        } else {
            None
        }
    })
}

/// Clear all XHR instance state (call after navigation)
pub fn clear_xhr_instances() {
    XHR_INSTANCES.with(|inst| {
        inst.borrow_mut().clear();
    });
}

/// Register XMLHttpRequest support in the JS context
pub fn register_xhr(ctx: &rquickjs::Context, http_client: Arc<HttpClient>, runtime_handle: Handle) -> Result<()> {
    XHR_CLIENT.with(|c| { *c.borrow_mut() = Some(http_client); });
    XHR_RUNTIME.with(|r| { *r.borrow_mut() = Some(runtime_handle); });

    ctx.with(|ctx| -> rquickjs::Result<()> {
        let globals = ctx.globals();

        // _xhr_create() -> id (f64)
        let _xhr_create = Function::new(ctx.clone(), |_args: Rest<Value>| -> rquickjs::Result<f64> {
            let id = next_xhr_id();
            XHR_INSTANCES.with(|inst| {
                inst.borrow_mut().insert(id, XhrState::default());
            });
            Ok(id as f64)
        })?;
        globals.set("_xhr_create", _xhr_create)?;

        // _xhr_open(id, method, url, async?) -> ()
        // Silently accepts async=true; all requests are executed synchronously.
        let _xhr_open = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<()> {
            let id = val_to_f64(args.get(0))
                .ok_or(rquickjs::Error::Exception)? as u64;
            let method = val_to_string(args.get(1))
                .ok_or(rquickjs::Error::Exception)?;
            let url = val_to_string(args.get(2))
                .ok_or(rquickjs::Error::Exception)?;

            // async flag (args.get(3)) is intentionally ignored —
            // we accept both sync and async XHR, executing all synchronously.

            XHR_INSTANCES.with(|inst| {
                if let Some(s) = inst.borrow_mut().get_mut(&id) {
                    s.method = method;
                    s.url = url;
                    s.ready_state = 1;
                }
            });
            Ok(())
        })?;
        globals.set("_xhr_open", _xhr_open)?;

        // _xhr_set_header(id, name, value) -> ()
        let _xhr_set_header = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<()> {
            let id = val_to_f64(args.get(0))
                .ok_or(rquickjs::Error::Exception)? as u64;
            let name = val_to_string(args.get(1))
                .ok_or(rquickjs::Error::Exception)?;
            let value = val_to_string(args.get(2))
                .ok_or(rquickjs::Error::Exception)?;

            XHR_INSTANCES.with(|inst| {
                if let Some(s) = inst.borrow_mut().get_mut(&id) {
                    s.request_headers.insert(name, value);
                }
            });
            Ok(())
        })?;
        globals.set("_xhr_set_header", _xhr_set_header)?;

        // _xhr_send(id, body) -> bool
        let _xhr_send = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<bool> {
            let id = val_to_f64(args.get(0))
                .ok_or(rquickjs::Error::Exception)? as u64;
            let body = val_to_string(args.get(1))
                .map(|s| s.into_bytes())
                .unwrap_or_default();

            // Extract request info
            let (method, url, headers) = XHR_INSTANCES.with(|inst| {
                let mut map = inst.borrow_mut();
                if let Some(s) = map.get_mut(&id) {
                    s.body = body;
                    s.ready_state = 2;
                    (s.method.clone(), s.url.clone(), s.request_headers.clone())
                } else {
                    (String::new(), String::new(), HashMap::new())
                }
            });

            if method.is_empty() || url.is_empty() {
                return Ok(false);
            }

            let http_method = match method.to_uppercase().as_str() {
                "GET" => Method::Get,
                "POST" => Method::Post,
                "PUT" => Method::Put,
                "DELETE" => Method::Delete,
                "HEAD" => Method::Head,
                "OPTIONS" => Method::Options,
                "PATCH" => Method::Patch,
                _ => Method::Get,
            };

            let mut request = HttpRequest::new(http_method, &url);
            for (k, v) in &headers {
                request = request.header(k.as_str(), v.as_str());
            }

            let body_bytes = XHR_INSTANCES.with(|inst| {
                inst.borrow().get(&id).map(|s| s.body.clone()).unwrap_or_default()
            });
            if !body_bytes.is_empty() {
                request = request.body(body_bytes);
            }

            // Execute request using block_in_place to allow blocking inside a tokio runtime
            let result = XHR_RUNTIME.with(|r| {
                let handle_opt = r.borrow();
                if let Some(handle) = handle_opt.as_ref() {
                    XHR_CLIENT.with(|c| {
                        let client_opt = c.borrow();
                        if let Some(client) = client_opt.as_ref() {
                            let client = Arc::clone(client);
                            tokio::task::block_in_place(|| {
                                handle.block_on(async move { client.execute(request).await })
                            })
                        } else {
                            Err(anyhow!("HTTP client not initialized"))
                        }
                    })
                } else {
                    Err(anyhow!("Tokio runtime handle not available"))
                }
            });

            match result {
                Ok(response) => {
                    let status = response.status_code();
                    let text = response.text().unwrap_or_default();
                    let resp_url = response.url.clone();
                    let mut hdrs = String::new();
                    for (name, value) in response.headers.iter() {
                        hdrs.push_str(&format!("{}: {}\r\n", name, value.to_str().unwrap_or("")));
                    }
                    XHR_INSTANCES.with(|inst| {
                        if let Some(s) = inst.borrow_mut().get_mut(&id) {
                            s.status = status;
                            s.status_text = format!("{}", status);
                            s.response_text = text;
                            s.response_url = resp_url;
                            s.response_headers = hdrs;
                            s.ready_state = 4;
                        }
                    });
                    Ok(true)
                }
                Err(_e) => {
                    XHR_INSTANCES.with(|inst| {
                        if let Some(s) = inst.borrow_mut().get_mut(&id) {
                            s.status = 0;
                            s.status_text = "Error".to_string();
                            s.response_text = format!("{}", _e);
                            s.ready_state = 4;
                        }
                    });
                    Ok(false)
                }
            }
        })?;
        globals.set("_xhr_send", _xhr_send)?;

        // _xhr_abort(id) -> ()
        let _xhr_abort = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<()> {
            let id = val_to_f64(args.get(0))
                .ok_or(rquickjs::Error::Exception)? as u64;
            XHR_INSTANCES.with(|inst| {
                inst.borrow_mut().remove(&id);
            });
            Ok(())
        })?;
        globals.set("_xhr_abort", _xhr_abort)?;

        // _xhr_get_property(id, prop_name) -> String
        let _xhr_get_property = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<String> {
            let id = val_to_f64(args.get(0))
                .ok_or(rquickjs::Error::Exception)? as u64;
            let prop = val_to_string(args.get(1))
                .ok_or(rquickjs::Error::Exception)?;

            let result = XHR_INSTANCES.with(|inst| {
                inst.borrow().get(&id).map(|s| {
                    match prop.as_str() {
                        "readyState" => s.ready_state.to_string(),
                        "status" => s.status.to_string(),
                        "statusText" => s.status_text.clone(),
                        "responseText" => s.response_text.clone(),
                        "responseURL" => s.response_url.clone(),
                        "response" => s.response_text.clone(),
                        _ => String::new(),
                    }
                }).unwrap_or_default()
            });
            Ok(result)
        })?;
        globals.set("_xhr_get_property", _xhr_get_property)?;

        // _xhr_get_response_header(id, name) -> String
        let _xhr_get_response_header = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<String> {
            let id = val_to_f64(args.get(0))
                .ok_or(rquickjs::Error::Exception)? as u64;
            let name = val_to_string(args.get(1))
                .ok_or(rquickjs::Error::Exception)?;

            let result = XHR_INSTANCES.with(|inst| {
                inst.borrow().get(&id).map(|s| {
                    for line in s.response_headers.lines() {
                        if let Some((k, v)) = line.split_once(':') {
                            if k.trim().eq_ignore_ascii_case(&name) {
                                return v.trim().to_string();
                            }
                        }
                    }
                    String::new()
                }).unwrap_or_default()
            });
            Ok(result)
        })?;
        globals.set("_xhr_get_response_header", _xhr_get_response_header)?;

        // _xhr_get_all_response_headers(id) -> String
        let _xhr_get_all_response_headers = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<String> {
            let id = val_to_f64(args.get(0))
                .ok_or(rquickjs::Error::Exception)? as u64;

            let result = XHR_INSTANCES.with(|inst| {
                inst.borrow().get(&id).map(|s| s.response_headers.clone()).unwrap_or_default()
            });
            Ok(result)
        })?;
        globals.set("_xhr_get_all_response_headers", _xhr_get_all_response_headers)?;

        // _native_fetch(method, url, body) -> {ok, status, statusText, body, headers}
        let _native_fetch = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<String> {
            let method = val_to_string(args.get(0)).unwrap_or_else(|| "GET".to_string());
            let url = val_to_string(args.get(1)).unwrap_or_default();
            let body_str = val_to_string(args.get(2)).unwrap_or_default();

            let http_method = match method.to_uppercase().as_str() {
                "POST" => Method::Post,
                "PUT" => Method::Put,
                "DELETE" => Method::Delete,
                "HEAD" => Method::Head,
                "OPTIONS" => Method::Options,
                "PATCH" => Method::Patch,
                _ => Method::Get,
            };

            let mut request = HttpRequest::new(http_method, &url);
            if !body_str.is_empty() {
                request = request.body(body_str.into_bytes());
            }

            let result = XHR_RUNTIME.with(|r| {
                let handle_opt = r.borrow();
                if let Some(handle) = handle_opt.as_ref() {
                    XHR_CLIENT.with(|c| {
                        let client_opt = c.borrow();
                        if let Some(client) = client_opt.as_ref() {
                            let client = Arc::clone(client);
                            tokio::task::block_in_place(|| {
                                handle.block_on(async move { client.execute(request).await })
                            })
                        } else {
                            Err(anyhow!("HTTP client not initialized"))
                        }
                    })
                } else {
                    Err(anyhow!("Tokio runtime handle not available"))
                }
            });

            match result {
                Ok(response) => {
                    let status = response.status_code();
                    let status_text = response.status.canonical_reason().unwrap_or("Unknown").to_string();
                    let text = response.text().unwrap_or_default();
                    // Build headers JSON object
                    let mut hdrs = Vec::new();
                    for (name, value) in response.headers.iter() {
                        let k = serde_json::to_string(name.as_str()).unwrap_or_default();
                        let v = serde_json::to_string(value.to_str().unwrap_or("")).unwrap_or_default();
                        hdrs.push(format!("{}:{}", k, v));
                    }
                    let headers_json = format!("{{{}}}", hdrs.join(","));
                    let body_json = serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".to_string());
                    Ok(format!(r#"{{"ok":true,"status":{},"statusText":"{}","body":{},"headers":{}}}"#,
                        status, status_text, body_json, headers_json
                    ))
                }
                Err(e) => {
                    Ok(format!(r#"{{"ok":false,"status":0,"statusText":"{}","body":"","headers":{{}}}}"#,
                        e.to_string().replace('"', "'")))
                }
            }
        })?;
        globals.set("_native_fetch", _native_fetch)?;

        // Inject XMLHttpRequest class via eval
        let _: () = ctx.eval(r#"
        function XMLHttpRequest() {
            this._id = _xhr_create();
            this.readyState = 0;
            this.status = 0;
            this.statusText = '';
            this.responseText = '';
            this.responseURL = '';
            this.response = '';
            this.onload = null;
            this.onerror = null;
            this.onreadystatechange = null;
        }
        XMLHttpRequest.prototype.open = function(method, url, async) {
            _xhr_open(this._id, method, url, async);
            this.readyState = 1;
        };
        XMLHttpRequest.prototype.send = function(body) {
            var ok = _xhr_send(this._id, body || '');
            // Always read response properties from native side,
            // whether the request succeeded or failed.
            // On error, _xhr_send still sets readyState=4, status=0, etc.
            this.readyState = parseInt(_xhr_get_property(this._id, 'readyState'));
            this.status = parseInt(_xhr_get_property(this._id, 'status'));
            this.statusText = _xhr_get_property(this._id, 'statusText');
            this.responseText = _xhr_get_property(this._id, 'responseText');
            this.responseURL = _xhr_get_property(this._id, 'responseURL');
            this.response = this.responseText;
            if (this.onreadystatechange) this.onreadystatechange();
            if (ok && this.onload) this.onload();
            if (!ok && this.onerror) this.onerror();
        };
        XMLHttpRequest.prototype.abort = function() { _xhr_abort(this._id); };
        XMLHttpRequest.prototype.setRequestHeader = function(name, value) { _xhr_set_header(this._id, name, value); };
        XMLHttpRequest.prototype.getResponseHeader = function(name) { return _xhr_get_response_header(this._id, name); };
        XMLHttpRequest.prototype.getAllResponseHeaders = function() { return _xhr_get_all_response_headers(this._id); };
        globalThis.XMLHttpRequest = XMLHttpRequest;

        // fetch() — synchronous implementation wrapped in Promise
        // _native_fetch(method, url, body) returns {ok, status, statusText, body, headers}
        globalThis.fetch = function(url, options) {
            var method = (options && options.method) || 'GET';
            var body = (options && options.body) || '';
            var raw = _native_fetch(method, url, body);
            var result = JSON.parse(raw);
            return new Promise(function(resolve, reject) {
                if (result.ok) {
                    resolve({
                        ok: true,
                        status: result.status,
                        statusText: result.statusText,
                        url: url,
                        _body: result.body,
                        _headers: result.headers,
                        json: function() { return JSON.parse(this._body); },
                        text: function() { return this._body; },
                        blob: function() { return this._body; },
                        arrayBuffer: function() { return this._body; },
                        headers: {
                            get: function(name) {
                                var h = this._headers || {};
                                return h[name.toLowerCase()] || null;
                            },
                            has: function(name) {
                                var h = this._headers || {};
                                return h.hasOwnProperty(name.toLowerCase());
                            }
                        }
                    });
                } else {
                    reject(new Error('fetch failed: ' + result.statusText));
                }
            });
        };
        "#)?;

        Ok(())
    }).map_err(|e| anyhow!("Failed to register XHR: {:?}", e))
}
