//! JavaScript engine implementation using boa_engine
//!
//! Provides a JavaScript runtime with basic Web API bindings:
//! - console.log
//! - navigator.userAgent
//! - location.href
//! - document (DOM bridge via bind_dom)

use anyhow::{Result, anyhow};
use boa_engine::{Context, Source, JsValue, JsString};
use mb_dom::tree::DomTree;
use mb_dom::node::{NodeKind, NodeId};
use slotmap::Key;

/// JavaScript engine wrapper around boa_engine
pub struct JsEngine {
    context: Context,
}

impl JsEngine {
    /// Create a new JS engine instance
    pub fn new() -> Self {
        let context = Context::default();
        Self { context }
    }

    /// Create a new engine with console, navigator, and location set up
    pub fn new_with_defaults() -> Self {
        let mut engine = Self::new();
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

    /// Setup navigator object
    pub fn setup_navigator(&mut self) -> Result<()> {
        let code = r#"
        var navigator = {
            userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36',
            platform: 'MacIntel',
            language: 'en-US',
            languages: ['en-US', 'en'],
            cookieEnabled: true,
            onLine: true
        };
        "#;

        self.context.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| anyhow!("Failed to setup navigator: {:?}", e))?;
        Ok(())
    }

    /// Setup location object
    pub fn setup_location(&mut self, url: &str) -> Result<()> {
        let (protocol, host, pathname, search, hash) = Self::parse_url_components(url);
        let origin = if host.is_empty() { String::new() } else { format!("{}{}", protocol, host) };

        let code = format!(r#"
        var location = {{
            href: {},
            protocol: {},
            host: {},
            hostname: {},
            pathname: {},
            search: {},
            hash: {},
            origin: {}
        }};
        "#, format_args!("{:?}", url),
            format_args!("{:?}", protocol),
            format_args!("{:?}", host),
            format_args!("{:?}", host),
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
        code.push_str(&format!("var document = {{\n\
            _title: {title},\n\
            nodeType: 9,\n\
            nodeName: \"#document\",\n\
            get title() {{ return this._title; }},\n\
            set title(v) {{ this._title = String(v); }},\n\
            get head() {{ return __dom_node_name__(__dom_head_id__); }},\n\
            get body() {{ return __dom_node_name__(__dom_body_id__); }},\n\
            cookie: \"\",\n\
            getElementById: function(id) {{\n\
                var nid = __dom_by_id__[id];\n\
                if (nid === undefined || nid === null) return null;\n\
                return __dom_elements__[nid] || null;\n\
            }},\n\
            getElementsByTagName: function(tag) {{\n\
                tag = tag.toUpperCase();\n\
                var result = [];\n\
                for (var k in __dom_elements__) {{\n\
                    var e = __dom_elements__[k];\n\
                    if (e && e.tagName === tag) result.push(e);\n\
                }}\n\
                return result;\n\
            }},\n\
            getElementsByClassName: function(cls) {{\n\
                var result = [];\n\
                for (var k in __dom_elements__) {{\n\
                    var e = __dom_elements__[k];\n\
                    if (e && e.className && e.className.split(' ').indexOf(cls) >= 0) result.push(e);\n\
                }}\n\
                return result;\n\
            }},\n\
            querySelector: function(sel) {{\n\
                sel = sel.trim();\n\
                if (sel.charAt(0) === '#') {{\n\
                    return this.getElementById(sel.substring(1));\n\
                }}\n\
                if (sel.charAt(0) === '.') {{\n\
                    var cls = sel.substring(1);\n\
                    var arr = this.getElementsByClassName(cls);\n\
                    return arr.length > 0 ? arr[0] : null;\n\
                }}\n\
                var arr = this.getElementsByTagName(sel);\n\
                return arr.length > 0 ? arr[0] : null;\n\
            }},\n\
            querySelectorAll: function(sel) {{\n\
                sel = sel.trim();\n\
                if (sel.charAt(0) === '#') {{\n\
                    var e = this.getElementById(sel.substring(1));\n\
                    return e ? [e] : [];\n\
                }}\n\
                if (sel.charAt(0) === '.') {{\n\
                    return this.getElementsByClassName(sel.substring(1));\n\
                }}\n\
                return this.getElementsByTagName(sel);\n\
            }},\n\
            createElement: function(tag) {{\n\
                var newId = 'created_' + (++document._createCounter);\n\
                var el = {{\n\
                    tagName: tag.toUpperCase(),\n\
                    id: \"\",\n\
                    className: \"\",\n\
                    textContent: \"\",\n\
                    innerHTML: \"\",\n\
                    getAttribute: function(n) {{ return this._attrs[n] || null; }},\n\
                    setAttribute: function(n, v) {{ this._attrs[n] = String(v); }},\n\
                    hasAttribute: function(n) {{ return n in this._attrs; }},\n\
                    style: {{}},\n\
                    _attrs: {{}},\n\
                    children: [],\n\
                    childNodes: [],\n\
                    parentNode: null\n\
                }};\n\
                __dom_elements__[newId] = el;\n\
                return el;\n\
            }},\n\
            _createCounter: 0\n\
            }};\n",
            title = title,
        ));

        // Fault-tolerant: if boa_engine cannot parse the generated JS
        // (e.g. getter/setter syntax not supported), log a warning and
        // continue instead of crashing the whole page load.
        if let Err(e) = self.context.eval(Source::from_bytes(code.as_bytes())) {
            eprintln!("[bind_dom] JS eval error (continuing anyway): {:?}", e);
        }

        Ok(())
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
}
