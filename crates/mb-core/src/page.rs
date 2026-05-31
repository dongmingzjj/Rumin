//! Page — the core unit that wires network, HTML parsing, DOM, and JS together

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};

use mb_dom::tree::DomTree;
use mb_html::parser::HtmlParser;
use mb_js::JsEngine;
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
        self.js.eval(code)
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
