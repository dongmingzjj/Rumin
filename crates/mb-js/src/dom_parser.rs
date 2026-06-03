//! DOMParser implementation — bridges to HtmlParser

use super::*;
use mb_html::parser::HtmlParser;
use slotmap::Key;

impl JsEngine {
    /// Setup DOMParser.parseFromString()
    pub fn setup_dom_parser(&mut self) -> Result<()> {
        // Register a native function that parses HTML and returns serialized DOM
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let globals = ctx.globals();

            let parse_fn = Function::new(ctx.clone(), |args: rquickjs::function::Rest<Value>| -> rquickjs::Result<String> {
                let html = args.get(0)
                    .and_then(|v| v.as_string().and_then(|s| s.to_string().ok()))
                    .unwrap_or_default();

                let tree = HtmlParser::parse(&html, "about:blank").unwrap_or_default();

                // Serialize the document to a simple JSON structure
                let doc_id = tree.document_node;
                let mut result = String::from("{");
                result.push_str("\"nodeType\":9,\"nodeName\":\"#document\",");
                result.push_str("\"childNodes\":[");

                // Serialize children recursively
                fn serialize_node(tree: &mb_dom::tree::DomTree, node_id: mb_dom::node::NodeId) -> String {
                    let node = tree.get_node(node_id);
                    let id = node_id.data().as_ffi();

                    match &node.kind {
                        mb_dom::node::NodeKind::Element(e) => {
                            let mut out = format!(
                                "{{\"nodeType\":1,\"nodeName\":\"{}\",\"tagName\":\"{}\",\"_nodeId\":{},\"attributes\":{{}},\"childNodes\":[",
                                e.tag_name.to_uppercase(), e.tag_name.to_uppercase(), id
                            );
                            let mut child = node.first_child;
                            let mut first = true;
                            while let Some(cid) = child {
                                if !first { out.push(','); }
                                first = false;
                                let child_node = tree.get_node(cid);
                                out.push_str(&serialize_node(tree, cid));
                                child = child_node.next_sibling;
                            }
                            out.push_str("]}");
                            out
                        }
                        mb_dom::node::NodeKind::Text(t) => {
                            format!(
                                "{{\"nodeType\":3,\"nodeName\":\"#text\",\"textContent\":{},\"_nodeId\":{}}}",
                                serde_json::json!(t.data), id
                            )
                        }
                        mb_dom::node::NodeKind::Document(_) => {
                            format!("{{\"nodeType\":9,\"nodeName\":\"#document\",\"_nodeId\":{}}}", id)
                        }
                        mb_dom::node::NodeKind::Comment(_) => {
                            format!("{{\"nodeType\":8,\"nodeName\":\"#comment\",\"_nodeId\":{}}}", id)
                        }
                    }
                }

                // Serialize document children
                let doc_node = tree.get_node(doc_id);
                let mut child = doc_node.first_child;
                let mut first = true;
                while let Some(cid) = child {
                    if !first { result.push(','); }
                    first = false;
                    let child_node = tree.get_node(cid);
                    result.push_str(&serialize_node(&tree, cid));
                    child = child_node.next_sibling;
                }
                result.push_str("]}");

                Ok(result)
            })?;
            globals.set("__dom_parser_parse__", parse_fn)?;

            let _: Value = ctx.eval(r#"
            function DOMParser() {}
            DOMParser.prototype.parseFromString = function(html, mimeType) {
                var json = __dom_parser_parse__(html || '');
                var data = JSON.parse(json);
                // Convert the raw JSON into a Document-like object
                function _buildNode(raw) {
                    if (!raw) return null;
                    var node = {
                        nodeType: raw.nodeType || 1,
                        nodeName: raw.nodeName || '',
                        tagName: raw.tagName || null,
                        _nodeId: raw._nodeId || 0,
                        attributes: raw.attributes || {},
                        textContent: raw.textContent || '',
                        childNodes: [],
                        children: [],
                        parentNode: null,
                        ownerDocument: data
                    };
                    if (raw.childNodes) {
                        for (var i = 0; i < raw.childNodes.length; i++) {
                            var child = _buildNode(raw.childNodes[i]);
                            if (child) {
                                child.parentNode = node;
                                node.childNodes.push(child);
                                if (child.nodeType === 1) node.children.push(child);
                            }
                        }
                    }
                    node.firstChild = node.childNodes[0] || null;
                    node.lastChild = node.childNodes.length > 0 ? node.childNodes[node.childNodes.length - 1] : null;
                    node.getElementsByTagName = function(tag) {
                        var results = [];
                        var search = (tag || '').toUpperCase();
                        function walk(n) {
                            if (n.tagName && (search === '*' || n.tagName === search)) results.push(n);
                            for (var i = 0; i < n.childNodes.length; i++) walk(n.childNodes[i]);
                        }
                        walk(node);
                        return results;
                    };
                    node.querySelector = function() { return null; };
                    node.querySelectorAll = function() { return []; };
                    node.getElementsByClassName = function() { return []; };
                    return node;
                }
                var doc = {
                    nodeType: 9,
                    nodeName: '#document',
                    childNodes: [],
                    children: [],
                    documentElement: null,
                    body: null,
                    head: null,
                    querySelector: function() { return null; },
                    querySelectorAll: function() { return []; },
                    getElementById: function(id) { return null; },
                    getElementsByClassName: function() { return []; },
                    getElementsByTagName: function(tag) {
                        if (doc.documentElement) return doc.documentElement.getElementsByTagName(tag);
                        return [];
                    },
                    createElement: function(tag) {
                        return { nodeType: 1, nodeName: tag.toUpperCase(), tagName: tag.toUpperCase(),
                                 _nodeId: 0, attributes: {}, childNodes: [], children: [], textContent: '',
                                 appendChild: function(c) { this.childNodes.push(c); if(c.nodeType===1) this.children.push(c); c.parentNode=this; return c; },
                                 setAttribute: function(n,v) { this.attributes[n]=v; },
                                 getAttribute: function(n) { return this.attributes[n]||null; },
                                 removeAttribute: function(n) { delete this.attributes[n]; },
                                 hasAttribute: function(n) { return this.attributes.hasOwnProperty(n); }
                        };
                    },
                    createTextNode: function(text) {
                        return { nodeType: 3, nodeName: '#text', textContent: text, _nodeId: 0 };
                    }
                };
                if (data.childNodes) {
                    for (var i = 0; i < data.childNodes.length; i++) {
                        var child = _buildNode(data.childNodes[i]);
                        if (child) {
                            child.parentNode = doc;
                            doc.childNodes.push(child);
                            if (child.nodeType === 1) doc.children.push(child);
                            if (child.tagName === 'HTML') doc.documentElement = child;
                        }
                    }
                }
                if (doc.documentElement) {
                    // Find head and body
                    for (var i = 0; i < doc.documentElement.childNodes.length; i++) {
                        var n = doc.documentElement.childNodes[i];
                        if (n.tagName === 'HEAD') doc.head = n;
                        if (n.tagName === 'BODY') doc.body = n;
                    }
                }
                if (!doc.body) doc.body = doc.createElement('body');
                if (!doc.head) doc.head = doc.createElement('head');
                return doc;
            };
            globalThis.DOMParser = DOMParser;
            globalThis.window = globalThis;
            "#)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup DOMParser: {:?}", e))?;
        Ok(())
    }
}
