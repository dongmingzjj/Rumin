//! DOM serialization, injection, and mutation tracking.
//!
//! Handles binding a Rust DomTree into the QuickJS JavaScript environment as a
//! `document` object, including native querySelectorAll support.

use super::*;
use slotmap::Key;

// Thread-local storage for DOM element data (used by native querySelectorAll)
// Maps node_id (u64) -> (tag_name_uppercase, class_list, id_attr, parent_id)
thread_local! {
    pub(crate) static DOM_ELEMENTS: RefCell<HashMap<u64, (String, Vec<String>, String, Option<u64>)>> = RefCell::new(HashMap::new());
}

// Thread-local queue for dynamically inserted script URLs (from appendChild/insertBefore)
thread_local! {
    pub(crate) static DYNAMIC_SCRIPTS: RefCell<Vec<String>> = RefCell::new(Vec::new());
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

impl JsEngine {
    /// Serialize a node's text content (recursive, like DOM textContent).
    pub(crate) fn serialize_text_content(tree: &DomTree, node_id: NodeId) -> String {
        let mut buf = String::new();
        Self::collect_text(tree, node_id, &mut buf);
        buf
    }

    pub(crate) fn collect_text(tree: &DomTree, node_id: NodeId, buf: &mut String) {
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

    /// Serialize innerHTML: child nodes as HTML string
    pub(crate) fn serialize_inner_html(tree: &DomTree, node_id: NodeId) -> String {
        let mut buf = String::new();
        let node = tree.get_node(node_id);
        let mut child = node.first_child;
        while let Some(cid) = child {
            Self::serialize_node_html(tree, cid, &mut buf);
            child = tree.get_node(cid).next_sibling;
        }
        buf
    }

    pub(crate) fn serialize_node_html(tree: &DomTree, node_id: NodeId, buf: &mut String) {
        let node = tree.get_node(node_id);
        match &node.kind {
            NodeKind::Element(el) => {
                buf.push('<');
                buf.push_str(&el.tag_name.to_lowercase());
                for attr in el.attributes.iter() {
                    buf.push(' ');
                    buf.push_str(&attr.name);
                    buf.push_str("=\"");
                    buf.push_str(&attr.value.replace('"', "&quot;"));
                    buf.push('"');
                }
                buf.push('>');
                // Children
                let mut child = node.first_child;
                while let Some(cid) = child {
                    Self::serialize_node_html(tree, cid, buf);
                    child = tree.get_node(cid).next_sibling;
                }
                // Closing tag (skip void elements)
                let void_tags = ["area","base","br","col","embed","hr","img","input","link","meta","param","source","track","wbr"];
                if !void_tags.contains(&el.tag_name.to_lowercase().as_str()) {
                    buf.push_str("</");
                    buf.push_str(&el.tag_name.to_lowercase());
                    buf.push('>');
                }
            }
            NodeKind::Text(t) => {
                buf.push_str(&t.data);
            }
            NodeKind::Comment(c) => {
                buf.push_str("<!--");
                buf.push_str(&c.data);
                buf.push_str("-->");
            }
            _ => {}
        }
    }

    /// Serialize a single element node into a JS object literal string.
    /// Returns (element_literal, property_definitions) where property_definitions
    /// are Object.defineProperty calls for textContent/innerHTML.
    pub(crate) fn serialize_element(tree: &DomTree, node_id: NodeId) -> (String, Vec<String>) {
        let node = tree.get_node(node_id);
        let el = match &node.kind {
            NodeKind::Element(e) => e,
            _ => return ("null".to_string(), Vec::new()),
        };

        let tag = el.tag_name.to_lowercase();
        let id = el.attributes.get_value("id").unwrap_or("");
        let class_name = el.class_list.join(" ");
        let text = Self::escape_js_string(&Self::serialize_text_content(tree, node_id));
        let inner_html = Self::escape_js_string(&Self::serialize_inner_html(tree, node_id));

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

        // Build the element literal — note: _parentId and _childNodesIds are internal
        // backing stores. Getters for parentNode, children, childNodes are added via
        // Object.defineProperty below so they always return live objects from __dom_elements__.
        let element_literal = format!(
            r#"{{tagName:"{tag}",id:{id},className:{cls},_nodeId:{nid},_parentId:{parent_id},_childNodesIds:{child_nodes},_textContent:{txt},_innerHTML:{inner_html},_attrs:{attrs},style:{style}}}"#,
            tag = tag.to_uppercase(),
            id = Self::escape_js_string(id),
            cls = Self::escape_js_string(&class_name),
            txt = text,
            inner_html = inner_html,
            nid = nid,
            style = style_js,
            attrs = attrs_js,
            child_nodes = child_nodes_js,
            parent_id = parent_js,
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

        // Add common HTML attribute getters (href, src, alt, title, value, etc.)
        prop_defs.push(format!(
            r#"(function(el) {{
                var attrGetters = {{href:1,src:1,alt:1,title:1,value:1,type:1,name:1,action:1,method:1,target:1,rel:1,placeholder:1}};
                Object.keys(attrGetters).forEach(function(k) {{
                    Object.defineProperty(el, k, {{
                        get: function() {{ return this._attrs[k] || ''; }},
                        set: function(v) {{ this._attrs[k] = String(v); }},
                        enumerable: true, configurable: true
                    }});
                }});
            }})(__dom_elements__[{nid}])"#,
            nid = nid
        ));

        // parentNode getter — returns live object from __dom_elements__ (not raw node ID)
        prop_defs.push(format!(
            r#"Object.defineProperty(__dom_elements__[{nid}],'parentNode',{{get:function(){{return this._parentId?(__dom_elements__[this._parentId]||null):null}},enumerable:true,configurable:true}})"#,
            nid = nid
        ));

        // children getter — returns only element children (filter out text nodes)
        prop_defs.push(format!(
            r#"Object.defineProperty(__dom_elements__[{nid}],'children',{{get:function(){{var r=[];var ids=this._childNodesIds||[];for(var i=0;i<ids.length;i++){{var e=__dom_elements__[ids[i]];if(e&&e.tagName)r.push(e)}}return r}},enumerable:true,configurable:true}})"#,
            nid = nid
        ));

        // childNodes getter — returns all child nodes (elements + text)
        prop_defs.push(format!(
            r#"Object.defineProperty(__dom_elements__[{nid}],'childNodes',{{get:function(){{var r=[];var ids=this._childNodesIds||[];for(var i=0;i<ids.length;i++){{var e=__dom_elements__[ids[i]];if(e)r.push(e)}}return r}},enumerable:true,configurable:true}})"#,
            nid = nid
        ));

        // Set the shared prototype so all DOM methods are available
        prop_defs.push(format!(
            r#"Object.setPrototypeOf(__dom_elements__[{nid}],__dom_element_proto__)"#,
            nid = nid
        ));

        (element_literal, prop_defs)
    }

    /// Escape a string for use as a JS double-quoted string literal.
    pub(crate) fn escape_js_string(s: &str) -> String {
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
    pub(crate) fn escape_js_key(s: &str) -> String {
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
    pub(crate) fn find_title(tree: &DomTree) -> String {
        if let Some(title_id) = tree.query_selector("title") {
            Self::serialize_text_content(tree, title_id)
        } else {
            String::new()
        }
    }

    /// Build an id-to-nodeId index string for getElementById lookups.
    pub(crate) fn build_id_index(tree: &DomTree) -> String {
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
                let parent_id = node.parent
                    .map(|p| p.data().as_ffi().to_string())
                    .unwrap_or_else(|| "null".to_string());
                elements_js.push(format!(
                    "{}:{{nodeType:3,nodeName:\"#text\",textContent:{},_parentId:{}}}",
                    key, text, parent_id
                ));
                // Add parentNode getter for text nodes too
                prop_defs_js.push(format!(
                    r#"Object.defineProperty(__dom_elements__[{key}],'parentNode',{{get:function(){{return this._parentId?(__dom_elements__[this._parentId]||null):null}},enumerable:true,configurable:true}})"#,
                    key = key
                ));
            }
        }

        // 3. Build id index for getElementById
        let id_index = Self::build_id_index(dom);

        // 4. Find title, head, body node IDs
        let title = Self::escape_js_string(&Self::find_title(dom));
        let head_id = dom.head_node.data().as_ffi();
        let body_id = dom.body_node.data().as_ffi();
        let html_id = dom.html_node.data().as_ffi();

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
        code.push_str(&format!("var __dom_html_id__ = {};\n", html_id));
        code.push_str(
            "var __dom_node_name__ = function(n) {\n\
             if (n === null || n === undefined) return null;\n\
             var e = __dom_elements__[n];\n\
             return e || null;\n\
             };\n",
        );

        // Set window alias
        code.push_str("globalThis.window = globalThis;\n");

        // Minimal Node / Element / HTMLElement constructors for SPA frameworks
        code.push_str(r#"
        // --- Node constructor (constants only) ---
        var Node = function() {};
        Node.ELEMENT_NODE = 1;
        Node.ATTRIBUTE_NODE = 2;
        Node.TEXT_NODE = 3;
        Node.CDATA_SECTION_NODE = 4;
        Node.PROCESSING_INSTRUCTION_NODE = 7;
        Node.COMMENT_NODE = 8;
        Node.DOCUMENT_NODE = 9;
        Node.DOCUMENT_TYPE_NODE = 10;
        Node.DOCUMENT_FRAGMENT_NODE = 11;
        globalThis.Node = Node;

        // --- Element constructor ---
        var Element = function() {};
        Element.prototype = Object.create(Node.prototype);
        globalThis.Element = Element;

        // --- HTMLElement constructor ---
        var HTMLElement = function() {};
        HTMLElement.prototype = Object.create(Element.prototype);
        globalThis.HTMLElement = HTMLElement;

        // Fix __dom_element_proto__ chain: inherit from HTMLElement.prototype for instanceof
        if (typeof __dom_element_proto__ !== 'undefined') {
            Object.setPrototypeOf(__dom_element_proto__, HTMLElement.prototype);
        }
        "#);

        // Eval 1: document + window globals (must succeed)
        let doc_code = format!(
            r#"globalThis.window = globalThis;
globalThis.document = {{
    _title: {title},
    nodeType: 9,
    nodeName: '#document',
    get title() {{ return this._title; }},
    set title(v) {{ this._title = String(v); }},
    _head: null, _body: null, _documentElement: null,
    get head() {{ if (!this._head && typeof __dom_node_name__ === 'function') this._head = __dom_node_name__(__dom_head_id__); return this._head; }},
    get body() {{ if (!this._body && typeof __dom_node_name__ === 'function') this._body = __dom_node_name__(__dom_body_id__); return this._body; }},
    get documentElement() {{ if (!this._documentElement && typeof __dom_node_name__ === 'function') this._documentElement = __dom_node_name__(__dom_html_id__); return this._documentElement; }},
    get cookie() {{ return (typeof _get_cookies === 'function') ? _get_cookies() : ''; }},
    set cookie(v) {{ if (typeof _set_cookie === 'function') _set_cookie(v); }},
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
            tagName: tag.toUpperCase(), id: "", className: "",
            _nodeId: 0, _createdId: newId, _parentId: null, _childNodesIds: [],
            _textContent: "", _innerHTML: "",
            _attrs: {{}}, style: {{}}
        }};
        Object.setPrototypeOf(el, __dom_element_proto__);
        if (typeof __dom_elements__ !== 'undefined') __dom_elements__[newId] = el;
        return el;
    }},
    createDocumentFragment: function() {{
        var newId = 'created_' + (++this._createCounter);
        var frag = {{
            nodeType: 11, nodeName: '#document-fragment', nodeValue: null,
            _nodeId: 0, _createdId: newId, _parentId: null, _childNodesIds: [],
            _textContent: "", _innerHTML: "",
            _attrs: {{}}, style: {{}}
        }};
        Object.setPrototypeOf(frag, __dom_element_proto__);
        if (typeof __dom_elements__ !== 'undefined') __dom_elements__[newId] = frag;
        return frag;
    }},
    createComment: function(data) {{
        var newId = 'created_' + (++this._createCounter);
        var comment = {{
            nodeType: 8, nodeName: '#comment', nodeValue: String(data || ''),
            _nodeId: 0, _createdId: newId, _parentId: null, _childNodesIds: [],
            _textContent: String(data || ''), _innerHTML: "",
            _attrs: {{}}, style: {{}}
        }};
        Object.setPrototypeOf(comment, __dom_element_proto__);
        if (typeof __dom_elements__ !== 'undefined') __dom_elements__[newId] = comment;
        return comment;
    }},
    createTextNode: function(data) {{
        var newId = 'created_' + (++this._createCounter);
        var text = {{
            nodeType: 3, nodeName: '#text', nodeValue: String(data || ''),
            _nodeId: 0, _createdId: newId, _parentId: null, _childNodesIds: [],
            _textContent: String(data || ''), _innerHTML: "",
            _attrs: {{}}, style: {{}}
        }};
        Object.setPrototypeOf(text, __dom_element_proto__);
        if (typeof __dom_elements__ !== 'undefined') __dom_elements__[newId] = text;
        return text;
    }},
    createRange: function() {{
        return {{
            selectNodeContents: function(node) {{ this._node = node; }},
            createContextualFragment: function(html) {{
                var frag = document.createDocumentFragment();
                if (typeof html === 'string' && html.length > 0) {{
                    var tmp = document.createElement('div');
                    tmp.innerHTML = html;
                    while (tmp.firstChild) {{
                        frag.appendChild(tmp.firstChild);
                    }}
                }}
                return frag;
            }},
            collapse: function() {{}},
            selectNode: function(node) {{ this._node = node; }},
            deleteContents: function() {{}},
            cloneContents: function() {{ return document.createDocumentFragment(); }},
            extractContents: function() {{ return document.createDocumentFragment(); }}
        }};
    }},
    createEvent: function(type) {{
        return {{
            type: type || '',
            bubbles: false,
            cancelable: false,
            target: null,
            currentTarget: null,
            defaultPrevented: false,
            timeStamp: Date.now(),
            initEvent: function(type, bubbles, cancelable) {{
                this.type = type || '';
                this.bubbles = !!bubbles;
                this.cancelable = !!cancelable;
            }},
            preventDefault: function() {{ this.defaultPrevented = true; }},
            stopPropagation: function() {{}},
            stopImmediatePropagation: function() {{}}
        }};
    }},
    _listeners: {{}},
    addEventListener: function(type, fn) {{
        if (!this._listeners[type]) this._listeners[type] = [];
        this._listeners[type].push(fn);
    }},
    removeEventListener: function(type, fn) {{
        if (this._listeners[type]) {{
            var idx = this._listeners[type].indexOf(fn);
            if (idx >= 0) this._listeners[type].splice(idx, 1);
        }}
    }},
    dispatchEvent: function(evt) {{
        if (this._listeners[evt.type]) {{
            var self = this;
            this._listeners[evt.type].forEach(function(fn) {{ fn.call(self, evt); }});
        }}
        return true;
    }},
    readyState: 'complete',
    visibilityState: 'visible',
    hidden: false,
    hasFocus: function() {{ return true; }},
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
                        .filter(|(_nid, (tag, classes, id, parent))| {
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

        // Register native __queue_dynamic_script__ function
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let queue_fn = Function::new(ctx.clone(), |args: Rest<Value>| -> rquickjs::Result<()> {
                let url = args.get(0)
                    .and_then(|v| v.as_string())
                    .and_then(|s| s.to_string().ok())
                    .unwrap_or_default();
                if !url.is_empty() {
                    DYNAMIC_SCRIPTS.with(|scripts| {
                        scripts.borrow_mut().push(url);
                    });
                }
                Ok(())
            })?;
            ctx.globals().set("__queue_dynamic_script__", queue_fn)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to register __queue_dynamic_script__: {:?}", e))?;

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

        // Helper: check if elementId is a descendant of rootId
        function __is_descendant__(rootId, elementId) {
            if (!rootId || !elementId) return false;
            if (rootId === elementId) return true;
            var el = __dom_elements__[elementId];
            while (el) {
                var pid = el._parentId;
                if (!pid) return false;
                if (pid === rootId) return true;
                el = __dom_elements__[pid];
            }
            return false;
        }

        // Shared prototype for all DOM elements — add methods here once, all elements inherit them
        var __dom_element_proto__ = {
            appendChild: function(child) {
                var childId = child._nodeId || child._createdId;
                if (childId) {
                    __mut_append_child__(this._nodeId || this._createdId, childId);
                } else {
                    __mut_append_child__(this._nodeId || this._createdId, child.tagName);
                }
                this._childNodesIds.push(childId || 0);
                child._parentId = this._nodeId || this._createdId;
                // Detect dynamic script insertion
                if (child.tagName === 'SCRIPT') {
                    var scriptSrc = child.src || (child._attrs && child._attrs.src) || '';
                    if (scriptSrc) {
                        if (typeof __queue_dynamic_script__ === 'function') {
                            __queue_dynamic_script__(scriptSrc);
                        }
                    } else {
                        var code = child.textContent || child._textContent || '';
                        if (code) {
                            try { eval(code); } catch(e) {}
                        }
                    }
                }
                return child;
            },
            insertBefore: function(newNode, refNode) {
                if (!refNode) return this.appendChild(newNode);
                var newId = newNode._nodeId || 0;
                var refId = refNode._nodeId || 0;
                __mut_append_child__(this._nodeId, newId);
                var refIdx = this._childNodesIds.indexOf(refId);
                if (refIdx >= 0) {
                    this._childNodesIds.splice(refIdx, 0, newId);
                } else {
                    this._childNodesIds.push(newId);
                }
                newNode._parentId = this._nodeId;
                // Detect dynamic script insertion
                if (newNode.tagName === 'SCRIPT') {
                    var scriptSrc = newNode.src || (newNode._attrs && newNode._attrs.src) || '';
                    if (scriptSrc) {
                        if (typeof __queue_dynamic_script__ === 'function') {
                            __queue_dynamic_script__(scriptSrc);
                        }
                    } else {
                        var code = newNode.textContent || newNode._textContent || '';
                        if (code) {
                            try { eval(code); } catch(e) {}
                        }
                    }
                }
                return newNode;
            },
            removeChild: function(child) {
                var childId = child._nodeId || 0;
                __mut_remove_child__(this._nodeId, childId);
                var idx = this._childNodesIds.indexOf(childId);
                if (idx >= 0) this._childNodesIds.splice(idx, 1);
                child._parentId = null;
                return child;
            },
            remove: function() {
                var parent = this.parentNode;
                if (parent && typeof parent.removeChild === 'function') {
                    parent.removeChild(this);
                }
            },
            cloneNode: function(deep) {
                // Generate a new ID for the clone
                var _cloneCounter = (globalThis.__cloneCounter = (globalThis.__cloneCounter || 0) + 1);
                var cloneId = 'cloned_' + _cloneCounter;
                var clone = {
                    tagName: this.tagName, id: this.id || '', className: this.className || '',
                    _nodeId: 0, _parentId: null, _childNodesIds: [],
                    _textContent: this._textContent || '', _innerHTML: this._innerHTML || '',
                    _attrs: JSON.parse(JSON.stringify(this._attrs || {})),
                    style: JSON.parse(JSON.stringify(this.style || {}))
                };
                Object.setPrototypeOf(clone, __dom_element_proto__);
                // Register clone in __dom_elements__
                if (typeof __dom_elements__ !== 'undefined') __dom_elements__[cloneId] = clone;
                if (deep) {
                    var kids = this._childNodesIds || [];
                    for (var i = 0; i < kids.length; i++) {
                        var child = __dom_elements__[kids[i]];
                        if (child && typeof child.cloneNode === 'function') {
                            var childClone = child.cloneNode(true);
                            childClone._parentId = cloneId;
                            clone._childNodesIds.push(childClone._nodeId || childClone._clonedId || ('cloned_' + globalThis.__cloneCounter));
                        }
                    }
                }
                clone._clonedId = cloneId;
                return clone;
            },
            addEventListener: function(type, listener) {
                if (!this._listeners) this._listeners = {};
                if (!this._listeners[type]) this._listeners[type] = [];
                this._listeners[type].push(listener);
            },
            removeEventListener: function(type, listener) {
                if (this._listeners && this._listeners[type]) {
                    var idx = this._listeners[type].indexOf(listener);
                    if (idx >= 0) this._listeners[type].splice(idx, 1);
                }
            },
            dispatchEvent: function(event) {
                if (this._listeners && this._listeners[event.type]) {
                    var self = this;
                    this._listeners[event.type].forEach(function(l) { l.call(self, event); });
                }
                return true;
            },
            matches: function(selector) {
                selector = selector.trim();
                if (selector.charAt(0) === '#') return this.id === selector.substring(1);
                if (selector.charAt(0) === '.') {
                    return (this.className || '').split(' ').indexOf(selector.substring(1)) >= 0;
                }
                return this.tagName === selector.toUpperCase();
            },
            closest: function(selector) {
                var node = this;
                while (node) {
                    if (typeof node.matches === 'function' && node.matches(selector)) return node;
                    node = node.parentNode;
                }
                return null;
            },
            contains: function(other) {
                while (other) {
                    if (other === this) return true;
                    other = other.parentNode;
                }
                return false;
            },
            getAttribute: function(n) { return this._attrs[n] || null; },
            setAttribute: function(n, v) { this._attrs[n] = String(v); __mut_set_attr__(this._nodeId, n, String(v)); },
            removeAttribute: function(n) { delete this._attrs[n]; __mut_remove_attr__(this._nodeId, n); },
            hasAttribute: function(n) { return n in this._attrs; },
            get firstChild() {
                if (!this._childNodesIds || this._childNodesIds.length === 0) return null;
                return __dom_elements__[this._childNodesIds[0]] || null;
            },
            get lastChild() {
                if (!this._childNodesIds || this._childNodesIds.length === 0) return null;
                return __dom_elements__[this._childNodesIds[this._childNodesIds.length - 1]] || null;
            },
            get nextSibling() {
                var p = this.parentNode;
                if (!p || !p._childNodesIds) return null;
                var idx = p._childNodesIds.indexOf(this._nodeId);
                if (idx < 0 || idx >= p._childNodesIds.length - 1) return null;
                return __dom_elements__[p._childNodesIds[idx + 1]] || null;
            },
            get previousSibling() {
                var p = this.parentNode;
                if (!p || !p._childNodesIds) return null;
                var idx = p._childNodesIds.indexOf(this._nodeId);
                if (idx <= 0) return null;
                return __dom_elements__[p._childNodesIds[idx - 1]] || null;
            },
            get outerHTML() {
                var tag = (this.tagName || '').toLowerCase();
                var attrs = '';
                var a = this._attrs || {};
                for (var k in a) {
                    if (a.hasOwnProperty(k)) {
                        attrs += ' ' + k + '="' + String(a[k]).replace(/"/g, '&quot;') + '"';
                    }
                }
                var voidTags = {area:1,base:1,br:1,col:1,embed:1,hr:1,img:1,input:1,link:1,meta:1,param:1,source:1,track:1,wbr:1};
                if (voidTags[tag]) return '<' + tag + attrs + '>';
                return '<' + tag + attrs + '>' + (this.innerHTML || '') + '</' + tag + '>';
            },
            get parentElement() {
                return this._parentId ? __dom_elements__[this._parentId] : null;
            },
            get dataset() {
                var ds = {};
                var attrs = this._attributes || this._attrs || {};
                for (var k in attrs) {
                    if (k.startsWith('data-')) {
                        var key = k.substring(5).replace(/-([a-z])/g, function(m, c) { return c.toUpperCase(); });
                        ds[key] = attrs[k];
                    }
                }
                return ds;
            },
            get classList() {
                var self = this;
                var classes = (self.className || '').split(' ').filter(function(c) { return c; });
                return {
                    add: function(c) { if (!classes.includes(c)) { classes.push(c); self.className = classes.join(' '); } },
                    remove: function(c) { classes = classes.filter(function(x) { return x !== c; }); self.className = classes.join(' '); },
                    contains: function(c) { return classes.indexOf(c) >= 0; },
                    toggle: function(c) { if (classes.includes(c)) { this.remove(c); return false; } else { this.add(c); return true; } },
                    toString: function() { return classes.join(' '); }
                };
            },
            getBoundingClientRect: function() {
                return { top: 0, left: 0, bottom: 0, right: 0, width: 0, height: 0, x: 0, y: 0 };
            },
            getClientRects: function() {
                return [];
            },
            get offsetWidth() { return 0; },
            get offsetHeight() { return 0; },
            get clientWidth() { return 0; },
            get clientHeight() { return 0; },
            get scrollWidth() { return 0; },
            get scrollHeight() { return 0; },
            get ownerDocument() { return typeof document !== 'undefined' ? document : null; },
            insertAdjacentHTML: function(position, html) {
                // simplified implementation
            },
            getElementsByTagName: function(tag) {
                tag = (tag || "*").toUpperCase();
                var result = [];
                var walk = function(node) {
                    var kids = node._childNodesIds || node.childNodes || [];
                    for (var i = 0; i < kids.length; i++) {
                        var child = typeof kids[i] === 'object' ? kids[i] : __dom_elements__[kids[i]];
                        if (child) {
                            if (child.tagName && (child.tagName === tag || tag === "*")) result.push(child);
                            var sub = child.getElementsByTagName ? child.getElementsByTagName(tag) : [];
                            for (var j = 0; j < sub.length; j++) result.push(sub[j]);
                        }
                    }
                };
                walk(this);
                return result;
            },
            getElementsByClassName: function(cls) {
                var result = [];
                var walk = function(node) {
                    var kids = node._childNodesIds || node.childNodes || [];
                    for (var i = 0; i < kids.length; i++) {
                        var child = typeof kids[i] === 'object' ? kids[i] : __dom_elements__[kids[i]];
                        if (child) {
                            if (child.className && child.className.split && child.className.split(' ').indexOf(cls) >= 0) result.push(child);
                            var sub = child.getElementsByClassName ? child.getElementsByClassName(cls) : [];
                            for (var j = 0; j < sub.length; j++) result.push(sub[j]);
                        }
                    }
                };
                walk(this);
                return result;
            },
            querySelector: function(sel) {
                if (typeof _dom_query_selector_all === 'function') {
                    var ids = _dom_query_selector_all(sel);
                    if (ids) {
                        var rootId = this._nodeId || this._createdId;
                        for (var i = 0; i < ids.length; i++) {
                            if (__is_descendant__(rootId, ids[i])) return __dom_elements__[ids[i]] || null;
                        }
                    }
                }
                return null;
            },
            querySelectorAll: function(sel) {
                if (typeof _dom_query_selector_all === 'function') {
                    var ids = _dom_query_selector_all(sel);
                    var result = [];
                    if (ids) {
                        var rootId = this._nodeId || this._createdId;
                        for (var i = 0; i < ids.length; i++) {
                            var el = __dom_elements__[ids[i]];
                            if (el && __is_descendant__(rootId, ids[i])) result.push(el);
                        }
                    }
                    return result;
                }
                return [];
            }
        };
        "#;
        self.context.with(|ctx| -> rquickjs::Result<()> {
            let _: Value = ctx.eval(code)?;
            Ok(())
        }).map_err(|e| anyhow!("Failed to setup mutation queue: {:?}", e))?;
        Ok(())
    }
}
