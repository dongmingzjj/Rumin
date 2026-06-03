//! JavaScript engine implementation using rquickjs (QuickJS-NG)
//!
//! Provides a JavaScript runtime with basic Web API bindings:
//! - console.log
//! - navigator.userAgent
//! - location.href
//! - document (DOM bridge via bind_dom)
//! - setTimeout / setInterval / clearTimeout / clearInterval (MVP)

use anyhow::{Result, anyhow};
use rquickjs::{Context as QContext, Runtime, Value, Function};
use rquickjs::function::Rest;
use rquickjs::promise::PromiseState;
use mb_dom::tree::DomTree;
use mb_dom::node::{NodeKind, NodeId};
use mb_dom::selector::SelectorEngine;

use std::cell::RefCell;
use std::collections::HashMap;
use mb_network::cookie::CookieJar;
use std::sync::{Arc, Mutex};

pub mod dom_bridge;
pub mod web_apis;
pub mod timers;
pub mod cookies;
pub mod xhr;
pub mod event;
pub mod animation;
pub mod mutation_observer;
pub mod intersection_observer;
pub mod anti_detect;
pub mod intl;
pub mod indexed_db;
pub mod websocket;
pub mod computed_style;
pub mod canvas;
pub mod text_encoding;

pub use dom_bridge::{Mutation, MutationKind};
pub use timers::PendingCallback;

/// JavaScript engine wrapper around rquickjs
pub struct JsEngine {
    pub(crate) runtime: Runtime,
    pub(crate) context: QContext,
    #[allow(dead_code)]
    pub(crate) pending_callbacks: Vec<PendingCallback>,
    /// Mutations recorded by JS that need to be applied to the Rust DomTree.
    pub mutations: Vec<Mutation>,
}

/// Convert a rquickjs Value to a String representation
pub(crate) fn js_value_to_string(val: &Value) -> String {
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
pub(crate) fn value_to_f64(val: &Value) -> Option<f64> {
    val.as_float()
}

/// Extract a u64 from a Value
pub(crate) fn value_to_u64(val: &Value) -> Option<u64> {
    if val.is_int() {
        val.as_int().map(|i| i as u64)
    } else {
        val.as_float().and_then(|f| {
            if f.is_finite() && f >= 0.0 && f <= u64::MAX as f64 {
                Some(f as u64)
            } else {
                None
            }
        })
    }
}

/// Extract a string from a Value
pub(crate) fn value_to_string(val: &Value) -> Option<String> {
    if val.is_string() {
        val.as_string().and_then(|s| s.to_string().ok())
    } else {
        None
    }
}

/// Extract a boolean from a Value
pub(crate) fn value_to_bool(val: &Value) -> Option<bool> {
    val.as_bool()
}

/// Split JavaScript code at the last top-level semicolon.
/// Returns (setup_code, last_expression).
///
/// If there's no top-level semicolon, returns (None, code).
/// Tracks string literals, template literals, and bracket depth to avoid
/// splitting inside strings or nested expressions.
fn split_last_expression(code: &str) -> (Option<String>, String) {
    let bytes = code.as_bytes();
    let len = bytes.len();
    let mut last_semi: Option<usize> = None;
    let mut depth: i32 = 0; // paren/bracket/brace depth
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_template = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Handle comments
        if in_line_comment {
            if b == b'\n' { in_line_comment = false; }
            i += 1; continue;
        }
        if in_block_comment {
            if b == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                in_block_comment = false; i += 2; continue;
            }
            i += 1; continue;
        }

        // Handle string escapes
        if in_single_quote {
            if b == b'\\' { i += 2; continue; } // skip escaped char
            if b == b'\'' { in_single_quote = false; }
            i += 1; continue;
        }
        if in_double_quote {
            if b == b'\\' { i += 2; continue; }
            if b == b'"' { in_double_quote = false; }
            i += 1; continue;
        }
        if in_template {
            if b == b'\\' { i += 2; continue; }
            if b == b'`' { in_template = false; }
            i += 1; continue;
        }

        // Start of strings/comments
        if b == b'\'' { in_single_quote = true; i += 1; continue; }
        if b == b'"' { in_double_quote = true; i += 1; continue; }
        if b == b'`' { in_template = true; i += 1; continue; }
        if b == b'/' && i + 1 < len {
            if bytes[i + 1] == b'/' { in_line_comment = true; i += 2; continue; }
            if bytes[i + 1] == b'*' { in_block_comment = true; i += 2; continue; }
        }

        // Track bracket depth
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b';' if depth == 0 => last_semi = Some(i),
            _ => {}
        }
        i += 1;
    }

    match last_semi {
        Some(pos) => {
            let setup = code[..pos].trim().to_string();
            let last = code[pos + 1..].trim().to_string();
            if setup.is_empty() {
                (None, last)
            } else {
                (Some(setup), last)
            }
        }
        None => (None, code.to_string()),
    }
}

