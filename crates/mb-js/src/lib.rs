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
    val.as_float().map(|f| f as u64)
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
mod tests;
