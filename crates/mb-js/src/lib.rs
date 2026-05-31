//! JavaScript engine implementation using boa_engine
//!
//! Provides a JavaScript runtime with basic Web API bindings:
//! - console.log
//! - navigator.userAgent
//! - location.href
//! - document (DOM bridge via bind_dom)
//! - setTimeout / setInterval / clearTimeout / clearInterval (MVP)

use anyhow::{Result, anyhow};
use boa_engine::{Context, Source, JsValue, JsString};
use mb_dom::tree::DomTree;
use mb_dom::node::{NodeKind, NodeId};
use slotmap::Key;

pub mod xhr;

/// A pending timer callback registered via setTimeout/setInterval.
#[derive(Debug, Clone)]
pub struct PendingCallback {
    pub timer_id: u32,
    pub delay_ms: u32,
    pub repeating: bool,
    pub fire_at_ms: u64,
}

/// JavaScript engine wrapper around boa_engine
pub struct JsEngine {
    context: Context,
    pending_callbacks: Vec<PendingCallback>,
}

impl JsEngine {
    /// Create a new JS engine instance
    pub fn new() -> Self {
        let context = Context::default();
        Self {
            context,
            pending_callbacks: Vec::new(),
        }
    }

    /// Create a new engine with console, navigator, and location set up
    pub fn new_with_defaults() -> Self {
        let mut engine = Self::new();
        let _ = engine.setup_timers();
        let _ = engine.setup_console();
        let _ = engine.setup_navigator();
        let _ = engine.setup_location("about:blank");
        engine
    }

    /// Evaluate JavaScript code and return the result as a string
    pub fn eval(&mut self, code: &str) -> Result<String> {
        let result = self.context.eval(Source::from_bytes(code))
            .map_err(|e| anyhow!("JS evaluation error: {:?}", e))?;

        Ok(self.js_value_to_string(&result))
    }

    /// Convert a JsValue to a String representation
    fn js_value_to_string(&self, val: &JsValue) -> String {
        match val {
            JsValue::Null => "null".to_string(),
            JsValue::Undefined => "undefined".to_string(),
            JsValue::Boolean(b) => b.to_string(),
            JsValue::Integer(i) => i.to_string(),
            JsValue::Rational(f) => f.to_string(),
            JsValue::String(s) => s.to_std_string_escaped(),
            JsValue::BigInt(b) => format!("{}", b),
            _ => val.display().to_string(),
        }
    }

