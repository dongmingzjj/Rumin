// DOM tree implementation
use std::collections::HashMap;
use slotmap::SlotMap;

use crate::node::{
    DomNode, NodeId, NodeKind, DocumentData, ElementData, TextData, CommentData,
    DocumentReadyState, NamedNodeMap,
};
use crate::selector::SelectorEngine;

/// The main DOM tree structure.
pub struct DomTree {
    pub nodes: SlotMap<NodeId, DomNode>,
    /// Quick lookup by element id attribute value.
    pub id_index: HashMap<String, NodeId>,
    /// The root Document node.
    pub document_node: NodeId,
    /// The <html> element (created during `new()`).
    pub html_node: NodeId,
    /// The <head> element (created during `new()`).
    pub head_node: NodeId,
    /// The <body> element (created during `new()`).
    pub body_node: NodeId,
}

impl DomTree {
    /// Create a new DOM tree with a minimal document skeleton:
    ///   Document → html → head + body
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();

        let document_node = nodes.insert(DomNode::new(NodeKind::Document(DocumentData {
            ready_state: DocumentReadyState::Loading,
            ..Default::default()
        })));

        let html_node = nodes.insert(DomNode::new(NodeKind::Element(ElementData::new("html"))));
        let head_node = nodes.insert(DomNode::new(NodeKind::Element(ElementData::new("head"))));
        let body_node = nodes.insert(DomNode::new(NodeKind::Element(ElementData::new("body"))));

        // Wire up: document → html, html → head + body (siblings)
        nodes[document_node].first_child = Some(html_node);
        nodes[document_node].last_child = Some(html_node);

        nodes[html_node].parent = Some(document_node);
        nodes[html_node].first_child = Some(head_node);
        nodes[html_node].last_child = Some(body_node);

        nodes[head_node].parent = Some(html_node);
        nodes[head_node].next_sibling = Some(body_node);

        nodes[body_node].parent = Some(html_node);
        nodes[body_node].prev_sibling = Some(head_node);

