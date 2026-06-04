//! Fetch API 实现
//!
//! 提供标准的 fetch() 函数，返回 Promise<Response>。
//! 支持 GET/POST/PUT/DELETE 等方法、Headers 对象、string/FormData/Blob body。
//! 使用同步 HTTP 调用模式（block_in_place + handle.block_on）并包装为 Promise。

use std::cell::RefCell;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use rquickjs::{Function, Value};
use rquickjs::function::Rest;

use mb_network::client::HttpClient;
use mb_network::request::{HttpRequest, Method};
use tokio::runtime::Handle;

use super::xhr;

// Thread-local 存储 HTTP 客户端和 tokio 运行时句柄（与 xhr.rs 相同模式）
thread_local! {
    static FETCH_CLIENT: RefCell<Option<Arc<HttpClient>>> = RefCell::new(None);
    static FETCH_RUNTIME: RefCell<Option<Handle>> = RefCell::new(None);
}

/// 辅助函数：从 Rest<Value> 中提取字符串
fn val_to_string(v: Option<&Value>) -> Option<String> {
    v.and_then(|v| {
        if v.is_string() {
            v.as_string().and_then(|s| s.to_string().ok())
        } else {
            None
        }
    })
}

/// 注册 fetch API 到 JS 上下文
///
/// 注册内容：
/// - _fetch_request(method, url, body, headers_json) — 原生 HTTP 请求函数
/// - globalThis.fetch(url, options) — 返回 Promise<Response>
/// - globalThis.Headers — 标准 Headers 构造函数
/// - globalThis.Response — 标准 Response 构造函数
pub fn register_fetch(
    ctx: &rquickjs::Context,
    http_client: Arc<HttpClient>,
    runtime_handle: Handle,
) -> Result<()> {
    // 存储 HTTP 客户端和运行时句柄到 thread-local
    FETCH_CLIENT.with(|c| {
        *c.borrow_mut() = Some(http_client);
    });
    FETCH_RUNTIME.with(|r| {
        *r.borrow_mut() = Some(runtime_handle);
    });

    ctx.with(|ctx| -> rquickjs::Result<()> {
        let globals = ctx.globals();

        // ---- 注册原生 Rust 函数 ----

        // _fetch_request(method, url, body_str, headers_json) -> JSON 字符串
        // 执行实际的 HTTP 请求，返回包含响应数据的 JSON 字符串
        let _fetch_request = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<String> {
            let method = val_to_string(args.get(0)).unwrap_or_else(|| "GET".to_string());
            let url = val_to_string(args.get(1)).unwrap_or_default();
            let body_str = val_to_string(args.get(2)).unwrap_or_default();
            let headers_json = val_to_string(args.get(3)).unwrap_or_default();

            // SSRF 防护：拒绝访问私有/内部 IP 地址
            if xhr::url_targets_private_ip(&url) {
                return Ok(serde_json::json!({
                    "ok": false,
                    "status": 0,
                    "statusText": "SSRF blocked: request to private IP address",
                    "body": "",
                    "headers": {},
                    "url": ""
                })
                .to_string());
            }

            // 映射 HTTP 方法
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

            // 解析并应用自定义请求头
            if !headers_json.is_empty() {
                if let Ok(map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&headers_json) {
                    for (k, v) in &map {
                        if let Some(val) = v.as_str() {
                            request = request.header(k.as_str(), val);
                        }
                    }
                }
            }

            // 设置请求体
            if !body_str.is_empty() {
                request = request.body(body_str.into_bytes());
            }

            // 使用 block_in_place 在 tokio 运行时中执行同步 HTTP 请求
            let result = FETCH_RUNTIME.with(|r| {
                let handle_opt = r.borrow();
                if let Some(handle) = handle_opt.as_ref() {
                    FETCH_CLIENT.with(|c| {
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
                    let status_text = response
                        .status
                        .canonical_reason()
                        .unwrap_or("Unknown")
                        .to_string();
                    let text = response.text().unwrap_or_default();
                    let resp_url = response.url.clone();

                    // 将响应头构建为 JSON Map
                    let mut hdrs = serde_json::Map::new();
                    for (name, value) in response.headers.iter() {
                        hdrs.insert(
                            name.as_str().to_string(),
                            serde_json::Value::String(value.to_str().unwrap_or("").to_string()),
                        );
                    }

                    Ok(serde_json::json!({
                        "ok": status >= 200 && status < 400,
                        "status": status,
                        "statusText": status_text,
                        "body": text,
                        "headers": hdrs,
                        "url": resp_url
                    })
                    .to_string())
                }
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "status": 0,
                    "statusText": e.to_string(),
                    "body": "",
                    "headers": {},
                    "url": ""
                })
                .to_string()),
            }
        })?;
        globals.set("_fetch_request", _fetch_request)?;

        // ---- 注入 JavaScript 层的 Headers / Response / fetch 实现 ----
        let _: () = ctx.eval(
            r#"
        // ==================== Headers 构造函数 ====================
        function Headers(init) {
            this._headers = {};
            if (init) {
                if (init instanceof Headers) {
                    var self = this;
                    init.forEach(function(v, k) { self._headers[k.toLowerCase()] = v; });
                } else if (typeof init === 'object') {
                    var keys = Object.keys(init);
                    for (var i = 0; i < keys.length; i++) {
                        this._headers[keys[i].toLowerCase()] = String(init[keys[i]]);
                    }
                }
            }
        }
        Headers.prototype.append = function(name, value) {
            var key = name.toLowerCase();
            if (this._headers[key]) {
                this._headers[key] += ', ' + value;
            } else {
                this._headers[key] = String(value);
            }
        };
        Headers.prototype.delete = function(name) {
            delete this._headers[name.toLowerCase()];
        };
        Headers.prototype.get = function(name) {
            return this._headers[name.toLowerCase()] || null;
        };
        Headers.prototype.has = function(name) {
            return this._headers.hasOwnProperty(name.toLowerCase());
        };
        Headers.prototype.set = function(name, value) {
            this._headers[name.toLowerCase()] = String(value);
        };
        Headers.prototype.forEach = function(callback) {
            var keys = Object.keys(this._headers);
            for (var i = 0; i < keys.length; i++) {
                callback(this._headers[keys[i]], keys[i], this);
            }
        };
        Headers.prototype.entries = function() {
            var result = [];
            var keys = Object.keys(this._headers);
            for (var i = 0; i < keys.length; i++) {
                result.push([keys[i], this._headers[keys[i]]]);
            }
            return result;
        };
        Headers.prototype.keys = function() {
            return Object.keys(this._headers);
        };
        Headers.prototype.values = function() {
            var result = [];
            var keys = Object.keys(this._headers);
            for (var i = 0; i < keys.length; i++) {
                result.push(this._headers[keys[i]]);
            }
            return result;
        };
        globalThis.Headers = Headers;

        // ==================== Response 构造函数 ====================
        function Response(body, options) {
            var opts = options || {};
            this._body = (body !== undefined && body !== null) ? body : null;
            this.status = opts.status !== undefined ? opts.status : 200;
            this.statusText = opts.statusText !== undefined ? opts.statusText : 'OK';
            this.ok = (this.status >= 200 && this.status < 300);
            this.headers = (opts.headers instanceof Headers) ? opts.headers : new Headers(opts.headers || {});
            this.url = opts.url || '';
            this.redirected = false;
            this.type = 'basic';
            this.bodyUsed = false;
        }
        Response.prototype.text = function() {
            var body = this._body;
            this.bodyUsed = true;
            return Promise.resolve(body !== null ? String(body) : '');
        };
        Response.prototype.json = function() {
            var body = this._body;
            this.bodyUsed = true;
            return Promise.resolve(JSON.parse(body !== null ? String(body) : 'null'));
        };
        Response.prototype.blob = function() {
            var body = this._body;
            this.bodyUsed = true;
            return Promise.resolve(body);
        };
        Response.prototype.arrayBuffer = function() {
            var body = this._body;
            this.bodyUsed = true;
            return Promise.resolve(body);
        };
        Response.prototype.clone = function() {
            return new Response(this._body, {
                status: this.status,
                statusText: this.statusText,
                headers: this.headers,
                url: this.url
            });
        };
        Response.error = function() {
            return new Response(null, { status: 0, statusText: '' });
        };
        Response.redirect = function(url, status) {
            return new Response(null, {
                status: status || 302,
                headers: { 'Location': url }
            });
        };
        globalThis.Response = Response;

        // ==================== fetch 函数 ====================
        // 调用原生 _fetch_request 完成 HTTP 请求，返回 Promise<Response>
        globalThis.fetch = function(input, init) {
            // 解析 URL 参数（支持字符串或 Request 对象）
            var url = (typeof input === 'string') ? input
                    : (input && input.url) ? input.url
                    : String(input);
            var options = init || {};
            var method = (options.method || 'GET').toUpperCase();

            // 处理请求体：支持 string / FormData / Blob / JSON 对象
            var body = '';
            if (options.body !== undefined && options.body !== null) {
                if (typeof options.body === 'string') {
                    body = options.body;
                } else if (options.body instanceof Blob) {
                    body = options.body._data || '';
                } else if (options.body instanceof FormData) {
                    // 将 FormData 转换为 application/x-www-form-urlencoded 格式
                    var pairs = [];
                    var entries = options.body.entries();
                    for (var i = 0; i < entries.length; i++) {
                        pairs.push(
                            encodeURIComponent(entries[i][0]) + '=' +
                            encodeURIComponent(entries[i][1])
                        );
                    }
                    body = pairs.join('&');
                } else if (typeof options.body === 'object') {
                    try { body = JSON.stringify(options.body); }
                    catch (e) { body = String(options.body); }
                } else {
                    body = String(options.body);
                }
            }

            // 处理请求头：支持 Headers 对象 / 普通对象
            var headers = null;
            if (options.headers) {
                if (options.headers instanceof Headers) {
                    headers = {};
                    options.headers.forEach(function(v, k) { headers[k] = v; });
                } else if (typeof options.headers === 'object') {
                    headers = {};
                    var hkeys = Object.keys(options.headers);
                    for (var i = 0; i < hkeys.length; i++) {
                        headers[hkeys[i]] = String(options.headers[hkeys[i]]);
                    }
                }
            }
            var headersStr = headers ? JSON.stringify(headers) : '';

            // 调用原生 HTTP 请求函数（同步执行）
            var raw = _fetch_request(method, url, body, headersStr);
            var result = JSON.parse(raw);

            // 包装为 Promise
            return new Promise(function(resolve, reject) {
                if (result.status > 0) {
                    // 构建响应头 Headers 对象
                    var responseHeaders = new Headers();
                    if (result.headers) {
                        var rkeys = Object.keys(result.headers);
                        for (var i = 0; i < rkeys.length; i++) {
                            responseHeaders.set(rkeys[i], result.headers[rkeys[i]]);
                        }
                    }
                    // 构建 Response 对象
                    var response = new Response(result.body, {
                        status: result.status,
                        statusText: result.statusText,
                        headers: responseHeaders,
                        url: result.url || url
                    });
                    resolve(response);
                } else {
                    // 网络错误时拒绝 Promise
                    reject(new Error('fetch failed: ' + result.statusText));
                }
            });
        };

        globalThis.window = globalThis;
        "#,
        )?;

        Ok(())
    })
    .map_err(|e| anyhow!("Failed to register fetch API: {:?}", e))
}
