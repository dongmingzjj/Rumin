//! Page — the core unit that wires network, HTML parsing, DOM, and JS together

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};

use mb_dom::tree::DomTree;
use mb_dom::node::{NodeId, NodeKind};
use mb_html::parser::HtmlParser;
use mb_js::{JsEngine, Mutation, MutationKind};
use mb_network::client::HttpClient;
use mb_network::cookie::CookieJar;
use mb_network::HttpRequest;

/// Resolve a script URL relative to a base URL.
fn resolve_script_url(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }
    if relative.starts_with("//") {
        return format!("https:{}", relative);
    }
    if relative.starts_with('/') {
        if let Ok(base_url) = url::Url::parse(base) {
            return format!(
                "{}://{}{}",
                base_url.scheme(),
                base_url.host_str().unwrap_or(""),
                relative
            );
        }
    }
    if let Ok(base_url) = url::Url::parse(base) {
        if let Ok(resolved) = base_url.join(relative) {
            return resolved.to_string();
        }
    }
    relative.to_string()
}

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
        let request = HttpRequest::get(url)
            .header("sec-fetch-user", "?1")
            .header("upgrade-insecure-requests", "1");
        self.navigate_with_request(request).await
    }

    /// 使用自定义 HttpRequest 导航（支持自定义 headers）
    pub async fn navigate_with_request(&mut self, request: HttpRequest) -> Result<()> {
        // 从 request 中获取 URL
        let url = request.url.clone();
        self.url = url.clone();

        // 1. Fetch the page（使用传入的请求，支持自定义 headers）
        let response = self.client.execute(request).await
            .context("HTTP request failed")?;

        // Store the status code — 404 etc. are valid responses, not errors
        self.status = response.status_code();

        let html = response.text()
            .context("Failed to decode response body")?;

        self.html = html.clone();

        // 2. Parse HTML into DOM
        self.dom = HtmlParser::parse(&html, &url)
            .context("HTML parsing failed")?;

        // 3. Update document URL
        if let Some(doc_data) = self.dom.get_node_mut(self.dom.document_node).kind.as_document_mut() {
            doc_data.url = url.to_string();
        }

        // 4. Set up JS engine with the page context
        // Clean up old XHR instances to prevent memory leaks
        mb_js::xhr::clear_xhr_instances();
        self.js = JsEngine::new_with_defaults();

        // Apply emulation preset's user-agent and platform to navigator
        let preset = self.client.config().emulation;
        self.js.setup_navigator_with_overrides(preset.user_agent(), preset.platform())?;
        self.js.setup_anti_detect()?;
        self.js.setup_stealth()?;

        self.js.setup_location(&url)?;

        // 5. Inject DOM tree into JS environment
        self.js.bind_dom(&self.dom)?;
        self.js.setup_canvas_after_dom()?;

        // 5b. Setup XMLHttpRequest support
        if let Err(e) = self.js.setup_xhr(Arc::clone(&self.client)) {
            tracing::warn!("Failed to setup XHR: {}", e);
        }

        // 5b2. Setup fetch API support
        if let Err(e) = self.js.setup_fetch(Arc::clone(&self.client)) {
            tracing::warn!("Failed to setup fetch API: {}", e);
        }

        // 5c. Setup WebSocket support
        if let Err(e) = self.js.setup_websocket() {
            tracing::warn!("Failed to setup WebSocket: {}", e);
        }

        // 5c. Setup CookieJar for document.cookie (shared reference)
        if let Err(e) = self.js.set_cookie_jar(Arc::clone(&self.cookies), &self.url) {
            tracing::warn!("Failed to setup cookies: {}", e);
        }

        // 6. Collect and execute scripts (inline + external) in document order
        // 修复：先并发下载所有外部脚本，再按文档顺序依次执行
        // 这确保了外部脚本（如定义全局变量的）在后续 inline 脚本之前执行
        let scripts = HtmlParser::collect_scripts(&self.dom);

        // 第一步：收集所有外部脚本 URL 并发下载
        let mut external_urls: Vec<String> = Vec::new();
        for script in &scripts {
            if script.inline_content.is_none() {
                if let Some(src) = &script.src {
                    let script_url = resolve_script_url(&self.url, src);
                    external_urls.push(script_url);
                }
            }
        }

        // 并发下载所有外部脚本，建立 URL->代码 的映射
        let mut external_code: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if !external_urls.is_empty() {
            let downloads: Vec<_> = external_urls
                .iter()
                .map(|url| self.client.get(url))
                .collect();
            let results = futures_util::future::join_all(downloads).await;

            for (script_url, result) in external_urls.iter().zip(results.into_iter()) {
                match result {
                    Ok(response) => {
                        if response.is_success() {
                            match response.text() {
                                Ok(code) => {
                                    external_code.insert(script_url.clone(), code);
                                }
                                Err(e) => tracing::warn!("Failed to read script body from {}: {}", script_url, e),
                            }
                        } else {
                            tracing::warn!("Script fetch failed: HTTP {} for {}", response.status_code(), script_url);
                        }
                    }
                    Err(e) => tracing::warn!("Script download error for {}: {}", script_url, e),
                }
            }
        }

        // 第二步：按文档顺序依次执行所有脚本（保持原始顺序）
        for script in &scripts {
            if let Some(inline) = &script.inline_content {
                tracing::debug!("Executing inline script ({} bytes)", inline.len());
                if let Err(e) = self.js.eval(inline) {
                    tracing::warn!("Inline script error: {}", e);
                }
            } else if let Some(src) = &script.src {
                let script_url = resolve_script_url(&self.url, src);
                if let Some(code) = external_code.get(&script_url) {
                    tracing::debug!("Executing external script ({} bytes) from {}", code.len(), script_url);
                    if let Err(e) = self.js.eval(code) {
                        tracing::warn!("External script error: {}", e);
                    }
                }
            }
        }

        // Trigger DOMContentLoaded and load events for SPA framework initialization
        if let Err(e) = self.js.eval("document.dispatchEvent(new Event('DOMContentLoaded'))") {
            tracing::debug!("DOMContentLoaded dispatch: {}", e);
        }
        if let Err(e) = self.js.eval("window.dispatchEvent(new Event('load'))") {
            tracing::debug!("load event dispatch: {}", e);
        }

        // Execute any dynamically queued scripts (from event handlers, etc.)
        self.execute_dynamic_scripts();

        // Drain timer callbacks (setInterval/setTimeout) — needed for WAF challenges
        // that use setInterval to wait for async operations
        if let Err(e) = self.js.drain_and_execute_timers() {
            tracing::debug!("Timer drain: {}", e);
        }

        // WAF challenge reload loop — if scripts called location.reload(), re-navigate
        const MAX_RELOAD_ROUNDS: usize = 5;
        for reload_round in 0..MAX_RELOAD_ROUNDS {
            let should_reload = self.js.eval(
                "typeof __location_reload__ !== 'undefined' && __location_reload__"
            ).unwrap_or_default();

            if should_reload != "true" {
                break;
            }

            // Get target URL (may be original or new URL)
            let target = self.js.eval(
                "(typeof __location_href_target__ !== 'undefined' && __location_href_target__) ? __location_href_target__ : ''"
            ).unwrap_or_default();

            let reload_url = if !target.is_empty() && target != "undefined" && target != "null" {
                target
            } else {
                self.url.clone()
            };

            tracing::info!("WAF challenge reload #{}: navigating to {}", reload_round + 1, reload_url);

            // Re-fetch and re-parse (cookie jar is shared via Arc<Mutex>)
            // WAF 重载也是顶层导航，添加同样的浏览器原生 headers
            let request = HttpRequest::get(&reload_url)
                .header("sec-fetch-user", "?1")
                .header("upgrade-insecure-requests", "1");
            let response = self.client.execute(request).await
                .context("WAF reload HTTP request failed")?;
            self.status = response.status_code();
            let html = response.text()
                .context("Failed to decode WAF reload body")?;
            self.html = html.clone();
            self.url = reload_url.clone();

            // Re-parse DOM
            self.dom = HtmlParser::parse(&html, &self.url)
                .context("WAF reload HTML parsing failed")?;

            // Update document URL
            if let Some(doc_data) = self.dom.get_node_mut(self.dom.document_node).kind.as_document_mut() {
                doc_data.url = self.url.clone();
            }

            // Rebuild JS engine (cookie jar persists via Arc<Mutex>)
            mb_js::xhr::clear_xhr_instances();
            self.js = JsEngine::new_with_defaults();

            let preset = self.client.config().emulation;
            self.js.setup_navigator_with_overrides(preset.user_agent(), preset.platform())?;
            self.js.setup_anti_detect()?;
            self.js.setup_stealth()?;
            self.js.setup_location(&self.url)?;
            self.js.bind_dom(&self.dom)?;
            self.js.setup_canvas_after_dom()?;

            if let Err(e) = self.js.setup_xhr(Arc::clone(&self.client)) {
                tracing::warn!("Failed to setup XHR: {}", e);
            }
            if let Err(e) = self.js.setup_fetch(Arc::clone(&self.client)) {
                tracing::warn!("Failed to setup fetch API: {}", e);
            }
            if let Err(e) = self.js.setup_websocket() {
                tracing::warn!("Failed to setup WebSocket: {}", e);
            }
            if let Err(e) = self.js.set_cookie_jar(Arc::clone(&self.cookies), &self.url) {
                tracing::warn!("Failed to setup cookies: {}", e);
            }

            // Execute scripts again（按文档顺序执行，同初始加载的修复）
            let scripts = HtmlParser::collect_scripts(&self.dom);

            // 先并发下载所有外部脚本
            let mut external_urls: Vec<String> = Vec::new();
            for script in &scripts {
                if script.inline_content.is_none() {
                    if let Some(src) = &script.src {
                        let script_url = resolve_script_url(&self.url, src);
                        external_urls.push(script_url);
                    }
                }
            }

            let mut external_code: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            if !external_urls.is_empty() {
                let downloads: Vec<_> = external_urls
                    .iter()
                    .map(|url| self.client.get(url))
                    .collect();
                let results = futures_util::future::join_all(downloads).await;
                for (script_url, result) in external_urls.iter().zip(results.into_iter()) {
                    if let Ok(response) = result {
                        if response.is_success() {
                            if let Ok(code) = response.text() {
                                external_code.insert(script_url.clone(), code);
                            }
                        }
                    }
                }
            }

            // 按文档顺序依次执行所有脚本
            for script in &scripts {
                if let Some(inline) = &script.inline_content {
                    if let Err(e) = self.js.eval(inline) {
                        tracing::warn!("WAF reload inline script error: {}", e);
                    }
                } else if let Some(src) = &script.src {
                    let script_url = resolve_script_url(&self.url, src);
                    if let Some(code) = external_code.get(&script_url) {
                        if let Err(e) = self.js.eval(code) {
                            tracing::warn!("WAF reload external script error: {}", e);
                        }
                    }
                }
            }

            // Re-trigger events
            if let Err(e) = self.js.eval("document.dispatchEvent(new Event('DOMContentLoaded'))") {
                tracing::debug!("WAF reload DOMContentLoaded: {}", e);
            }
            if let Err(e) = self.js.eval("window.dispatchEvent(new Event('load'))") {
                tracing::debug!("WAF reload load: {}", e);
            }
            self.execute_dynamic_scripts();
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

        // Execute any dynamically queued scripts
        self.execute_dynamic_scripts();

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

    /// Execute dynamically queued scripts (from appendChild/insertBefore of <script> tags).
    /// Downloads external scripts and executes inline scripts. Loops until no more are queued.
    fn execute_dynamic_scripts(&mut self) {
        const MAX_ROUNDS: usize = 10;
        for _round in 0..MAX_ROUNDS {
            let urls = self.js.drain_dynamic_scripts();
            if urls.is_empty() {
                break;
            }
            for raw_url in urls {
                let script_url = resolve_script_url(&self.url, &raw_url);
                tracing::debug!("Loading dynamic script from {}", script_url);

                let client = Arc::clone(&self.client);
                let url_clone = script_url.clone();
                let result = tokio::task::block_in_place(|| {
                    let handle = tokio::runtime::Handle::current();
                    handle.block_on(async move { client.get(&url_clone).await })
                });

                match result {
                    Ok(response) => {
                        if response.is_success() {
                            match response.text() {
                                Ok(code) => {
                                    tracing::debug!(
                                        "Executing dynamic script ({} bytes) from {}",
                                        code.len(),
                                        script_url
                                    );
                                    if let Err(e) = self.js.eval(&code) {
                                        tracing::warn!(
                                            "Dynamic script error from {}: {}",
                                            script_url,
                                            e
                                        );
                                    }
                                    // Apply mutations from dynamic script
                                    let mutations = self.js.drain_mutations();
                                    for m in mutations {
                                        if let Err(e) = self.apply_mutation(m) {
                                            tracing::warn!(
                                                "Failed to apply mutation: {}",
                                                e
                                            );
                                        }
                                    }
                                }
                                Err(e) => tracing::warn!(
                                    "Failed to read dynamic script body from {}: {}",
                                    script_url,
                                    e
                                ),
                            }
                        } else {
                            tracing::warn!(
                                "Dynamic script fetch failed: HTTP {} for {}",
                                response.status_code(),
                                script_url
                            );
                        }
                    }
                    Err(e) => tracing::warn!(
                        "Dynamic script download error for {}: {}",
                        script_url,
                        e
                    ),
                }
            }
        }
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
