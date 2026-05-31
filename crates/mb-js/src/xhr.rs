//! Synchronous XMLHttpRequest implementation for boa_engine
//!
//! Since boa_engine has no async/Promise support, all XHR operations are synchronous.
//! The HttpClient is stored in a thread-local and retrieved by native functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use boa_engine::{
    Context, JsValue, JsString, NativeFunction, Source,
};

use mb_network::client::HttpClient;
use mb_network::request::{HttpRequest, Method};
use tokio::runtime::Handle;

/// Thread-local storage for the HTTP client and runtime handle
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

fn xhr_error(msg: &str) -> boa_engine::JsError {
    boa_engine::JsError::from_opaque(JsValue::from(JsString::from(msg)))
}

/// Register XMLHttpRequest support in the JS context
pub fn register_xhr(context: &mut Context, http_client: Arc<HttpClient>, runtime_handle: Handle) -> Result<()> {
    XHR_CLIENT.with(|c| { *c.borrow_mut() = Some(http_client); });
    XHR_RUNTIME.with(|r| { *r.borrow_mut() = Some(runtime_handle); });

    // _xhr_create() -> id
    context.register_global_callable(
        JsString::from("_xhr_create"),
        0,
        NativeFunction::from_copy_closure(
            |_this: &JsValue, _args: &[JsValue], _ctx: &mut Context| -> boa_engine::JsResult<JsValue> {
                let id = next_xhr_id();
                XHR_INSTANCES.with(|inst| {
                    inst.borrow_mut().insert(id, XhrState::default());
                });
                Ok(JsValue::from(id as f64))
            },
        ),
    ).map_err(|e| anyhow!("_xhr_create: {:?}", e))?;

    // _xhr_open(id, method, url)
    context.register_global_callable(
        JsString::from("_xhr_open"),
        3,
        NativeFunction::from_copy_closure(
            |_this: &JsValue, args: &[JsValue], _ctx: &mut Context| -> boa_engine::JsResult<JsValue> {
                let id = args.get(0).and_then(|v| v.as_number())
                    .ok_or_else(|| xhr_error("xhr_open: invalid id"))? as u64;
                let method = args.get(1).and_then(|v| v.as_string())
                    .ok_or_else(|| xhr_error("xhr_open: invalid method"))?
                    .to_std_string_escaped();
                let url = args.get(2).and_then(|v| v.as_string())
                    .ok_or_else(|| xhr_error("xhr_open: invalid url"))?
                    .to_std_string_escaped();

                // Check async flag
                if let Some(async_val) = args.get(3) {
                    if async_val.as_boolean().unwrap_or(true) {
                        return Err(xhr_error("XMLHttpRequest: async=true is not supported, only synchronous requests"));
                    }
                }

                XHR_INSTANCES.with(|inst| {
                    if let Some(s) = inst.borrow_mut().get_mut(&id) {
                        s.method = method;
                        s.url = url;
                        s.ready_state = 1;
                    }
                });
                Ok(JsValue::undefined())
            },
        ),
    ).map_err(|e| anyhow!("_xhr_open: {:?}", e))?;

    // _xhr_set_header(id, name, value)
    context.register_global_callable(
        JsString::from("_xhr_set_header"),
        3,
        NativeFunction::from_copy_closure(
            |_this: &JsValue, args: &[JsValue], _ctx: &mut Context| -> boa_engine::JsResult<JsValue> {
                let id = args.get(0).and_then(|v| v.as_number())
                    .ok_or_else(|| xhr_error("xhr_set_header: invalid id"))? as u64;
                let name = args.get(1).and_then(|v| v.as_string())
                    .ok_or_else(|| xhr_error("xhr_set_header: invalid name"))?
                    .to_std_string_escaped();
                let value = args.get(2).and_then(|v| v.as_string())
                    .ok_or_else(|| xhr_error("xhr_set_header: invalid value"))?
                    .to_std_string_escaped();

                XHR_INSTANCES.with(|inst| {
                    if let Some(s) = inst.borrow_mut().get_mut(&id) {
                        s.request_headers.insert(name, value);
                    }
                });
                Ok(JsValue::undefined())
            },
        ),
    ).map_err(|e| anyhow!("_xhr_set_header: {:?}", e))?;

    // _xhr_send(id, body) -> boolean
    context.register_global_callable(
        JsString::from("_xhr_send"),
        2,
        NativeFunction::from_copy_closure(
            |_this: &JsValue, args: &[JsValue], _ctx: &mut Context| -> boa_engine::JsResult<JsValue> {
                let id = args.get(0).and_then(|v| v.as_number())
                    .ok_or_else(|| xhr_error("xhr_send: invalid id"))? as u64;
                let body = args.get(1).and_then(|v| v.as_string())
                    .map(|s| s.to_std_string_escaped().into_bytes())
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
                    return Ok(JsValue::from(false));
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
                        Ok(JsValue::from(true))
                    }
                    Err(e) => {
                        XHR_INSTANCES.with(|inst| {
                            if let Some(s) = inst.borrow_mut().get_mut(&id) {
                                s.status = 0;
                                s.status_text = "Error".to_string();
                                s.response_text = format!("{}", e);
                                s.ready_state = 4;
                            }
                        });
                        Ok(JsValue::from(false))
                    }
                }
            },
        ),
    ).map_err(|e| anyhow!("_xhr_send: {:?}", e))?;

    // _xhr_abort(id)
    context.register_global_callable(
        JsString::from("_xhr_abort"),
        1,
        NativeFunction::from_copy_closure(
            |_this: &JsValue, args: &[JsValue], _ctx: &mut Context| -> boa_engine::JsResult<JsValue> {
                let id = args.get(0).and_then(|v| v.as_number())
                    .ok_or_else(|| xhr_error("xhr_abort: invalid id"))? as u64;
                XHR_INSTANCES.with(|inst| {
                    if let Some(s) = inst.borrow_mut().get_mut(&id) {
                        s.ready_state = 0;
                    }
                });
                Ok(JsValue::undefined())
            },
        ),
    ).map_err(|e| anyhow!("_xhr_abort: {:?}", e))?;

    // _xhr_get_property(id, name) -> value
    context.register_global_callable(
        JsString::from("_xhr_get_property"),
        2,
        NativeFunction::from_copy_closure(
            |_this: &JsValue, args: &[JsValue], _ctx: &mut Context| -> boa_engine::JsResult<JsValue> {
                let id = args.get(0).and_then(|v| v.as_number())
                    .ok_or_else(|| xhr_error("xhr_get_property: invalid id"))? as u64;
                let prop = args.get(1).and_then(|v| v.as_string())
                    .ok_or_else(|| xhr_error("xhr_get_property: invalid prop"))?
                    .to_std_string_escaped();

                XHR_INSTANCES.with(|inst| {
                    let map = inst.borrow();
                    match map.get(&id) {
                        Some(s) => match prop.as_str() {
                            "readyState" => Ok(JsValue::from(s.ready_state as f64)),
                            "status" => Ok(JsValue::from(s.status as f64)),
                            "statusText" => Ok(JsValue::from(JsString::from(s.status_text.as_str()))),
                            "responseText" => Ok(JsValue::from(JsString::from(s.response_text.as_str()))),
                            "responseURL" => Ok(JsValue::from(JsString::from(s.response_url.as_str()))),
                            "response" => Ok(JsValue::from(JsString::from(s.response_text.as_str()))),
                            _ => Ok(JsValue::undefined()),
                        },
                        None => Ok(JsValue::undefined()),
                    }
                })
            },
        ),
    ).map_err(|e| anyhow!("_xhr_get_property: {:?}", e))?;

    // _xhr_get_response_header(id, name) -> string|null
    context.register_global_callable(
        JsString::from("_xhr_get_response_header"),
        2,
        NativeFunction::from_copy_closure(
            |_this: &JsValue, args: &[JsValue], _ctx: &mut Context| -> boa_engine::JsResult<JsValue> {
                let id = args.get(0).and_then(|v| v.as_number())
                    .ok_or_else(|| xhr_error("xhr_get_response_header: invalid id"))? as u64;
                let name = args.get(1).and_then(|v| v.as_string())
                    .ok_or_else(|| xhr_error("xhr_get_response_header: invalid name"))?
                    .to_std_string_escaped();

                XHR_INSTANCES.with(|inst| {
                    let map = inst.borrow();
                    match map.get(&id) {
                        Some(s) => {
                            for line in s.response_headers.lines() {
                                if let Some((k, v)) = line.split_once(':') {
                                    if k.trim().eq_ignore_ascii_case(&name) {
                                        return Ok(JsValue::from(JsString::from(v.trim())));
                                    }
                                }
                            }
                            Ok(JsValue::null())
                        }
                        None => Ok(JsValue::null()),
                    }
                })
            },
        ),
    ).map_err(|e| anyhow!("_xhr_get_response_header: {:?}", e))?;

    // _xhr_get_all_response_headers(id) -> string
    context.register_global_callable(
        JsString::from("_xhr_get_all_response_headers"),
        1,
        NativeFunction::from_copy_closure(
            |_this: &JsValue, args: &[JsValue], _ctx: &mut Context| -> boa_engine::JsResult<JsValue> {
                let id = args.get(0).and_then(|v| v.as_number())
                    .ok_or_else(|| xhr_error("xhr_get_all_response_headers: invalid id"))? as u64;

                XHR_INSTANCES.with(|inst| {
                    let map = inst.borrow();
                    match map.get(&id) {
                        Some(s) => Ok(JsValue::from(JsString::from(s.response_headers.as_str()))),
                        None => Ok(JsValue::from(JsString::from(""))),
                    }
                })
            },
        ),
    ).map_err(|e| anyhow!("_xhr_get_all_response_headers: {:?}", e))?;

    // Define XMLHttpRequest as a JS constructor
    let xhr_class_js = r#"
    var XMLHttpRequest = function() {
        this._id = _xhr_create();
        this._listeners = {};
    };

    XMLHttpRequest.UNSENT = 0;
    XMLHttpRequest.OPENED = 1;
    XMLHttpRequest.HEADERS_RECEIVED = 2;
    XMLHttpRequest.LOADING = 3;
    XMLHttpRequest.DONE = 4;

    XMLHttpRequest.prototype.open = function(method, url, async) {
        if (async !== undefined && async !== false && async !== null) {
            throw new Error('XMLHttpRequest: async=true is not supported, only synchronous requests');
        }
        _xhr_open(this._id, method, url, false);
    };

    XMLHttpRequest.prototype.send = function(body) {
        var bodyStr = (body === undefined || body === null) ? '' : String(body);
        var success = _xhr_send(this._id, bodyStr);
        if (!success) {
            if (typeof this.onerror === 'function') this.onerror();
        } else {
            if (typeof this.onload === 'function') this.onload();
        }
    };

    XMLHttpRequest.prototype.abort = function() {
        _xhr_abort(this._id);
    };

    XMLHttpRequest.prototype.setRequestHeader = function(name, value) {
        _xhr_set_header(this._id, name, value);
    };

    XMLHttpRequest.prototype.getResponseHeader = function(name) {
        return _xhr_get_response_header(this._id, name);
    };

    XMLHttpRequest.prototype.getAllResponseHeaders = function() {
        return _xhr_get_all_response_headers(this._id);
    };

    Object.defineProperty(XMLHttpRequest.prototype, 'readyState', {
        get: function() { return _xhr_get_property(this._id, 'readyState'); }
    });
    Object.defineProperty(XMLHttpRequest.prototype, 'status', {
        get: function() { return _xhr_get_property(this._id, 'status'); }
    });
    Object.defineProperty(XMLHttpRequest.prototype, 'statusText', {
        get: function() { return _xhr_get_property(this._id, 'statusText'); }
    });
    Object.defineProperty(XMLHttpRequest.prototype, 'responseText', {
        get: function() { return _xhr_get_property(this._id, 'responseText'); }
    });
    Object.defineProperty(XMLHttpRequest.prototype, 'responseURL', {
        get: function() { return _xhr_get_property(this._id, 'responseURL'); }
    });
    Object.defineProperty(XMLHttpRequest.prototype, 'response', {
        get: function() { return _xhr_get_property(this._id, 'response'); }
    });
    "#;

    context.eval(Source::from_bytes(xhr_class_js.as_bytes()))
        .map_err(|e| anyhow!("Failed to define XMLHttpRequest class: {:?}", e))?;

    Ok(())
}