    /// Set a global string variable
    pub fn set_global(&mut self, name: &str, value: &str) -> Result<()> {
        let code = format!("var {} = {:?};", name, value);
        self.context.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| anyhow!("Failed to set global: {:?}", e))?;
        Ok(())
    }

    /// Get a global variable as a string
    pub fn get_global(&mut self, name: &str) -> Result<String> {
        let code = format!("typeof {0} !== 'undefined' ? {0} : undefined", name);
        let result = self.context.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| anyhow!("Failed to get global: {:?}", e))?;
        Ok(self.js_value_to_string(&result))
    }

    /// Setup console object with log method
    pub fn setup_console(&mut self) -> Result<()> {
        let code = r#"
        var console = {
            _output: [],
            log: function(...args) {
                var msg = args.map(function(a) {
                    if (a === null) return 'null';
                    if (a === undefined) return 'undefined';
                    if (typeof a === 'object') {
                        try { return JSON.stringify(a); } catch(e) { return String(a); }
                    }
                    return String(a);
                }).join(' ');
                this._output.push(msg);
            },
            error: function(...args) {
                var msg = args.map(function(a) { return String(a); }).join(' ');
                this._output.push('[ERROR] ' + msg);
            },
            warn: function(...args) {
                var msg = args.map(function(a) { return String(a); }).join(' ');
                this._output.push('[WARN] ' + msg);
            }
        };
        "#;

        self.context.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| anyhow!("Failed to setup console: {:?}", e))?;
        Ok(())
    }

    /// Get console output
    pub fn get_console_output(&mut self) -> Result<Vec<String>> {
        let code = r#"
        (function() {
            if (typeof console !== 'undefined' && console._output) {
                return console._output;
            }
            return [];
        })()
        "#;
        let result = self.context.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| anyhow!("Failed to get console output: {:?}", e))?;

        // Try to convert the array result
        let mut output = Vec::new();
        if let Some(arr) = result.as_object() {
            let length = arr.get(JsString::from("length"), &mut self.context)
                .ok()
                .and_then(|v| v.as_number())
                .unwrap_or(0.0) as usize;
            for i in 0..length {
                if let Ok(val) = arr.get(i as f64, &mut self.context) {
                    output.push(self.js_value_to_string(&val));
                }
            }
        }
        Ok(output)
    }

    /// Setup navigator object — set as a true global property.
    ///
    /// Uses `globalThis.navigator = {...}` because boa_engine 0.19
    /// does not persist `var` declarations across eval calls.
    pub fn setup_navigator(&mut self) -> Result<()> {
        let code = r#"
        globalThis.navigator = {
            userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36',
            platform: 'MacIntel',
            language: 'en-US',
            languages: ['en-US', 'en'],
            cookieEnabled: true,
            onLine: true
        };
        globalThis.window = globalThis;
        "#;

        self.context.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| anyhow!("Failed to setup navigator: {:?}", e))?;
        Ok(())
    }

    /// Setup location object — set as a true global property.
    pub fn setup_location(&mut self, url: &str) -> Result<()> {
        let (protocol, host, pathname, search, hash) = Self::parse_url_components(url);
        let origin = if host.is_empty() { String::new() } else { format!("{}{}", protocol, host) };
        // Derive port from protocol
        let port = if protocol == "https://" {
            "443".to_string()
        } else if protocol == "http://" {
            "80".to_string()
        } else {
            String::new()
        };

        let code = format!(r#"
        globalThis.location = {{
            href: {},
            protocol: {},
            host: {},
            hostname: {},
            port: {},
            pathname: {},
            search: {},
            hash: {},
            origin: {}
        }};
        globalThis.window = globalThis;
        "#, format_args!("{:?}", url),
            format_args!("{:?}", protocol),
            format_args!("{:?}", host),
            format_args!("{:?}", host),
            format_args!("{:?}", port),
            format_args!("{:?}", pathname),
            format_args!("{:?}", search),
            format_args!("{:?}", hash),
            format_args!("{:?}", origin));

        self.context.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| anyhow!("Failed to setup location: {:?}", e))?;
        Ok(())
    }

    /// Parse URL into components (simple implementation)
    fn parse_url_components(url: &str) -> (String, String, String, String, String) {
        let mut protocol = String::from("about:");
        let mut host = String::new();
        let mut pathname = String::from("/");
        let mut search = String::new();
        let mut hash = String::new();

        if let Some(proto_end) = url.find("://") {
            protocol = url[..proto_end + 3].to_string();
            let rest = &url[proto_end + 3..];

            if let Some(path_start) = rest.find('/') {
                host = rest[..path_start].to_string();
                let path_and_query = &rest[path_start..];

                if let Some(hash_pos) = path_and_query.find('#') {
                    hash = path_and_query[hash_pos..].to_string();
                    let path_and_query = &path_and_query[..hash_pos];
                    if let Some(query_pos) = path_and_query.find('?') {
                        search = path_and_query[query_pos..].to_string();
                        pathname = path_and_query[..query_pos].to_string();
                    } else {
                        pathname = path_and_query.to_string();
                    }
                } else if let Some(query_pos) = path_and_query.find('?') {
                    search = path_and_query[query_pos..].to_string();
                    pathname = path_and_query[..query_pos].to_string();
                } else {
                    pathname = path_and_query.to_string();
                }
            } else {
                host = rest.to_string();
            }
        }

        (protocol, host, pathname, search, hash)
    }

    /// Serialize a node's text content (recursive, like DOM textContent).
    fn serialize_text_content(tree: &DomTree, node_id: NodeId) -> String {
        let mut buf = String::new();
        Self::collect_text(tree, node_id, &mut buf);
        buf
    }

    fn collect_text(tree: &DomTree, node_id: NodeId, buf: &mut String) {
        let node = tree.get_node(node_id);
        if let NodeKind::Text(ref t) = node.kind {
            buf.push_str(&t.data);
        }
        let mut child = node.first_child;
        while let Some(cid) = child {
            Self::collect_text(tree, cid, buf);
            child = tree.get_node(cid).next_sibling;
        }
    }

    /// Serialize a single element node into a JS object literal string.
    fn serialize_element(tree: &DomTree, node_id: NodeId) -> String {
        let node = tree.get_node(node_id);
        let el = match &node.kind {
            NodeKind::Element(e) => e,
            _ => return "null".to_string(),
        };

        let tag = el.tag_name.to_lowercase();
        let id = el.attributes.get_value("id").unwrap_or("");
        let class_name = el.class_list.join(" ");
        let text = Self::escape_js_string(&Self::serialize_text_content(tree, node_id));

        // Collect child element node IDs
        let child_ids: Vec<u64> = tree.children(node_id)
            .into_iter()
            .map(|id| id.data().as_ffi())
            .collect();
        let child_ids_str: Vec<String> = child_ids.iter().map(|i| i.to_string()).collect();
        let child_ids_js = format!("[{}]", child_ids_str.join(","));

        // Parent node ID
        let parent_js = node.parent
            .map(|p| p.data().as_ffi().to_string())
            .unwrap_or_else(|| "null".to_string());

        // Collect all child node IDs (including text nodes)
        let mut all_child_ids = Vec::new();
        let mut child = node.first_child;
        while let Some(cid) = child {
            all_child_ids.push(cid.data().as_ffi().to_string());
            child = tree.get_node(cid).next_sibling;
        }
        let child_nodes_js = format!("[{}]", all_child_ids.join(","));

        // Attributes as key-value pairs
        let mut attrs_js = String::from("{");
        let mut first = true;
        for attr in el.attributes.iter() {
            if !first { attrs_js.push(','); }
            first = false;
            attrs_js.push_str(&format!(
                "{}:{}",
                Self::escape_js_key(&attr.name),
                Self::escape_js_string(&attr.value)
            ));
        }
        attrs_js.push('}');

        // Style as key-value pairs
        let mut style_js = String::from("{");
        let mut first = true;
        for (k, v) in el.style.iter() {
            if !first { style_js.push(','); }
            first = false;
            style_js.push_str(&format!(
                "{}:{}",
                Self::escape_js_key(k),
                Self::escape_js_string(v)
            ));
        }
        style_js.push('}');

        format!(
            r#"{{tagName:"{tag}",id:{id},className:{cls},textContent:{txt},innerHTML:"",getAttribute:function(n){{return this._attrs[n]||null}},setAttribute:function(n,v){{this._attrs[n]=String(v)}},hasAttribute:function(n){{return n in this._attrs}},style:{style},_attrs:{attrs},children:{children},childNodes:{child_nodes},parentNode:{parent}}}"#,
            tag = tag.to_uppercase(),
            id = Self::escape_js_string(id),
            cls = Self::escape_js_string(&class_name),
            txt = text,
            style = style_js,
            attrs = attrs_js,
            children = child_ids_js,
            child_nodes = child_nodes_js,
            parent = parent_js,
        )
    }

    /// Escape a string for use as a JS double-quoted string literal.
    fn escape_js_string(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for ch in s.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    /// Escape a string for use as a JS object key.
    fn escape_js_key(s: &str) -> String {
        // Check if it's a valid identifier
        let valid_id = !s.is_empty() && s.chars().next().unwrap().is_ascii_alphabetic()
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if valid_id {
            s.to_string()
        } else {
            Self::escape_js_string(s)
        }
    }

    /// Walk the entire DOM tree, find the <title> text, and the <head>/<body> node IDs.
    fn find_title(tree: &DomTree) -> String {
        if let Some(title_id) = tree.query_selector("title") {
            Self::serialize_text_content(tree, title_id)
        } else {
            String::new()
        }
    }

    /// Build an id-to-nodeId index string for getElementById lookups.
    fn build_id_index(tree: &DomTree) -> String {
        let mut entries = Vec::new();
        for (id_str, node_id) in &tree.id_index {
            entries.push(format!(
                "{}:{}",
                Self::escape_js_string(id_str),
                node_id.data().as_ffi()
            ));
        }
        format!("{{{}}}", entries.join(","))
    }

    /// Inject the DOM tree into the JS environment as a `document` object.
    ///
    /// This serializes all elements into a `__dom_elements__` global object,
    /// and creates a `document` with standard methods (getElementById, querySelector, etc.).
    ///
    /// The `document` object is also set as a true global property so that
    /// both `document.title` and `window.document.title` work.
    pub fn bind_dom(&mut self, dom: &DomTree) -> Result<()> {
        // 1. Serialize all element nodes
        let mut elements_js = Vec::new();
        for (node_id, node) in dom.nodes.iter() {
            if let NodeKind::Element(_) = &node.kind {
                let key = node_id.data().as_ffi();
                let serialized = Self::serialize_element(dom, node_id);
                elements_js.push(format!("{}:{}", key, serialized));
            }
        }

        // 2. Serialize text nodes into __dom_elements__ as well
        for (node_id, node) in dom.nodes.iter() {
            if let NodeKind::Text(ref t) = &node.kind {
                let key = node_id.data().as_ffi();
                let text = Self::escape_js_string(&t.data);
                let parent_js = node.parent
                    .map(|p| p.data().as_ffi().to_string())
                    .unwrap_or_else(|| "null".to_string());
                elements_js.push(format!(
                    "{}:{{nodeType:3,nodeName:\"#text\",textContent:{},parentNode:{}}}",
                    key, text, parent_js
                ));
            }
        }

        // 3. Build id index for getElementById
        let id_index = Self::build_id_index(dom);

        // 4. Find title, head, body node IDs
        let title = Self::escape_js_string(&Self::find_title(dom));
        let head_id = dom.head_node.data().as_ffi();
        let body_id = dom.body_node.data().as_ffi();

        // 5. Inject everything into JS
        let mut code = String::new();
        code.push_str(&format!("var __dom_elements__ = {{{}}};\n", elements_js.join(",\n")));
        code.push_str(&format!("var __dom_by_id__ = {};\n", id_index));
        code.push_str(&format!("var __dom_head_id__ = {};\n", head_id));
        code.push_str(&format!("var __dom_body_id__ = {};\n", body_id));
        code.push_str(
            "var __dom_node_name__ = function(n) {\n\
             if (n === null || n === undefined) return null;\n\
             var e = __dom_elements__[n];\n\
             return e || null;\n\
             };\n",
        );
        code.push_str(&format!(r#"globalThis.document = {{
            _title: {title},
            nodeType: 9,
            nodeName: '#document',
            get title() {{ return this._title; }},
            set title(v) {{ this._title = String(v); }},
            get head() {{ return __dom_node_name__(__dom_head_id__); }},
            get body() {{ return __dom_node_name__(__dom_body_id__); }},
            cookie: "",
            getElementById: function(id) {{
                var nid = __dom_by_id__[id];
                if (nid === undefined || nid === null) return null;
                return __dom_elements__[nid] || null;
            }},
            getElementsByTagName: function(tag) {{
                tag = tag.toUpperCase();
                var result = [];
                for (var k in __dom_elements__) {{
                    var e = __dom_elements__[k];
                    if (e && e.tagName === tag) result.push(e);
                }}
                return result;
            }},
            getElementsByClassName: function(cls) {{
                var result = [];
                for (var k in __dom_elements__) {{
                    var e = __dom_elements__[k];
                    if (e && e.className && e.className.split(' ').indexOf(cls) >= 0) result.push(e);
                }}
                return result;
            }},
            querySelector: function(sel) {{
                sel = sel.trim();
                if (sel.charAt(0) === '#') {{
                    return this.getElementById(sel.substring(1));
                }}
                if (sel.charAt(0) === '.') {{
                    var cls = sel.substring(1);
                    var arr = this.getElementsByClassName(cls);
                    return arr.length > 0 ? arr[0] : null;
                }}
                var arr = this.getElementsByTagName(sel);
                return arr.length > 0 ? arr[0] : null;
            }},
            querySelectorAll: function(sel) {{
                sel = sel.trim();
                if (sel.charAt(0) === '#') {{
                    var e = this.getElementById(sel.substring(1));
                    return e ? [e] : [];
                }}
                if (sel.charAt(0) === '.') {{
                    return this.getElementsByClassName(sel.substring(1));
                }}
                return this.getElementsByTagName(sel);
            }},
"#, title = title,
        ));
        // createElement is split into a separate push_str to avoid
        // complex escaping issues inside format!().
        code.push_str(r#"            createElement: function(tag) {
                var newId = 'created_' + (++this._createCounter);
                var el = {
                    tagName: tag.toUpperCase(),
                    id: "",
                    className: "",
                    textContent: "",
                    innerHTML: "",
                    getAttribute: function(n) { return this._attrs[n] || null; },
                    setAttribute: function(n, v) { this._attrs[n] = String(v); },
                    hasAttribute: function(n) { return n in this._attrs; },
                    style: {},
                    _attrs: {},
                    children: [],
                    childNodes: [],
                    parentNode: null
                };
                __dom_elements__[newId] = el;
                return el;
            },
            _createCounter: 0
            };
"#);

        // Set window alias so `window.xxx` works.
        code.push_str("globalThis.window = globalThis;\n");

        // Fault-tolerant: if boa_engine cannot parse the generated JS
        // (e.g. getter/setter syntax not supported), log a warning and
        // continue instead of crashing the whole page load.
        if let Err(e) = self.context.eval(Source::from_bytes(code.as_bytes())) {
            eprintln!("[bind_dom] JS eval error (continuing anyway): {:?}", e);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Timer infrastructure (MVP)
    // ------------------------------------------------------------------

    /// Register the global setTimeout / setInterval / clearTimeout /
    /// clearInterval functions.
    ///
    /// Callbacks are accumulated in a JS-side `__pending_callbacks__`
    /// array that can be drained from Rust via [`drain_callbacks`].
    ///
    /// Uses `globalThis.xxx = ...` instead of `var` / `function`
    /// declarations because boa_engine 0.19 does not persist
    /// eval-local bindings across separate eval calls.
    pub fn setup_timers(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            var __next_timer_id__ = 1;
            var __pending_callbacks__ = [];

            globalThis.setTimeout = function(fn, delay) {
                var id = __next_timer_id__++;
                __pending_callbacks__.push({
                    timer_id: id,
                    callback: fn,
                    delay_ms: delay || 0,
                    repeating: false
                });
                return id;
            };

            globalThis.setInterval = function(fn, delay) {
                var id = __next_timer_id__++;
                __pending_callbacks__.push({
                    timer_id: id,
                    callback: fn,
                    delay_ms: delay || 0,
                    repeating: true
                });
                return id;
            };

            globalThis.clearTimeout = function(id) {
                for (var i = 0; i < __pending_callbacks__.length; i++) {
                    if (__pending_callbacks__[i].timer_id === id) {
                        __pending_callbacks__.splice(i, 1);
                        return;
                    }
                }
            };

            globalThis.clearInterval = globalThis.clearTimeout;

            globalThis.__drainTimerCallbacks__ = function() {
                var out = __pending_callbacks__.slice();
                __pending_callbacks__.length = 0;
                return out;
            };
        })();
        "#;

        self.context.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| anyhow!("Failed to setup timers: {:?}", e))?;
        Ok(())
    }

    /// Drain pending timer callbacks from the JS side and return them
    /// as Rust-side metadata.
    ///
    /// Each entry contains the timer id, delay, repeating flag, and a
    /// fire-at timestamp (currently set to 0 — the caller can compute
    /// the actual deadline from `delay_ms` and `std::time::Instant`).
    ///
    /// After draining, the JS-side `__pending_callbacks__` array is
    /// cleared so the same callbacks are not returned twice.
    ///
    /// **MVP note:** The actual JS callback *function* stays on the JS
    /// side; only metadata is returned.  A future iteration can store
    /// `JsValue` references in `PendingCallback` and fire them
    /// synchronously from Rust.
    pub fn drain_callbacks(&mut self) -> Result<Vec<PendingCallback>> {
        let code = r#"
        (function() {
            if (typeof __drainTimerCallbacks__ === 'undefined') return [];
            return __drainTimerCallbacks__();
        })()
        "#;

        let result = self.context.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| anyhow!("Failed to drain callbacks: {:?}", e))?;

        let mut callbacks = Vec::new();
        if let Some(arr) = result.as_object() {
            let len = arr.get(JsString::from("length"), &mut self.context)
                .ok()
                .and_then(|v| v.as_number())
                .unwrap_or(0.0) as usize;

            for i in 0..len {
                if let Ok(entry) = arr.get(i as f64, &mut self.context) {
                    if let Some(obj) = entry.as_object() {
                        let timer_id = obj
                            .get(JsString::from("timer_id"), &mut self.context)
                            .ok()
                            .and_then(|v| v.as_number())
                            .unwrap_or(0.0) as u32;

                        let delay_ms = obj
                            .get(JsString::from("delay_ms"), &mut self.context)
                            .ok()
                            .and_then(|v| v.as_number())
                            .unwrap_or(0.0) as u32;

                        let repeating = obj
                            .get(JsString::from("repeating"), &mut self.context)
                            .ok()
                            .and_then(|v| v.as_boolean())
                            .unwrap_or(false);

                        callbacks.push(PendingCallback {
                            timer_id,
                            delay_ms,
                            repeating,
                            fire_at_ms: 0, // Caller can compute actual deadline
                        });
                    }
                }
            }
        }
        Ok(callbacks)
    }

    /// Setup XMLHttpRequest support with the given HTTP client
    pub fn setup_xhr(&mut self, client: std::sync::Arc<mb_network::client::HttpClient>) -> Result<()> {
        let handle = tokio::runtime::Handle::current();
        xhr::register_xhr(&mut self.context, client, handle)
    }
}

