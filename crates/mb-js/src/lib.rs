//! JavaScript engine implementation using rquickjs (QuickJS-NG)
//!
//! Provides a JavaScript runtime with basic Web API bindings:
//! - console.log
//! - navigator.userAgent
//! - location.href
//! - document (DOM bridge via bind_dom)
//! - setTimeout / setInterval / clearTimeout / clearInterval (MVP)

use anyhow::{Result, anyhow};
use rquickjs::{Context as QContext, Runtime, Ctx, Value, String as JsString, Function};
use rquickjs::function::Rest;
use mb_dom::tree::DomTree;
use mb_dom::node::{NodeKind, NodeId};
use mb_dom::selector::SelectorEngine;
use slotmap::Key;
use std::cell::RefCell;
use std::collections::HashMap;

pub mod xhr;

/// Thread-local storage for DOM element data (used by native querySelectorAll)
/// Maps node_id (u64) -> (tag_name_uppercase, class_list, id_attr, parent_id)
thread_local! {
    static DOM_ELEMENTS: RefCell<HashMap<u64, (String, Vec<String>, String, Option<u64>)>> = RefCell::new(HashMap::new());
}

/// A mutation that was performed on the JS side and needs to be applied to the Rust DomTree.
#[derive(Debug, Clone)]
pub struct Mutation {
    pub kind: MutationKind,
}

/// The type of DOM mutation.
#[derive(Debug, Clone)]
pub enum MutationKind {
    SetAttribute { node_id: u64, name: String, value: String },
    RemoveAttribute { node_id: u64, name: String },
    SetTextContent { node_id: u64, text: String },
    AppendChild { parent_id: u64, child_tag: String },
    RemoveChild { parent_id: u64, child_id: u64 },
    SetInnerHTML { node_id: u64, html: String },
}

/// A pending timer callback registered via setTimeout/setInterval.
#[derive(Debug, Clone)]
pub struct PendingCallback {
    pub timer_id: u32,
    pub delay_ms: u32,
    pub repeating: bool,
    pub fire_at_ms: u64,
}

/// JavaScript engine wrapper around rquickjs
pub struct JsEngine {
    runtime: Runtime,
    context: QContext,
    pending_callbacks: Vec<PendingCallback>,
    /// Mutations recorded by JS that need to be applied to the Rust DomTree.
    pub mutations: Vec<Mutation>,
}

/// Convert a rquickjs Value to a String representation
fn js_value_to_string(val: &Value) -> String {
    if val.is_null() {
        "null".to_string()
    } else if val.is_undefined() {
        "undefined".to_string()
    } else if val.is_bool() {
        val.as_bool().unwrap_or(false).to_string()
    } else if val.is_int() {
        val.as_int().unwrap_or(0).to_string()
    } else if val.is_float() || val.is_number() {
        val.as_float().unwrap_or(0.0).to_string()
    } else if val.is_string() {
        val.as_string().and_then(|s| s.to_string().ok()).unwrap_or_default()
    } else {
        format!("{:?}", val)
    }
}

/// Extract a number from a Value (handles both Int and Float)
fn value_to_f64(val: &Value) -> Option<f64> {
    val.as_float()
}

/// Extract a u64 from a Value
fn value_to_u64(val: &Value) -> Option<u64> {
    val.as_float().map(|f| f as u64)
}

/// Extract a string from a Value
fn value_to_string(val: &Value) -> Option<String> {
    if val.is_string() {
        val.as_string().and_then(|s| s.to_string().ok())
    } else {
        None
    }
}

/// Extract a boolean from a Value
fn value_to_bool(val: &Value) -> Option<bool> {
    val.as_bool()
}

impl JsEngine {
    /// Create a new JS engine instance
    pub fn new() -> Self {
        let rt = Runtime::new().unwrap();
        let ctx = QContext::full(&rt).unwrap();
        Self {
            runtime: rt,
            context: ctx,
            pending_callbacks: Vec::new(),
            mutations: Vec::new(),
        }
    }

    /// Create a new engine with all anti-detection setup
    pub fn new_with_defaults() -> Self {
        let mut engine = Self::new();
        let _ = engine.run_setup();
        engine
    }

    /// Run all setup functions for a full browser-like environment
    pub fn run_setup(&mut self) -> Result<()> {
        self.setup_timers()?;
        self.setup_console()?;
        self.setup_navigator()?;
        self.setup_location("about:blank")?;
        self.setup_screen()?;
        self.setup_chrome()?;
        self.setup_performance()?;
        self.setup_misc()?;
        Ok(())
    }

