// Node types and data structures
use std::collections::HashMap;
use slotmap::new_key_type;
use serde::{Serialize, Deserialize};

new_key_type! { pub struct NodeId; }

/// DomNode — a single node in the DOM tree.
#[derive(Debug, Clone)]
pub struct DomNode {
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub last_child: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
}

impl DomNode {
    pub fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            parent: None,
            first_child: None,
            last_child: None,
            next_sibling: None,
            prev_sibling: None,
        }
    }

    pub fn is_element(&self) -> bool {
        matches!(self.kind, NodeKind::Element(_))
    }

    pub fn is_text(&self) -> bool {
        matches!(self.kind, NodeKind::Text(_))
    }

    pub fn tag_name(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Element(data) => Some(&data.tag_name),
            _ => None,
        }
    }

    pub fn element_data(&self) -> Option<&ElementData> {
        match &self.kind {
            NodeKind::Element(data) => Some(data),
            _ => None,
        }
    }

    pub fn text_data(&self) -> Option<&TextData> {
        match &self.kind {
            NodeKind::Text(data) => Some(data),
            _ => None,
        }
    }
}

/// The four fundamental DOM node kinds.
#[derive(Debug, Clone)]
pub enum NodeKind {
    Document(DocumentData),
    Element(ElementData),
    Text(TextData),
    Comment(CommentData),
}

impl NodeKind {
    pub fn node_type(&self) -> u16 {
        match self {
            NodeKind::Document(_) => 9,
            NodeKind::Element(_) => 1,
            NodeKind::Text(_) => 3,
            NodeKind::Comment(_) => 8,
        }
    }

    pub fn node_name(&self) -> &str {
        match self {
            NodeKind::Document(_) => "#document",
            NodeKind::Element(e) => &e.tag_name,
            NodeKind::Text(_) => "#text",
            NodeKind::Comment(_) => "#comment",
        }
    }

    pub fn as_document_mut(&mut self) -> Option<&mut DocumentData> {
        match self {
            NodeKind::Document(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_element_mut(&mut self) -> Option<&mut ElementData> {
        match self {
            NodeKind::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn as_element(&self) -> Option<&ElementData> {
        match self {
            NodeKind::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&TextData> {
        match self {
            NodeKind::Text(t) => Some(t),
            _ => None,
        }
    }
}

// ── Document ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct DocumentData {
    pub url: String,
    pub title: String,
    pub cookie: String,
    pub ready_state: DocumentReadyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DocumentReadyState {
    #[default]
    Loading,
    Interactive,
    Complete,
}

impl std::fmt::Display for DocumentReadyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Loading => write!(f, "loading"),
            Self::Interactive => write!(f, "interactive"),
            Self::Complete => write!(f, "complete"),
        }
    }
}

// ── Element ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ElementData {
    /// Tag name stored in uppercase (e.g. "DIV", "SPAN").
    pub tag_name: String,
    pub attributes: NamedNodeMap,
    pub class_list: Vec<String>,
    pub style: InlineStyle,
    pub listeners: EventListenerMap,
}

impl ElementData {
    pub fn new(tag_name: &str) -> Self {
        Self {
            tag_name: tag_name.to_ascii_uppercase(),
            attributes: NamedNodeMap::new(),
            class_list: Vec::new(),
            style: InlineStyle::default(),
            listeners: HashMap::new(),
        }
    }
}

// ── Text / Comment ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TextData {
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct CommentData {
    pub data: String,
}

// ── NamedNodeMap ──────────────────────────────────────────────────────────────

/// An ordered map of attributes that preserves insertion order.
/// Backed by a `Vec<Attr>` for ordering and a `HashMap<String, usize>` for
/// O(1) lookup by name.
#[derive(Debug, Clone, Default)]
pub struct NamedNodeMap {
    entries: Vec<Attr>,
    index: HashMap<String, usize>,
}

impl NamedNodeMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Attr> {
        self.index.get(name).map(|&i| &self.entries[i])
    }

    pub fn get_value(&self, name: &str) -> Option<&str> {
        self.get(name).map(|a| a.value.as_str())
    }

    pub fn set(&mut self, name: String, value: String) {
        if let Some(&i) = self.index.get(&name) {
            self.entries[i].value = value;
        } else {
            let i = self.entries.len();
            self.entries.push(Attr { name: name.clone(), value });
            self.index.insert(name, i);
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<Attr> {
        if let Some(&i) = self.index.get(name) {
            let removed = self.entries.remove(i);
            self.index.remove(name);
            // Rebuild index for entries after the removed one.
            for j in i..self.entries.len() {
                self.index.insert(self.entries[j].name.clone(), j);
            }
            Some(removed)
        } else {
            None
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Attr> {
        self.entries.iter()
    }
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    pub value: String,
}

// ── InlineStyle ───────────────────────────────────────────────────────────────

/// Simple inline style representation: property-name → value.
#[derive(Debug, Clone, Default)]
pub struct InlineStyle {
    props: HashMap<String, String>,
}

impl InlineStyle {
    pub fn get(&self, prop: &str) -> Option<&str> {
        self.props.get(prop).map(|s| s.as_str())
    }

    pub fn set(&mut self, prop: String, value: String) {
        self.props.insert(prop, value);
    }

    pub fn remove(&mut self, prop: &str) -> Option<String> {
        self.props.remove(prop)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.props.iter()
    }

    /// Parse an inline style string like "color:red;font-size:12px" and merge.
    pub fn parse_and_merge(&mut self, css: &str) {
        for declaration in css.split(';') {
            let declaration = declaration.trim();
            if declaration.is_empty() {
                continue;
            }
            if let Some((prop, val)) = declaration.split_once(':') {
                self.set(prop.trim().to_string(), val.trim().to_string());
            }
        }
    }

    /// Serialize back to a CSS inline style string.
    pub fn to_css_string(&self) -> String {
        self.props
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join(";")
    }
}

// ── EventListenerMap ──────────────────────────────────────────────────────────

/// Map from event-type to a list of listeners registered for that type.
pub type EventListenerMap = HashMap<String, Vec<EventListener>>;

#[derive(Clone)]
pub struct EventListener {
    pub callback: EventListenerCallback,
    pub once: bool,
    pub capture: bool,
    pub passive: bool,
}

impl std::fmt::Debug for EventListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventListener")
            .field("once", &self.once)
            .field("capture", &self.capture)
            .field("passive", &self.passive)
            .finish()
    }
}

/// Callback type — function pointer taking a mutable reference to the event.
pub type EventListenerCallback = fn(&mut Event);

/// The Event object passed to listeners.
#[derive(Debug, Clone)]
pub struct Event {
    pub event_type: String,
    pub bubbles: bool,
    pub cancelable: bool,
    pub target: Option<NodeId>,
    pub current_target: Option<NodeId>,
    pub phase: EventPhase,
    pub stopped: bool,
    pub prevented: bool,
}

impl Event {
    pub fn new(event_type: &str, bubbles: bool, cancelable: bool) -> Self {
        Self {
            event_type: event_type.to_string(),
            bubbles,
            cancelable,
            target: None,
            current_target: None,
            phase: EventPhase::None,
            stopped: false,
            prevented: false,
        }
    }

    pub fn stop_propagation(&mut self) {
        self.stopped = true;
    }

    pub fn prevent_default(&mut self) {
        if self.cancelable {
            self.prevented = true;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventPhase {
    #[default]
    None,
    Capturing,
    AtTarget,
    Bubbling,
}