/// Extract detailed exception info from the JS context after an eval error.
/// Tries to get message, stack, fileName, lineNumber from the exception object.
fn extract_js_exception(ctx: &rquickjs::Ctx<'_>) -> String {
    let exc = ctx.catch();
    if let Some(obj) = exc.as_object() {
        let msg: String = obj
            .get("message")
            .ok()
            .and_then(|v: Value| v.as_string().and_then(|s| s.to_string().ok()))
            .unwrap_or_default();
        let stack: String = obj
            .get("stack")
            .ok()
            .and_then(|v: Value| v.as_string().and_then(|s| s.to_string().ok()))
            .unwrap_or_default();
        let file: String = obj
            .get("fileName")
            .ok()
            .and_then(|v: Value| v.as_string().and_then(|s| s.to_string().ok()))
            .unwrap_or_default();
        let line: i32 = obj
            .get("lineNumber")
            .ok()
            .and_then(|v: Value| v.as_int())
            .unwrap_or(0);

        if !stack.is_empty() {
            stack
        } else if !msg.is_empty() {
            if !file.is_empty() && line > 0 {
                format!("{} ({}:{})", msg, file, line)
            } else {
                msg
            }
        } else {
            format!("{:?}", exc)
        }
    } else {
        format!("{:?}", exc)
    }
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
        self.setup_intl()?;
        self.setup_anti_detect()?;
        self.setup_location("about:blank")?;
        self.setup_screen()?;
        self.setup_chrome()?;
        self.setup_performance()?;
        self.setup_misc()?;
        self.setup_events()?;
        self.setup_animation_frames()?;
        self.setup_intersection_observer()?;
        self.setup_mutation_observer()?;
        self.setup_computed_style()?;
        self.setup_canvas()?;
        self.setup_indexed_db()?;
        self.setup_text_encoding()?;
        Ok(())
    }

    /// Evaluate JavaScript code and return the result as a string.
    ///
    /// If the return value is a Promise, drains the microtask queue until the
    /// Promise settles, then returns the resolved value (or error info for
    /// rejected Promises). This ensures that async patterns like:
    ///
    /// ```ignore
    /// Promise.resolve(42).then(v => v * 2)  // returns "84"
    /// (async function() { return 42; })()   // returns "42"
    /// ```
    ///
    /// work correctly from Rust's perspective.
    pub fn eval(&mut self, code: &str) -> Result<String> {
        // Split code into setup + last expression at the last top-level semicolon.
        // This allows us to drain microtasks between setup and result evaluation.
        // Example: "var r; Promise.resolve(42).then(v => r = v); r"
        //   → setup: "var r; Promise.resolve(42).then(v => r = v)"
        //   → last:  "r"
        let (setup_code, last_code) = split_last_expression(code);

        let js_error_detail: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
        let result_str = self.context.with(|ctx| -> rquickjs::Result<String> {
            // Phase 0: Execute setup code (everything except last expression)
            if let Some(ref setup) = setup_code {
                if let Err(_) = ctx.eval::<(), _>(setup.as_str()) {
                    let detail = extract_js_exception(&ctx);
                    *js_error_detail.borrow_mut() = Some(detail);
                    return Err(rquickjs::Error::Exception);
                }
            }

            // Phase 1: Execute the last expression
            let val: Value = match ctx.eval(last_code.as_str()) {
                Ok(v) => v,
                Err(_) => {
                    let detail = extract_js_exception(&ctx);
                    *js_error_detail.borrow_mut() = Some(detail);
                    return Err(rquickjs::Error::Exception);
                }
            };

            // Phase 1.5: Drain all pending microtasks (multiple rounds)
            for _ in 0..100 {
                if !ctx.execute_pending_job() {
                    break;
                }
            }
            if ctx.has_exception() {
                ctx.catch(); // swallow microtask errors
            }

            // Phase 2: If the return value is a Promise, drain microtasks
            // until it settles, then return the resolved value.
            if let Some(promise) = val.as_promise() {
                for _ in 0..1000 {
                    match promise.state() {
                        PromiseState::Pending => {
                            if !ctx.execute_pending_job() {
                                if ctx.has_exception() {
                                    let err = ctx.catch();
                                    return Ok(format!("Error: {}", js_value_to_string(&err)));
                                }
                                break;
                            }
                        }
                        _ => break,
                    }
                }

                match promise.state() {
                    PromiseState::Resolved => {
                        if let Some(Ok(resolved_val)) = promise.result::<Value>() {
                            // Drain microtasks from resolution
                            for _ in 0..100 {
                                if !ctx.execute_pending_job() { break; }
                            }
                            return Ok(js_value_to_string(&resolved_val));
                        }
                    }
                    PromiseState::Rejected => {
                        for _ in 0..100 {
                            if !ctx.execute_pending_job() { break; }
                        }
                        if ctx.has_exception() {
                            let err = ctx.catch();
                            return Ok(format!("Error: {}", js_value_to_string(&err)));
                        }
                        if let Some(Ok(v)) = promise.result::<Value>() {
                            return Ok(format!("Error: {}", js_value_to_string(&v)));
                        }
                    }
                    _ => {}
                }
            }

            // Phase 3: Drain all microtasks (for side-effect patterns like
            // Promise.resolve(42).then(v => r = v); r)
            for _ in 0..1000 {
                if !ctx.execute_pending_job() { break; }
            }

            // Phase 4: Re-evaluate the last expression to get the post-microtask value.
            // After draining, variables may have been updated by .then() callbacks.
            // This is safe for read-only expressions (variable refs, property access).
            // For expressions with side effects (counter++), this may double-execute,
            // but that's an acceptable trade-off for correctness.
            let final_val: Value = ctx.eval(last_code.as_str())?;
            Ok(js_value_to_string(&final_val))
        }).map_err(|e| {
            if let Some(detail) = js_error_detail.borrow_mut().take() {
                anyhow!("JS evaluation error: {}", detail)
            } else {
                anyhow!("JS evaluation error: {:?}", e)
            }
        })?;

        // Drain any DOM mutations that were queued during eval + microtasks
        if let Err(e) = self.drain_js_mutations() {
            tracing::warn!("Failed to drain JS mutations: {}", e);
        }

        // Drain and execute pending timer callbacks.
        // Timer callbacks may create new Promises (microtasks), so we loop
        // to handle: timer -> Promise -> .then() chains.
        for _round in 0..3u32 {
            let timer_count = self.drain_and_execute_timers_one_round()?;
            if timer_count == 0 { break; }

            // After timers, drain microtasks again
            self.drain_microtasks_outside_ctx();

            // Drain mutations from timer-triggered microtasks
            if let Err(e) = self.drain_js_mutations() {
                tracing::warn!("Failed to drain JS mutations: {}", e);
            }
        }

        Ok(result_str)
    }

    /// Drain microtasks using runtime.execute_pending_job() (outside ctx.with()).
    /// Used for draining microtasks created by timer callbacks.
    fn drain_microtasks_outside_ctx(&mut self) {
        let mut rounds = 0;
        loop {
            match self.runtime.execute_pending_job() {
                Ok(true) => { rounds += 1; }
                Ok(false) => break,
                Err(e) => {
                    tracing::warn!("Promise job error: {:?}", e);
                    break;
                }
            }
            if rounds > 100 { break; }
        }
    }

    /// Execute one round of timer callbacks. Returns the number of callbacks executed.
    fn drain_and_execute_timers_one_round(&mut self) -> Result<u32> {
        let code = r#"
        (function() {
            var count = 0;
            if (typeof __executeAllTimerCallbacks__ !== 'undefined') {
                count = __executeAllTimerCallbacks__();
            }
            if (typeof __executeAllRAFCallbacks__ !== 'undefined') {
                __executeAllRAFCallbacks__();
            }
            return count;
        })()
        "#;

        let count = self.context.with(|ctx| -> rquickjs::Result<u32> {
            let result: Value = ctx.eval(code)?;
            Ok(result.as_float().unwrap_or(0.0) as u32)
        }).map_err(|e| anyhow!("Failed to execute timer callbacks: {:?}", e))?;

        Ok(count)
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

    /// Drain and return all pending dynamic script URLs.
    /// These are URLs from dynamically inserted <script> elements (via appendChild/insertBefore).
    pub fn drain_dynamic_scripts(&mut self) -> Vec<String> {
        dom_bridge::DYNAMIC_SCRIPTS.with(|scripts| {
            std::mem::take(&mut *scripts.borrow_mut())
        })
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

    /// Setup XMLHttpRequest support with the given HTTP client
    pub fn setup_xhr(&mut self, client: std::sync::Arc<mb_network::client::HttpClient>) -> Result<()> {
        let handle = tokio::runtime::Handle::current();
        xhr::register_xhr(&self.context, client, handle)
    }

    /// Setup WebSocket support with real network connections
    pub fn setup_websocket(&mut self) -> Result<()> {
        let handle = tokio::runtime::Handle::current();
        websocket::register_websocket(&self.context, handle)
    }
}

impl Default for JsEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
