//! Synchronous XMLHttpRequest implementation for rquickjs
//!
//! Since boa_engine/rquickjs has no async/Promise support, all XHR operations are synchronous.
//! The HttpClient is stored in a thread-local and retrieved by native functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use rquickjs::{Function, Value};
use rquickjs::function::Rest;

use mb_network::client::HttpClient;
use mb_network::request::{HttpRequest, Method};
use tokio::runtime::Handle;

/// Check if an IP address string belongs to a private/internal network range.
/// Only checks literal IP addresses (not hostnames/DNS names).
fn is_private_ip(ip_str: &str) -> bool {
    let ip_str = ip_str.trim();
    // Strip IPv6 brackets if present
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
            v4.is_loopback()                          // 127.0.0.0/8
                || v4.is_private()                     // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local()                  // 169.254.0.0/16
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()                           // ::1
                || {
                    let octets = v6.octets();
                    // fc00::/7 (unique local addresses)
                    (octets[0] & 0xfe) == 0xfc
                }
                || v6.is_unicast_link_local()          // fe80::/10
        }
    }
}

/// Check if a URL targets a private/internal IP address.
/// Returns true only if the host is a literal IP in a private range.
pub(crate) fn url_targets_private_ip(url_str: &str) -> bool {
    if let Ok(parsed) = url::Url::parse(url_str) {
        if let Some(host) = parsed.host_str() {
            return is_private_ip(host);
        }
    }
    false
}

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

            // SSRF protection: reject requests to private/internal IP addresses
            if url_targets_private_ip(&url) {
                XHR_INSTANCES.with(|inst| {
                    if let Some(s) = inst.borrow_mut().get_mut(&id) {
                        s.status = 0;
                        s.status_text = "Blocked".to_string();
                        s.response_text = "SSRF blocked: request to private IP address".to_string();
                        s.ready_state = 4;
                    }
                });
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
        let _native_fetch = Function::new(ctx.clone(), move |args: Rest<Value>| -> rquickjs::Result<String> {
            let method = val_to_string(args.get(0)).unwrap_or_else(|| "GET".to_string());
            let url = val_to_string(args.get(1)).unwrap_or_default();
            let body_str = val_to_string(args.get(2)).unwrap_or_default();
            let headers_json = val_to_string(args.get(3)).unwrap_or_default();

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
            // Parse and apply custom headers from JS
            if !headers_json.is_empty() {
                if let Ok(map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&headers_json) {
                    for (k, v) in &map {
                        if let Some(val) = v.as_str() {
                            request = request.header(k.as_str(), val);
                        }
                    }
                }
            }
            if !body_str.is_empty() {
                request = request.body(body_str.into_bytes());
            }

            // SSRF protection: reject requests to private/internal IP addresses
            if url_targets_private_ip(&url) {
                return Ok(serde_json::json!({
                    "ok": false,
                    "status": 0,
                    "statusText": "SSRF blocked: request to private IP address",
                    "body": "",
                    "headers": {}
                }).to_string());
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
                    // Build headers as a serde_json Map for proper JSON escaping
                    let mut hdrs = serde_json::Map::new();
                    for (name, value) in response.headers.iter() {
                        hdrs.insert(
                            name.as_str().to_string(),
                            serde_json::Value::String(value.to_str().unwrap_or("").to_string()),
                        );
                    }
                    Ok(serde_json::json!({
                        "ok": true,
                        "status": status,
                        "statusText": status_text,
                        "body": text,
                        "headers": hdrs
                    }).to_string())
                }
                Err(e) => {
                    Ok(serde_json::json!({
                        "ok": false,
                        "status": 0,
                        "statusText": e.to_string(),
                        "body": "",
                        "headers": {}
                    }).to_string())
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
            var hdrs = (options && options.headers) || null;
            if (hdrs && typeof hdrs.entries === 'function') {
                var obj = {};
                hdrs.entries(function(v, k) { obj[k] = v; });
                hdrs = obj;
            }
            var headersStr = hdrs ? JSON.stringify(hdrs) : '';
            var raw = _native_fetch(method, url, body, headersStr);
            var result = JSON.parse(raw);
            return new Promise(function(resolve, reject) {
                if (result.ok) {
                    var _rawHeaders = result.headers || {};
                    var response = {
                        ok: true,
                        status: result.status,
                        statusText: result.statusText,
                        url: url,
                        redirected: false,
                        type: 'basic',
                        bodyUsed: false,
                        _body: result.body,
                        _headers: _rawHeaders,
                        json: function() { this.bodyUsed = true; return JSON.parse(this._body); },
                        text: function() { this.bodyUsed = true; return this._body; },
                        blob: function() { this.bodyUsed = true; return this._body; },
                        arrayBuffer: function() { this.bodyUsed = true; return this._body; },
                        clone: function() {
                            var cloned = Object.assign({}, this);
                            cloned._body = this._body;
                            cloned._headers = Object.assign({}, this._headers);
                            cloned.bodyUsed = false;
                            return cloned;
                        },
                        headers: {
                            _headers: _rawHeaders,
                            get: function(name) {
                                var h = this._headers || {};
                                return h[name.toLowerCase()] || null;
                            },
                            has: function(name) {
                                var h = this._headers || {};
                                return h.hasOwnProperty(name.toLowerCase());
                            },
                            entries: function() {
                                var h = this._headers || {};
                                var result = [];
                                var keys = Object.keys(h);
                                for (var i = 0; i < keys.length; i++) {
                                    result.push([keys[i], h[keys[i]]]);
                                }
                                return result;
                            },
                            keys: function() {
                                return Object.keys(this._headers || {});
                            },
                            values: function() {
                                var h = this._headers || {};
                                var result = [];
                                var keys = Object.keys(h);
                                for (var i = 0; i < keys.length; i++) {
                                    result.push(h[keys[i]]);
                                }
                                return result;
                            },
                            forEach: function(fn) {
                                var h = this._headers || {};
                                var keys = Object.keys(h);
                                for (var i = 0; i < keys.length; i++) {
                                    fn(h[keys[i]], keys[i], this);
                                }
                            }
                        }
                    };
                    resolve(response);
                } else {
                    reject(new Error('fetch failed: ' + result.statusText));
                }
            });
        };
        "#)?;

        Ok(())
    }).map_err(|e| anyhow!("Failed to register XHR: {:?}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssrf_blocked_loopback() {
        assert!(url_targets_private_ip("http://127.0.0.1/"));
        assert!(url_targets_private_ip("http://127.0.0.1:8080/api"));
        assert!(url_targets_private_ip("http://[::1]:8080/"));
    }

    #[test]
    fn test_ssrf_blocked_private_ranges() {
        assert!(url_targets_private_ip("http://10.0.0.1/"));
        assert!(url_targets_private_ip("http://172.16.0.1/"));
        assert!(url_targets_private_ip("http://192.168.1.1/"));
        assert!(url_targets_private_ip("http://169.254.1.1/"));
    }

    #[test]
    fn test_ssrf_allowed_public() {
        assert!(!url_targets_private_ip("https://example.com/"));
        assert!(!url_targets_private_ip("http://8.8.8.8/"));
        assert!(!url_targets_private_ip("https://1.1.1.1/"));
    }
}