    /// Evaluate JavaScript code and return the result as a string
    pub fn eval(&mut self, code: &str) -> Result<String> {
        let result_str = self.context.with(|ctx| -> rquickjs::Result<String> {
            let val: Value = ctx.eval(code)?;
            Ok(js_value_to_string(&val))
        }).map_err(|e| anyhow!("JS evaluation error: {:?}", e))?;

        // Drain pending Promise jobs (execute .then() callbacks)
        // NOTE: Must be called outside ctx.with() due to RefCell borrow conflict
        let mut job_rounds = 0;
        loop {
            match self.runtime.execute_pending_job() {
                Ok(true) => { job_rounds += 1; }
                Ok(false) => break,
                Err(e) => {
                    tracing::warn!("Promise job error: {:?}", e);
                    break;
                }
            }
            if job_rounds > 100 { break; }
        }
        if job_rounds > 0 {
            tracing::debug!("Executed {} Promise job rounds", job_rounds);
        }

        // Drain any DOM mutations that were queued during eval
        if let Err(e) = self.drain_js_mutations() {
            tracing::warn!("Failed to drain JS mutations: {}", e);
        }

        // Drain and execute pending timer callbacks (up to 5 rounds)
        let _ = self.drain_and_execute_timers();

        Ok(result_str)
    }