impl Default for JsEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eval_simple() {
        let mut engine = JsEngine::new();
        let result = engine.eval("1 + 1").unwrap();
        assert_eq!(result, "2");
    }

    #[test]
    fn test_eval_string() {
        let mut engine = JsEngine::new();
        let result = engine.eval("'hello' + ' ' + 'world'").unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_set_get_global() {
        let mut engine = JsEngine::new();
        engine.set_global("myVar", "42").unwrap();
        let result = engine.get_global("myVar").unwrap();
        assert_eq!(result, "42");
    }

    #[test]
    fn test_navigator_global() {
        let mut engine = JsEngine::new_with_defaults();
        let result = engine.eval("typeof navigator").unwrap();
        assert_eq!(result, "object");
    }

    #[test]
    fn test_window_alias() {
        let mut engine = JsEngine::new_with_defaults();
        let result = engine.eval("typeof window").unwrap();
        assert_eq!(result, "object");
    }

    #[test]
    fn test_window_navigator() {
        let mut engine = JsEngine::new_with_defaults();
        let result = engine.eval("window.navigator.userAgent").unwrap();
        assert!(result.contains("Chrome"), "Expected Chrome UA, got: {}", result);
    }

    #[test]
    fn test_location_global() {
        let mut engine = JsEngine::new_with_defaults();
        let result = engine.eval("typeof location").unwrap();
        assert_eq!(result, "object");
    }

    #[test]
    fn test_window_location() {
        let mut engine = JsEngine::new_with_defaults();
        engine.setup_location("https://example.com/path?q=1").unwrap();
        let result = engine.eval("window.location.href").unwrap();
        assert_eq!(result, "https://example.com/path?q=1");
    }

    #[test]
    fn test_settimeout_typeof() {
        let mut engine = JsEngine::new_with_defaults();
        let result = engine.eval("typeof setTimeout").unwrap();
        assert_eq!(result, "function");
    }

    #[test]
    fn test_setinterval_typeof() {
        let mut engine = JsEngine::new_with_defaults();
        let result = engine.eval("typeof setInterval").unwrap();
        assert_eq!(result, "function");
    }

    #[test]
    fn test_settimeout_returns_id() {
        let mut engine = JsEngine::new_with_defaults();
        let result = engine.eval("setTimeout(function(){}, 100)").unwrap();
        let id: u32 = result.parse().unwrap();
        assert!(id >= 1, "Expected timer_id >= 1, got {}", id);
    }

    #[test]
    fn test_drain_callbacks() {
        let mut engine = JsEngine::new_with_defaults();
        engine.eval("setTimeout(function(){}, 100)").unwrap();
        engine.eval("setInterval(function(){}, 200)").unwrap();
        let cb = engine.drain_callbacks().unwrap();
        assert_eq!(cb.len(), 2);
        assert_eq!(cb[0].delay_ms, 100);
        assert!(!cb[0].repeating);
        assert_eq!(cb[1].delay_ms, 200);
        assert!(cb[1].repeating);
        // Second drain should be empty
        let cb2 = engine.drain_callbacks().unwrap();
        assert_eq!(cb2.len(), 0);
    }

    #[test]
    fn test_cleartimeout() {
        let mut engine = JsEngine::new_with_defaults();
        let id = engine.eval("var tid = setTimeout(function(){}, 100); tid").unwrap();
        engine.eval(&format!("clearTimeout({})", id)).unwrap();
        let cb = engine.drain_callbacks().unwrap();
        assert_eq!(cb.len(), 0, "Cleared timer should not appear in drain");
    }
}
