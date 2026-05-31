//! Page — the core unit that wires network, HTML parsing, DOM, and JS together

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};

use mb_dom::tree::DomTree;
use mb_dom::node::{NodeId, NodeKind};
use mb_html::parser::HtmlParser;
use mb_js::{JsEngine, Mutation, MutationKind};
use mb_network::client::HttpClient;
use mb_network::cookie::CookieJar;

/// A single page/tab in the browser
pub struct Page {
    /// Current URL
    pub url: String,
    /// HTTP status code from last navigation (0 if not yet navigated)
    pub status: u16,
    /// The DOM tree
    pub dom: DomTree,
    /// JavaScript engine
    pub js: JsEngine,
    /// HTTP client (shared across pages)
    client: Arc<HttpClient>,
    /// Raw HTML source
    pub html: String,
    /// Cookie jar (shared)
    cookies: Arc<Mutex<CookieJar>>,
}

impl Page {
    /// Create a new page with the given HTTP client
    pub fn new(client: Arc<HttpClient>, cookies: Arc<Mutex<CookieJar>>) -> Self {
        Self {
            url: "about:blank".to_string(),
            status: 0,
            dom: DomTree::new(),
            js: JsEngine::new_with_defaults(),
            client,
            html: String::new(),
            cookies,
        }
    }