    /// Drain JS-side __mutations__ array and add to self.mutations.
    fn drain_js_mutations(&mut self) -> Result<()> {
        let code = r#"
        (function() {
            if (typeof __mutations__ === 'undefined') return [];
            var out = __mutations__.slice();
            __mutations__.length = 0;
            return out;
        })()
        "#;
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let result: Value = ctx.eval(code)?;

            if let Some(arr) = result.as_object() {
                let len: usize = arr.get::<_, Value>("length")
                    .ok()
                    .and_then(|v| value_to_f64(&v))
                    .unwrap_or(0.0) as usize;

                for i in 0..len {
                    if let Ok(entry) = arr.get::<_, Value>(i as u32) {
                        if let Some(obj) = entry.as_object() {
                            let kind = obj.get::<_, Value>("type")
                                .ok()
                                .and_then(|v| value_to_string(&v))
                                .unwrap_or_default();

                            let get_num = |key: &str| -> Option<u64> {
                                obj.get::<_, Value>(key)
                                    .ok()
                                    .and_then(|v| value_to_u64(&v))
                            };
                            let get_str = |key: &str| -> String {
                                obj.get::<_, Value>(key)
                                    .ok()
                                    .and_then(|v| value_to_string(&v))
                                    .unwrap_or_default()
                            };

                            let mutation_kind = match kind.as_str() {
                                "setAttribute" => {
                                    let node_id = get_num("nodeId").unwrap_or(0);
                                    let name = get_str("name");
                                    let value = get_str("value");
                                    if node_id > 0 && !name.is_empty() {
                                        Some(MutationKind::SetAttribute { node_id, name, value })
                                    } else { None }
                                }
                                "removeAttribute" => {
                                    let node_id = get_num("nodeId").unwrap_or(0);
                                    let name = get_str("name");
                                    if node_id > 0 && !name.is_empty() {
                                        Some(MutationKind::RemoveAttribute { node_id, name })
                                    } else { None }
                                }
                                "textContent" => {
                                    let node_id = get_num("nodeId").unwrap_or(0);
                                    let text = get_str("text");
                                    if node_id > 0 {
                                        Some(MutationKind::SetTextContent { node_id, text })
                                    } else { None }
                                }
                                "innerHTML" => {
                                    let node_id = get_num("nodeId").unwrap_or(0);
                                    let html = get_str("html");
                                    if node_id > 0 {
                                        Some(MutationKind::SetInnerHTML { node_id, html })
                                    } else { None }
                                }
                                "appendChild" => {
                                    let parent_id = get_num("parentId").unwrap_or(0);
                                    let child_tag = get_str("childTag");
                                    if parent_id > 0 {
                                        Some(MutationKind::AppendChild { parent_id, child_tag })
                                    } else { None }
                                }
                                "removeChild" => {
                                    let parent_id = get_num("parentId").unwrap_or(0);
                                    let child_id = get_num("childId").unwrap_or(0);
                                    if parent_id > 0 && child_id > 0 {
                                        Some(MutationKind::RemoveChild { parent_id, child_id })
                                    } else { None }
                                }
                                _ => None,
                            };

                            if let Some(kind) = mutation_kind {
                                self.mutations.push(Mutation { kind });
                            }
                        }
                    }
                }
            }
            Ok(())
        }).map_err(|e| anyhow!("Failed to drain mutations: {:?}", e))?;
        Ok(())
    }

    /// Drain and return all pending mutations.
    pub fn drain_mutations(&mut self) -> Vec<Mutation> {
        std::mem::take(&mut self.mutations)
    }

    /// Set a global string variable
    pub fn set_global(&mut self, name: &str, value: &str) -> Result<()> {
        let code = format!("var {} = {:?};", name, value);
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code.as_str())?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to set global: {:?}", e))?;
        Ok(())
    }

    /// Get a global variable as a string
    pub fn get_global(&mut self, name: &str) -> Result<String> {
        let code = format!("typeof {0} !== 'undefined' ? {0} : undefined", name);
        let result = self.context.with(|ctx| -> rquickjs::Result<String> {
            let val: Value = ctx.eval(code.as_str())?;
            Ok(js_value_to_string(&val))
        }).map_err(|e| anyhow!("Failed to get global: {:?}", e))?;
        Ok(result)
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

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup console: {:?}", e))?;
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
        let output = self.context.with(|ctx| -> rquickjs::Result<Vec<String>> {
            let result: Value = ctx.eval(code)?;
            let mut output = Vec::new();
            if let Some(arr) = result.as_object() {
                let length: usize = arr.get::<_, Value>("length")
                    .ok()
                    .and_then(|v| value_to_f64(&v))
                    .unwrap_or(0.0) as usize;
                for i in 0..length {
                    if let Ok(val) = arr.get::<_, Value>(i as u32) {
                        output.push(js_value_to_string(&val));
                    }
                }
            }
            Ok(output)
        }).map_err(|e| anyhow!("Failed to get console output: {:?}", e))?;
        Ok(output)
    }

    /// Setup navigator object — set as a true global property.
    pub fn setup_navigator(&mut self) -> Result<()> {
        let code = r#"
        globalThis.navigator = {
            userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36',
            platform: 'MacIntel',
            language: 'en-US',
            languages: ['en-US', 'en'],
            cookieEnabled: true,
            onLine: true,
            vendor: 'Google Inc.',
            maxTouchPoints: 0,
            deviceMemory: 8,
            hardwareConcurrency: 8,
            plugins: [
                { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
                { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' },
                { name: 'Native Client', filename: 'internal-nacl-plugin', description: '' }
            ],
            mimeTypes: []
        };
        globalThis.navigator.plugins.length = 3;
        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup navigator: {:?}", e))?;
        Ok(())
    }

    /// Setup screen object with typical desktop resolution
    pub fn setup_screen(&mut self) -> Result<()> {
        let code = r#"
        globalThis.screen = {
            width: 1920,
            height: 1080,
            availWidth: 1920,
            availHeight: 1040,
            colorDepth: 24,
            pixelDepth: 24,
            orientation: { angle: 0, type: 'landscape-primary' }
        };
        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup screen: {:?}", e))?;
        Ok(())
    }

    /// Setup window.chrome object for anti-detection
    pub fn setup_chrome(&mut self) -> Result<()> {
        let code = r#"
        globalThis.chrome = {
            runtime: {
                OnInstalledReason: {},
                OnRestartRequiredReason: {},
                PlatformArch: {},
                PlatformNaclArch: {},
                PlatformOs: {},
                RequestUpdateCheckStatus: {}
            },
            loadTimes: function() {
                return {
                    commitLoadTime: Date.now() / 1000,
                    connectionInfo: 'h2',
                    finishDocumentLoadTime: Date.now() / 1000,
                    finishLoadTime: Date.now() / 1000,
                    firstPaintAfterLoadTime: 0,
                    firstPaintTime: Date.now() / 1000,
                    navigationType: 'Other',
                    npnNegotiatedProtocol: 'h2',
                    requestTime: Date.now() / 1000,
                    startLoadTime: Date.now() / 1000,
                    wasAlternateProtocolAvailable: false,
                    wasFetchedViaSpdy: true,
                    wasNpnNegotiated: true
                };
            },
            csi: function() {
                var _ps = globalThis.__perf_start__ || Date.now();
                return {
                    onloadT: Date.now(),
                    pageT: Date.now() - _ps,
                    startE: Date.now(),
                    tran: 15
                };
            }
        };
        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup chrome: {:?}", e))?;
        Ok(())
    }

    /// Setup performance.now() and timeOrigin for anti-detection
    pub fn setup_performance(&mut self) -> Result<()> {
        let code = r#"
        globalThis.__perf_start__ = Date.now();
        globalThis.performance = {
            timeOrigin: globalThis.__perf_start__,
            now: function() {
                return Date.now() - globalThis.__perf_start__;
            },
            timing: {
                navigationStart: globalThis.__perf_start__,
                unloadEventStart: 0,
                unloadEventEnd: 0,
                redirectStart: 0,
                redirectEnd: 0,
                fetchStart: globalThis.__perf_start__,
                domainLookupStart: globalThis.__perf_start__,
                domainLookupEnd: globalThis.__perf_start__,
                connectStart: globalThis.__perf_start__,
                connectEnd: globalThis.__perf_start__,
                secureConnectionStart: globalThis.__perf_start__,
                requestStart: globalThis.__perf_start__,
                responseStart: globalThis.__perf_start__,
                responseEnd: globalThis.__perf_start__,
                domLoading: globalThis.__perf_start__,
                domInteractive: globalThis.__perf_start__,
                domContentLoadedEventStart: globalThis.__perf_start__,
                domContentLoadedEventEnd: globalThis.__perf_start__,
                domComplete: globalThis.__perf_start__,
                loadEventStart: globalThis.__perf_start__,
                loadEventEnd: globalThis.__perf_start__
            }
        };
        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup performance: {:?}", e))?;
        Ok(())
    }

    /// Setup misc Web APIs: atob, btoa, matchMedia, localStorage, sessionStorage
    pub fn setup_misc(&mut self) -> Result<()> {
        let code = r#"
        globalThis.atob = function(s) { return s; };
        globalThis.btoa = function(s) { return s; };
        globalThis.matchMedia = function(query) {
            return {
                matches: false,
                media: query,
                onchange: null,
                addListener: function() {},
                removeListener: function() {},
                addEventListener: function() {},
                removeEventListener: function() {},
                dispatchEvent: function() { return true; }
            };
        };

        // localStorage — in-memory key-value store (persists within session)
        (function() {
            var _store = {};
            globalThis.localStorage = {
                getItem: function(key) { return _store.hasOwnProperty(key) ? _store[key] : null; },
                setItem: function(key, value) { _store[key] = String(value); },
                removeItem: function(key) { delete _store[key]; },
                clear: function() { _store = {}; },
                get length() { return Object.keys(_store).length; },
                key: function(index) { return Object.keys(_store)[index] || null; }
            };
        })();

        // sessionStorage — in-memory key-value store (per-tab, cleared on close)
        (function() {
            var _store = {};
            globalThis.sessionStorage = {
                getItem: function(key) { return _store.hasOwnProperty(key) ? _store[key] : null; },
                setItem: function(key, value) { _store[key] = String(value); },
                removeItem: function(key) { delete _store[key]; },
                clear: function() { _store = {}; },
                get length() { return Object.keys(_store).length; },
                key: function(index) { return Object.keys(_store)[index] || null; }
            };
        })();

        globalThis.window = globalThis;
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup misc: {:?}", e))?;
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

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code.as_str())?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup location: {:?}", e))?;
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
    /// Returns (element_literal, property_definitions) where property_definitions
    /// are Object.defineProperty calls for textContent/innerHTML.
    fn serialize_element(tree: &DomTree, node_id: NodeId) -> (String, Vec<String>) {
        let node = tree.get_node(node_id);
        let el = match &node.kind {
            NodeKind::Element(e) => e,
            _ => return ("null".to_string(), Vec::new()),
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

        let nid = node_id.data().as_ffi();

        let element_literal = format!(
            r#"{{tagName:"{tag}",id:{id},className:{cls},_nodeId:{nid},_textContent:{txt},_innerHTML:"",getAttribute:function(n){{return this._attrs[n]||null}},setAttribute:function(n,v){{this._attrs[n]=String(v);__mut_set_attr__(this._nodeId,n,String(v))}},removeAttribute:function(n){{delete this._attrs[n];__mut_remove_attr__(this._nodeId,n)}},hasAttribute:function(n){{return n in this._attrs}},remove:function(){{if(this.parentNode!==null){{__mut_remove_child__(this.parentNode,this._nodeId)}}}},style:{style},_attrs:{attrs},children:{children},childNodes:{child_nodes},parentNode:{parent}}}"#,
            tag = tag.to_uppercase(),
            id = Self::escape_js_string(id),
            cls = Self::escape_js_string(&class_name),
            txt = text,
            nid = nid,
            style = style_js,
            attrs = attrs_js,
            children = child_ids_js,
            child_nodes = child_nodes_js,
            parent = parent_js,
        );

        // Object.defineProperty calls for textContent and innerHTML with mutation tracking.
        let mut prop_defs = Vec::new();
        prop_defs.push(format!(
            r#"Object.defineProperty(__dom_elements__[{nid}],'textContent',{{get:function(){{return this._textContent}},set:function(v){{this._textContent=String(v);__mut_set_text__({nid},String(v))}},enumerable:true,configurable:true}})"#,
            nid = nid
        ));
        prop_defs.push(format!(
            r#"Object.defineProperty(__dom_elements__[{nid}],'innerHTML',{{get:function(){{return this._innerHTML}},set:function(v){{this._innerHTML=String(v);__mut_set_inner_html__({nid},String(v))}},enumerable:true,configurable:true}})"#,
            nid = nid
        ));

        (element_literal, prop_defs)
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
        // Check if it's a valid JS identifier (no hyphens!)
        let valid_id = !s.is_empty() && s.chars().next().unwrap().is_ascii_alphabetic()
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
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
    pub fn bind_dom(&mut self, dom: &DomTree) -> Result<()> {
        // 0. Setup mutation queue (must be done before elements are created)
        self.setup_mutation_queue()?;

        // 1. Serialize all element nodes and populate DOM_ELEMENTS for native selectors
        let mut elements_js = Vec::new();
        let mut prop_defs_js = Vec::new();
        DOM_ELEMENTS.with(|de| { de.borrow_mut().clear(); });
        for (node_id, node) in dom.nodes.iter() {
            if let NodeKind::Element(el) = &node.kind {
                let key = node_id.data().as_ffi();
                let (serialized, prop_defs) = Self::serialize_element(dom, node_id);
                elements_js.push(format!("{}:{}", key, serialized));
                for def in prop_defs {
                    prop_defs_js.push(def);
                }
                // Store element data for native querySelectorAll
                let parent_id = node.parent.map(|p| p.data().as_ffi());
                DOM_ELEMENTS.with(|de| {
                    de.borrow_mut().insert(key, (
                        el.tag_name.clone(),
                        el.class_list.clone(),
                        el.attributes.get_value("id").unwrap_or("").to_string(),
                        parent_id,
                    ));
                });
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
        for def in &prop_defs_js {
            code.push_str(def);
            code.push_str(";\n");
        }
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

        // Set window alias
        code.push_str("globalThis.window = globalThis;\n");

        // Eval 1: document + window globals (must succeed)
        let doc_code = format!(
            r#"globalThis.window = globalThis;
globalThis.document = {{
    _title: {title},
    nodeType: 9,
    nodeName: '#document',
    get title() {{ return this._title; }},
    set title(v) {{ this._title = String(v); }},
    get head() {{ return (typeof __dom_node_name__ === 'function') ? __dom_node_name__(__dom_head_id__) : null; }},
    get body() {{ return (typeof __dom_node_name__ === 'function') ? __dom_node_name__(__dom_body_id__) : null; }},
    cookie: "",
    getElementById: function(id) {{
        if (typeof __dom_by_id__ === 'undefined') return null;
        var nid = __dom_by_id__[id];
        if (nid === undefined || nid === null) return null;
        return (typeof __dom_elements__ !== 'undefined') ? (__dom_elements__[nid] || null) : null;
    }},
    getElementsByTagName: function(tag) {{
        tag = tag.toUpperCase();
        var result = [];
        if (typeof __dom_elements__ !== 'undefined') {{
            for (var k in __dom_elements__) {{
                var e = __dom_elements__[k];
                if (e && e.tagName === tag) result.push(e);
            }}
        }}
        return result;
    }},
    getElementsByClassName: function(cls) {{
        var result = [];
        if (typeof __dom_elements__ !== 'undefined') {{
            for (var k in __dom_elements__) {{
                var e = __dom_elements__[k];
                if (e && e.className && e.className.split(' ').indexOf(cls) >= 0) result.push(e);
            }}
        }}
        return result;
    }},
    querySelector: function(sel) {{
        sel = sel.trim();
        if (typeof _dom_query_selector_all === 'function') {{
            var ids = _dom_query_selector_all(sel);
            if (ids.length > 0) return __dom_elements__[ids[0]] || null;
        }}
        // Fallback for simple selectors
        if (sel.charAt(0) === '#') {{ return this.getElementById(sel.substring(1)); }}
        if (sel.charAt(0) === '.') {{
            var arr = this.getElementsByClassName(sel.substring(1));
            return arr.length > 0 ? arr[0] : null;
        }}
        var arr = this.getElementsByTagName(sel);
        return arr.length > 0 ? arr[0] : null;
    }},
    querySelectorAll: function(sel) {{
        sel = sel.trim();
        if (typeof _dom_query_selector_all === 'function') {{
            var ids = _dom_query_selector_all(sel);
            var result = [];
            for (var i = 0; i < ids.length; i++) {{
                var el = __dom_elements__[ids[i]];
                if (el) result.push(el);
            }}
            return result;
        }}
        // Fallback for simple selectors
        if (sel.charAt(0) === '#') {{ var e = this.getElementById(sel.substring(1)); return e ? [e] : []; }}
        if (sel.charAt(0) === '.') {{ return this.getElementsByClassName(sel.substring(1)); }}
        return this.getElementsByTagName(sel);
    }},
    createElement: function(tag) {{
        var newId = 'created_' + (++this._createCounter);
        var el = {{
            tagName: tag.toUpperCase(), id: "", className: "", textContent: "", innerHTML: "",
            _attrs: {{}}, children: [], childNodes: [], parentNode: null,
            getAttribute: function(n) {{ return this._attrs[n] || null; }},
            setAttribute: function(n, v) {{ this._attrs[n] = String(v); }},
            hasAttribute: function(n) {{ return n in this._attrs; }},
            style: {{}}
        }};
        if (typeof __dom_elements__ !== 'undefined') __dom_elements__[newId] = el;
        return el;
    }},
    _createCounter: 0
}};
"#,
            title = title,
        );

        // Register native querySelectorAll function
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let func = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<Vec<u64>> {
                let selector = args.get(0)
                    .and_then(|v| v.as_string())
                    .and_then(|s| s.to_string().ok())
                    .unwrap_or_default();
                
                let results: Vec<u64> = DOM_ELEMENTS.with(|de| {
                    let elements = de.borrow();
                    // Build a mini DomTree-like structure for selector matching
                    // For each element, check if it matches the selector
                    let engine = SelectorEngine::parse(&selector);
                    elements.iter()
                        .filter(|(nid, (tag, classes, id, parent))| {
                            // Check each selector in the engine
                            engine.selectors.iter().any(|sel| {
                                if sel.parts.is_empty() { return false; }
                                // For simple selectors (1 part), just check the last part
                                if sel.parts.len() == 1 {
                                    let (ref simple, _) = sel.parts[0];
                                    // Tag check
                                    if let Some(ref t) = simple.tag {
                                        if !tag.eq_ignore_ascii_case(t) { return false; }
                                    }
                                    // Class check
                                    for cls in &simple.classes {
                                        if !classes.iter().any(|c| c == cls) { return false; }
                                    }
                                    // ID check
                                    if let Some(ref id_match) = simple.id {
                                        if id != id_match { return false; }
                                    }
                                    return true;
                                }
                                // For compound selectors (descendant/child), we need ancestor chain
                                // Simple approach: check if current node matches rightmost part,
                                // then walk up ancestors checking left parts
                                let (ref right_simple, _) = sel.parts[sel.parts.len() - 1];
                                // Check rightmost part against current node
                                if let Some(ref t) = right_simple.tag {
                                    if !tag.eq_ignore_ascii_case(t) { return false; }
                                }
                                for cls in &right_simple.classes {
                                    if !classes.iter().any(|c| c == cls) { return false; }
                                }
                                if let Some(ref id_match) = right_simple.id {
                                    if id != id_match { return false; }
                                }
                                // Walk up ancestors for remaining parts
                                let mut current_parent = *parent;
                                for part_idx in (0..sel.parts.len() - 1).rev() {
                                    let (ref simple, _) = sel.parts[part_idx];
                                    let mut found = false;
                                    let mut ancestor = current_parent;
                                    while let Some(aid) = ancestor {
                                        if let Some((atag, aclasses, aid_val, aparent)) = elements.get(&aid) {
                                            let mut matches = true;
                                            if let Some(ref t) = simple.tag {
                                                if !atag.eq_ignore_ascii_case(t) { matches = false; }
                                            }
                                            for cls in &simple.classes {
                                                if !aclasses.iter().any(|c| c == cls) { matches = false; }
                                            }
                                            if let Some(ref id_match) = simple.id {
                                                if aid_val != id_match { matches = false; }
                                            }
                                            if matches {
                                                current_parent = *aparent;
                                                found = true;
                                                break;
                                            }
                                            ancestor = *aparent;
                                        } else {
                                            break;
                                        }
                                    }
                                    if !found { return false; }
                                }
                                true
                            })
                        })
                        .map(|(nid, _)| *nid)
                        .collect()
                });
                Ok(results)
            })?;
            ctx.globals().set("_dom_query_selector_all", func)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to register querySelectorAll: {:?}", e))?;

        if let Err(e) = self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(doc_code.as_str())?;
            Ok(())
        }) {
            eprintln!("[bind_dom] CRITICAL: document setup failed: {:?}", e);
        }

        // Eval 2: DOM elements (may fail if inline scripts have ES2015+ syntax)
        if let Err(e) = self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code.as_str())?;
            Ok(())
        }) {
            eprintln!("[bind_dom] DOM elements eval error (continuing anyway): {:?}", e);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Mutation queue infrastructure
    // ------------------------------------------------------------------

    /// Inject the JS-side __mutations__ array and helper functions.
    pub fn setup_mutation_queue(&mut self) -> Result<()> {
        let code = r#"
        var __mutations__ = [];

        function __mut_set_attr__(nid, name, value) {
            __mutations__.push({type: 'setAttribute', nodeId: nid, name: name, value: value});
        }
        function __mut_remove_attr__(nid, name) {
            __mutations__.push({type: 'removeAttribute', nodeId: nid, name: name});
        }
        function __mut_set_text__(nid, text) {
            __mutations__.push({type: 'textContent', nodeId: nid, text: text});
        }
        function __mut_set_inner_html__(nid, html) {
            __mutations__.push({type: 'innerHTML', nodeId: nid, html: html});
        }
        function __mut_append_child__(parentId, childTag) {
            __mutations__.push({type: 'appendChild', parentId: parentId, childTag: childTag});
        }
        function __mut_remove_child__(parentId, childId) {
            __mutations__.push({type: 'removeChild', parentId: parentId, childId: childId});
        }
        "#;
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup mutation queue: {:?}", e))?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Timer infrastructure (MVP)
    // ------------------------------------------------------------------

    /// Register the global setTimeout / setInterval / clearTimeout / clearInterval functions.
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

            globalThis.__executeAllTimerCallbacks__ = function() {
                var batch = __pending_callbacks__.slice();
                __pending_callbacks__.length = 0;
                for (var i = 0; i < batch.length; i++) {
                    var cb = batch[i];
                    if (typeof cb.callback === 'function') {
                        try { cb.callback(); } catch(e) {}
                    }
                    if (cb.repeating) {
                        __pending_callbacks__.push({
                            timer_id: cb.timer_id,
                            callback: cb.callback,
                            delay_ms: cb.delay_ms,
                            repeating: true
                        });
                    }
                }
                return batch.length;
            };
        })();
        "#;

        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup timers: {:?}", e))?;
        Ok(())
    }

    /// Drain pending timer callbacks from the JS side and return them as Rust-side metadata.
    pub fn drain_callbacks(&mut self) -> Result<Vec<PendingCallback>> {
        let code = r#"
        (function() {
            if (typeof __drainTimerCallbacks__ === 'undefined') return [];
            return __drainTimerCallbacks__();
        })()
        "#;

        let callbacks = self.context.with(|ctx| -> rquickjs::Result<Vec<PendingCallback>> {
            let result: Value = ctx.eval(code)?;
            let mut callbacks = Vec::new();
            if let Some(arr) = result.as_object() {
                let len: usize = arr.get::<_, Value>("length")
                    .ok()
                    .and_then(|v| value_to_f64(&v))
                    .unwrap_or(0.0) as usize;

                for i in 0..len {
                    if let Ok(entry) = arr.get::<_, Value>(i as u32) {
                        if let Some(obj) = entry.as_object() {
                            let timer_id = obj.get::<_, Value>("timer_id")
                                .ok()
                                .and_then(|v| value_to_f64(&v))
                                .unwrap_or(0.0) as u32;

                            let delay_ms = obj.get::<_, Value>("delay_ms")
                                .ok()
                                .and_then(|v| value_to_f64(&v))
                                .unwrap_or(0.0) as u32;

                            let repeating = obj.get::<_, Value>("repeating")
                                .ok()
                                .and_then(|v| value_to_bool(&v))
                                .unwrap_or(false);

                            callbacks.push(PendingCallback {
                                timer_id,
                                delay_ms,
                                repeating,
                                fire_at_ms: 0,
                            });
                        }
                    }
                }
            }
            Ok(callbacks)
        }).map_err(|e| anyhow!("Failed to drain callbacks: {:?}", e))?;
        Ok(callbacks)
    }

    /// Drain and execute pending timer callbacks, up to 5 rounds.
    pub fn drain_and_execute_timers(&mut self) -> Result<()> {
        for _round in 0..5u32 {
            let code = r#"
            (function() {
                if (typeof __executeAllTimerCallbacks__ === 'undefined') return 0;
                return __executeAllTimerCallbacks__();
            })()
            "#;

            let count = self.context.with(|ctx| -> rquickjs::Result<u32> {
                let result: Value = ctx.eval(code)?;
                Ok(result.as_float().unwrap_or(0.0) as u32)
            }).map_err(|e| anyhow!("Failed to execute timer callbacks: {:?}", e))?;

            if count == 0 {
                break;
            }
        }
        Ok(())
    }

    /// Setup XMLHttpRequest support with the given HTTP client
    pub fn setup_xhr(&mut self, client: std::sync::Arc<mb_network::client::HttpClient>) -> Result<()> {
        let handle = tokio::runtime::Handle::current();
        xhr::register_xhr(&self.context, client, handle)
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
        let cb = engine.drain_callbacks().unwrap();
        assert_eq!(cb.len(), 0, "All pending callbacks should be drained");
    }

    #[test]
    fn test_cleartimeout() {
        let mut engine = JsEngine::new_with_defaults();
        let id = engine.eval("var tid = setTimeout(function(){}, 100); tid").unwrap();
        engine.eval(&format!("clearTimeout({})", id)).unwrap();
        let cb = engine.drain_callbacks().unwrap();
        assert_eq!(cb.len(), 0, "Cleared timer should not appear in drain");
    }

    #[test]
    fn test_navigator_plugins() {
        let mut engine = JsEngine::new_with_defaults();
        let result = engine.eval("navigator.plugins.length").unwrap();
        assert_eq!(result, "3", "navigator.plugins.length should be 3");
    }

    #[test]
    fn test_navigator_vendor() {
        let mut engine = JsEngine::new_with_defaults();
        let result = engine.eval("navigator.vendor").unwrap();
        assert_eq!(result, "Google Inc.", "navigator.vendor should be 'Google Inc.'");
    }

    #[test]
    fn test_navigator_hardware() {
        let mut engine = JsEngine::new_with_defaults();
        assert_eq!(engine.eval("navigator.deviceMemory").unwrap(), "8");
        assert_eq!(engine.eval("navigator.hardwareConcurrency").unwrap(), "8");
        assert_eq!(engine.eval("navigator.maxTouchPoints").unwrap(), "0");
    }

    #[test]
    fn test_screen() {
        let mut engine = JsEngine::new_with_defaults();
        assert_eq!(engine.eval("screen.width").unwrap(), "1920");
        assert_eq!(engine.eval("screen.height").unwrap(), "1080");
        assert_eq!(engine.eval("screen.colorDepth").unwrap(), "24");
        assert_eq!(engine.eval("screen.orientation.type").unwrap(), "landscape-primary");
    }

    #[test]
    fn test_chrome_object() {
        let mut engine = JsEngine::new_with_defaults();
        assert_eq!(engine.eval("typeof chrome").unwrap(), "object");
        assert_eq!(engine.eval("typeof chrome.runtime").unwrap(), "object");
        assert_eq!(engine.eval("typeof chrome.loadTimes").unwrap(), "function");
        assert_eq!(engine.eval("typeof chrome.csi").unwrap(), "function");
    }

    #[test]
    fn test_performance() {
        let mut engine = JsEngine::new_with_defaults();
        assert_eq!(engine.eval("typeof performance.now").unwrap(), "function");
        assert_eq!(engine.eval("typeof performance.timeOrigin").unwrap(), "number");
        assert_eq!(engine.eval("typeof performance.timing").unwrap(), "object");
    }

    #[test]
    fn test_misc_apis() {
        let mut engine = JsEngine::new_with_defaults();
        assert_eq!(engine.eval("typeof atob").unwrap(), "function");
        assert_eq!(engine.eval("typeof btoa").unwrap(), "function");
        assert_eq!(engine.eval("typeof matchMedia").unwrap(), "function");
        let result = engine.eval("matchMedia('(min-width: 800px)').matches").unwrap();
        assert_eq!(result, "false");
    }
}