        Self {
            nodes,
            id_index: HashMap::new(),
            document_node,
            html_node,
            head_node,
            body_node,
        }
    }

    // ── Creation helpers ────────────────────────────────────────────────────

    /// Create a new generic node (any kind).
    pub fn create_node(&mut self, kind: NodeKind) -> NodeId {
        self.nodes.insert(DomNode::new(kind))
    }

    /// Create a Document node.
    pub fn create_document(&mut self, url: &str) -> NodeId {
        let id = self.create_node(NodeKind::Document(DocumentData {
            url: url.to_string(),
            ..Default::default()
        }));
        self.document_node = id;
        id
    }

    /// Create a new Element node (not yet attached to the tree).
    pub fn create_element(&mut self, tag: &str) -> NodeId {
        self.nodes.insert(DomNode::new(NodeKind::Element(ElementData::new(tag))))
    }

    /// Create an element with pre-populated attributes.
    pub fn create_element_with_attrs(&mut self, tag: &str, attributes: NamedNodeMap) -> NodeId {
        let class_list = attributes
            .get_value("class")
            .map(|c| c.split_whitespace().map(String::from).collect())
            .unwrap_or_default();
        let mut style = crate::node::InlineStyle::default();
        if let Some(s) = attributes.get_value("style") {
            style.parse_and_merge(s);
        }
        let id_attr = attributes.get_value("id").map(|s| s.to_string());

        let el = ElementData {
            tag_name: tag.to_ascii_uppercase(),
            attributes,
            class_list,
            style,
            listeners: HashMap::new(),
        };
        let id = self.create_node(NodeKind::Element(el));

        if let Some(id_val) = id_attr {
            self.id_index.insert(id_val, id);
        }
        id
    }

    /// Create a new Text node.
    pub fn create_text(&mut self, data: &str) -> NodeId {
        self.nodes.insert(DomNode::new(NodeKind::Text(TextData {
            data: data.to_string(),
        })))
    }

    /// Create a new Comment node.
    pub fn create_comment(&mut self, data: &str) -> NodeId {
        self.nodes.insert(DomNode::new(NodeKind::Comment(CommentData {
            data: data.to_string(),
        })))
    }

    // ── Tree manipulation ───────────────────────────────────────────────────

    /// Append `child` as the last child of `parent`.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        // Detach from current parent first if needed.
        if let Some(cur_parent) = self.nodes[child].parent {
            self.remove_child(cur_parent, child);
        }

        self.nodes[child].parent = Some(parent);

        if let Some(last) = self.nodes[parent].last_child {
            self.nodes[last].next_sibling = Some(child);
            self.nodes[child].prev_sibling = Some(last);
            self.nodes[parent].last_child = Some(child);
        } else {
            self.nodes[parent].first_child = Some(child);
            self.nodes[parent].last_child = Some(child);
        }

        // Index maintenance.
        self.index_node(child);
    }

    /// Insert `child` before `reference` under `parent`.
    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        debug_assert_eq!(
            self.nodes[reference].parent,
            Some(parent),
            "reference must be a child of parent"
        );

        // Detach from current parent if needed.
        if let Some(cur_parent) = self.nodes[child].parent {
            self.remove_child(cur_parent, child);
        }

        let prev = self.nodes[reference].prev_sibling;
        self.nodes[child].parent = Some(parent);
        self.nodes[child].next_sibling = Some(reference);
        self.nodes[child].prev_sibling = prev;
        self.nodes[reference].prev_sibling = Some(child);

        if let Some(prev_id) = prev {
            self.nodes[prev_id].next_sibling = Some(child);
        } else {
            // `reference` was the first child.
            self.nodes[parent].first_child = Some(child);
        }

        self.index_node(child);
    }

    /// Remove `child` from `parent`'s child list.  Returns `true` if the child
    /// was actually present.
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> bool {
        if self.nodes[child].parent != Some(parent) {
            return false;
        }

        let prev = self.nodes[child].prev_sibling;
        let next = self.nodes[child].next_sibling;

        if let Some(prev_id) = prev {
            self.nodes[prev_id].next_sibling = next;
        } else {
            self.nodes[parent].first_child = next;
        }

        if let Some(next_id) = next {
            self.nodes[next_id].prev_sibling = prev;
        } else {
            self.nodes[parent].last_child = prev;
        }

        self.nodes[child].parent = None;
        self.nodes[child].prev_sibling = None;
        self.nodes[child].next_sibling = None;

        self.unindex_node(child);
        true
    }

    /// Get the children of a node as a Vec (walks the linked list).
    pub fn children(&self, parent: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        let mut cur = self.nodes[parent].first_child;
        while let Some(id) = cur {
            result.push(id);
            cur = self.nodes[id].next_sibling;
        }
        result
    }

    /// Get a node by id (convenience).
    pub fn get_node(&self, id: NodeId) -> &DomNode {
        &self.nodes[id]
    }

    /// Get a mutable node by id (convenience).
    pub fn get_node_mut(&mut self, id: NodeId) -> &mut DomNode {
        &mut self.nodes[id]
    }

    // ── Queries ─────────────────────────────────────────────────────────────

    /// Return the first element whose `id` attribute matches.
    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        self.id_index.get(id).copied()
    }

    /// Return the first element matching the given CSS selector.
    pub fn query_selector(&self, selector: &str) -> Option<NodeId> {
        let engine = SelectorEngine::parse(selector);
        engine.query_selector(self, self.html_node)
    }

    /// Return all elements matching the given CSS selector.
    pub fn query_selector_all(&self, selector: &str) -> Vec<NodeId> {
        let engine = SelectorEngine::parse(selector);
        engine.query_selector_all(self, self.html_node)
    }

    // ── Index helpers ───────────────────────────────────────────────────────

    fn index_node(&mut self, node_id: NodeId) {
        if let NodeKind::Element(ref el) = self.nodes[node_id].kind {
            if let Some(id_val) = el.attributes.get_value("id") {
                self.id_index.insert(id_val.to_string(), node_id);
            }
        }
    }

    fn unindex_node(&mut self, node_id: NodeId) {
        if let NodeKind::Element(ref el) = self.nodes[node_id].kind {
            if let Some(id_val) = el.attributes.get_value("id") {
                self.id_index.remove(id_val);
            }
        }
    }

    // ── Text content helper ─────────────────────────────────────────────────

    /// Recursively collect all Text data under `node`.
    pub fn text_content(&self, node: NodeId) -> String {
        let mut buf = String::new();
        self.collect_text(node, &mut buf);
        buf
    }

    fn collect_text(&self, node: NodeId, buf: &mut String) {
        if let NodeKind::Text(ref t) = self.nodes[node].kind {
            buf.push_str(&t.data);
        }
        let mut child = self.nodes[node].first_child;
        while let Some(id) = child {
            self.collect_text(id, buf);
            child = self.nodes[id].next_sibling;
        }
    }
}

impl Default for DomTree {
    fn default() -> Self {
        Self::new()
    }
}