    /// Navigate to a URL: fetch → parse HTML → build DOM → collect scripts
    pub async fn navigate(&mut self, url: &str) -> Result<()> {
        self.url = url.to_string();

        // 1. Fetch the page
        let response = self.client.get(url).await
            .context("HTTP request failed")?;

        // Store the status code — 404 etc. are valid responses, not errors
        self.status = response.status_code();

        let html = response.text()
            .context("Failed to decode response body")?;

        self.html = html.clone();

        // 2. Parse HTML into DOM
        self.dom = HtmlParser::parse(&html, url)
            .context("HTML parsing failed")?;

        // 3. Update document URL
        if let Some(doc_data) = self.dom.get_node_mut(self.dom.document_node).kind.as_document_mut() {
            doc_data.url = url.to_string();
        }

        // 4. Set up JS engine with the page context
        self.js = JsEngine::new_with_defaults();
        self.js.setup_location(url)?;

        // 5. Inject DOM tree into JS environment
        self.js.bind_dom(&self.dom)?;

        // 5b. Setup XMLHttpRequest support
        if let Err(e) = self.js.setup_xhr(Arc::clone(&self.client)) {
            tracing::warn!("Failed to setup XHR: {}", e);
        }

        // 6. Collect and optionally execute inline scripts
        let scripts = HtmlParser::collect_scripts(&self.dom);
        for script in &scripts {
            if let Some(inline) = &script.inline_content {
                tracing::debug!("Executing inline script ({} bytes)", inline.len());
                // Execute inline scripts but don't fail the page load if they error
                if let Err(e) = self.js.eval(inline) {
                    tracing::warn!("Inline script error: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Execute JavaScript in the page context
    pub fn eval(&mut self, code: &str) -> Result<String> {
        let result = self.js.eval(code)?;

        // Apply mutations from JS back to the DomTree
        let mutations = self.js.drain_mutations();
        for m in mutations {
            if let Err(e) = self.apply_mutation(m) {
                tracing::warn!("Failed to apply mutation: {}", e);
            }
        }

        Ok(result)
    }

    /// Apply a single JS mutation to the DomTree.
    fn apply_mutation(&mut self, m: Mutation) -> Result<()> {
        match m.kind {
            MutationKind::SetAttribute { node_id, name, value } => {
                let nid = NodeId::from(slotmap::KeyData::from_ffi(node_id));
                if self.dom.nodes.contains_key(nid) {
                    if let Some(el) = self.dom.get_node_mut(nid).kind.as_element_mut() {
                        el.attributes.set(name, value);
                    }
                }
            }
            MutationKind::RemoveAttribute { node_id, name } => {
                let nid = NodeId::from(slotmap::KeyData::from_ffi(node_id));
                if self.dom.nodes.contains_key(nid) {
                    if let Some(el) = self.dom.get_node_mut(nid).kind.as_element_mut() {
                        el.attributes.remove(&name);
                    }
                }
            }
            MutationKind::SetTextContent { node_id, text } => {
                let nid = NodeId::from(slotmap::KeyData::from_ffi(node_id));
                if self.dom.nodes.contains_key(nid) {
                    // Remove all existing children
                    let children = self.dom.children(nid);
                    for child in children {
                        self.dom.remove_child(nid, child);
                    }
                    // Add a new text node as child
                    let text_id = self.dom.create_text(&text);
                    self.dom.append_child(nid, text_id);
                }
            }
            MutationKind::SetInnerHTML { node_id, html } => {
                let nid = NodeId::from(slotmap::KeyData::from_ffi(node_id));
                if self.dom.nodes.contains_key(nid) {
                    // Remove all existing children
                    let children = self.dom.children(nid);
                    for child in children {
                        self.dom.remove_child(nid, child);
                    }
                    // Parse the HTML and append as children
                    if !html.is_empty() {
                        let fragment = mb_html::parser::HtmlParser::parse(&format!("<body>{}</body>", html), &self.url)
                            .unwrap_or_else(|_| DomTree::new());
                        // The fragment's body_node children become our children
                        let frag_body = fragment.body_node;
                        let frag_children = fragment.children(frag_body);
                        for fc in frag_children {
                            // Deep copy the fragment node into our DOM
                            let copied = self.deep_copy_node(&fragment, fc);
                            self.dom.append_child(nid, copied);
                        }
                    }
                }
            }
            MutationKind::AppendChild { parent_id, child_tag } => {
                let pid = NodeId::from(slotmap::KeyData::from_ffi(parent_id));
                if self.dom.nodes.contains_key(pid) {
                    let child_id = self.dom.create_element(&child_tag);
                    self.dom.append_child(pid, child_id);
                }
            }
            MutationKind::RemoveChild { parent_id, child_id } => {
                let pid = NodeId::from(slotmap::KeyData::from_ffi(parent_id));
                let cid = NodeId::from(slotmap::KeyData::from_ffi(child_id));
                if self.dom.nodes.contains_key(pid) && self.dom.nodes.contains_key(cid) {
                    self.dom.remove_child(pid, cid);
                }
            }
        }
        Ok(())
    }

    /// Deep copy a node and its subtree from another DomTree into this one.
    fn deep_copy_node(&mut self, src: &DomTree, src_id: NodeId) -> NodeId {
        let src_node = src.get_node(src_id);
        let new_id = match &src_node.kind {
            NodeKind::Element(el) => {
                let new_el = el.clone();
                let id = self.dom.create_node(NodeKind::Element(new_el));
                id
            }
            NodeKind::Text(t) => {
                let id = self.dom.create_text(&t.data);
                id
            }
            NodeKind::Comment(c) => {
                let id = self.dom.create_comment(&c.data);
                id
            }
            NodeKind::Document(_) => return self.dom.create_node(NodeKind::Document(Default::default())),
        };

        // Copy children
        let mut src_child = src_node.first_child;
        while let Some(cid) = src_child {
            let copied_child = self.deep_copy_node(src, cid);
            self.dom.append_child(new_id, copied_child);
            src_child = src.get_node(cid).next_sibling;
        }

        new_id
    }

    /// Get the page title from the DOM
    pub fn dom_title(&self) -> String {
        // Look for <title> in <head>
        if let Some(title_node) = self.dom.query_selector("title") {
            self.dom.text_content(title_node)
        } else {
            String::new()
        }
    }

    /// Get text content of an element matching a CSS selector
    pub fn query_text(&self, selector: &str) -> Option<String> {
        self.dom.query_selector(selector).map(|node| {
            self.dom.text_content(node)
        })
    }

    /// Get the inner HTML of an element matching a CSS selector
    pub fn query_html(&self, _selector: &str) -> Option<String> {
        // TODO: implement innerHTML serialization
        None
    }

    /// Get the raw HTML source
    pub fn source(&self) -> &str {
        &self.html
    }

    /// Get the current URL
    pub fn current_url(&self) -> &str {
        &self.url
    }

    /// Get the DOM tree for direct inspection
    pub fn dom(&self) -> &DomTree {
        &self.dom
    }

    /// Get a mutable reference to the DOM tree
    pub fn dom_mut(&mut self) -> &mut DomTree {
        &mut self.dom
    }
}
