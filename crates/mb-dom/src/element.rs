// Element operations — convenience wrappers around DomNode's ElementData.
use crate::node::{NodeId, NodeKind, EventListener, EventListenerCallback};

// We operate on DomTree, not on bare nodes, because index maintenance
// requires the full tree.  The free functions below all take &mut DomTree.

/// Get the value of an attribute on the element `node`.
pub fn get_attribute(tree: &crate::tree::DomTree, node: NodeId, name: &str) -> Option<String> {
    match &tree.nodes[node].kind {
        NodeKind::Element(el) => el.attributes.get_value(name).map(|s| s.to_string()),
        _ => None,
    }
}

/// Set an attribute on the element `node`.
pub fn set_attribute(tree: &mut crate::tree::DomTree, node: NodeId, name: &str, value: &str) {
    if let NodeKind::Element(ref mut el) = tree.nodes[node].kind {
        let is_id = name.eq_ignore_ascii_case("id");
        // If id changes, update index.
        if is_id {
            if let Some(old_id) = el.attributes.get_value("id") {
                tree.id_index.remove(old_id);
            }
            tree.id_index.insert(value.to_string(), node);
        }
        // Handle class attribute specially.
        if name.eq_ignore_ascii_case("class") {
            el.class_list = value.split_whitespace().map(|s| s.to_string()).collect();
        }
        // Handle style attribute.
        if name.eq_ignore_ascii_case("style") {
            el.style.parse_and_merge(value);
        }
        el.attributes.set(name.to_string(), value.to_string());
    }
}

/// Remove an attribute from the element `node`.
pub fn remove_attribute(tree: &mut crate::tree::DomTree, node: NodeId, name: &str) {
    if let NodeKind::Element(ref mut el) = tree.nodes[node].kind {
        if name.eq_ignore_ascii_case("id") {
            if let Some(old_id) = el.attributes.get_value("id") {
                tree.id_index.remove(old_id);
            }
        }
        if name.eq_ignore_ascii_case("class") {
            el.class_list.clear();
        }
        el.attributes.remove(name);
    }
}

/// Check whether the element `node` has the given attribute.
pub fn has_attribute(tree: &crate::tree::DomTree, node: NodeId, name: &str) -> bool {
    match &tree.nodes[node].kind {
        NodeKind::Element(el) => el.attributes.has(name),
        _ => false,
    }
}

/// Return a simple text representation of all descendant text nodes.
pub fn text_content(tree: &crate::tree::DomTree, node: NodeId) -> String {
    tree.text_content(node)
}

/// Add a CSS class to the element.
pub fn class_list_add(tree: &mut crate::tree::DomTree, node: NodeId, class: &str) {
    if let NodeKind::Element(ref mut el) = tree.nodes[node].kind {
        if !el.class_list.iter().any(|c| c == class) {
            el.class_list.push(class.to_string());
        }
        // Sync back to attribute.
        let val = el.class_list.join(" ");
        el.attributes.set("class".to_string(), val);
    }
}

/// Remove a CSS class from the element.
pub fn class_list_remove(tree: &mut crate::tree::DomTree, node: NodeId, class: &str) {
    if let NodeKind::Element(ref mut el) = tree.nodes[node].kind {
        el.class_list.retain(|c| c != class);
        let val = el.class_list.join(" ");
        if val.is_empty() {
            el.attributes.remove("class");
        } else {
            el.attributes.set("class".to_string(), val);
        }
    }
}

/// Check if the element has a given class.
pub fn class_list_contains(tree: &crate::tree::DomTree, node: NodeId, class: &str) -> bool {
    match &tree.nodes[node].kind {
        NodeKind::Element(el) => el.class_list.iter().any(|c| c == class),
        _ => false,
    }
}

/// Toggle a CSS class on the element; returns the new state.
pub fn class_list_toggle(tree: &mut crate::tree::DomTree, node: NodeId, class: &str) -> bool {
    if class_list_contains(tree, node, class) {
        class_list_remove(tree, node, class);
        false
    } else {
        class_list_add(tree, node, class);
        true
    }
}

// ── Event helpers ──────────────────────────────────────────────────────────────

/// Add an event listener to the element.
pub fn add_event_listener(
    tree: &mut crate::tree::DomTree,
    node: NodeId,
    event_type: &str,
    callback: EventListenerCallback,
    capture: bool,
) {
    if let NodeKind::Element(ref mut el) = tree.nodes[node].kind {
        el.listeners
            .entry(event_type.to_string())
            .or_default()
            .push(EventListener {
                callback,
                once: false,
                capture,
                passive: false,
            });
    }
}

/// Remove an event listener from the element by function pointer (removes the
/// first match).
pub fn remove_event_listener(
    tree: &mut crate::tree::DomTree,
    node: NodeId,
    event_type: &str,
    callback: EventListenerCallback,
) -> bool {
    if let NodeKind::Element(ref mut el) = tree.nodes[node].kind {
        if let Some(list) = el.listeners.get_mut(event_type) {
            if let Some(pos) = list.iter().position(|l| l.callback as usize == callback as usize) {
                list.remove(pos);
                return true;
            }
        }
    }
    false
}
