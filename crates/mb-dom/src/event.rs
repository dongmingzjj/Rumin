// Event system — EventTarget trait and three-phase dispatch

use crate::node::{
    NodeId, NodeKind, Event, EventPhase, EventListenerCallback,
};
use crate::tree::DomTree;

/// Trait for anything that can receive events (currently only Element nodes).
pub trait EventTarget {
    fn add_event_listener(
        &mut self,
        node_id: NodeId,
        event_type: &str,
        callback: EventListenerCallback,
        capture: bool,
    );

    fn remove_event_listener(
        &mut self,
        node_id: NodeId,
        event_type: &str,
        callback: EventListenerCallback,
    ) -> bool;

    fn dispatch_event(&mut self, target: NodeId, event: &mut Event) -> bool;
}

impl EventTarget for DomTree {
    fn add_event_listener(
        &mut self,
        node_id: NodeId,
        event_type: &str,
        callback: EventListenerCallback,
        capture: bool,
    ) {
        crate::element::add_event_listener(self, node_id, event_type, callback, capture);
    }

    fn remove_event_listener(
        &mut self,
        node_id: NodeId,
        event_type: &str,
        callback: EventListenerCallback,
    ) -> bool {
        crate::element::remove_event_listener(self, node_id, event_type, callback)
    }

    /// Dispatch an event with the three-phase model:
    ///   1. Capturing phase: walk from root to target's parent
    ///   2. Target phase: invoke listeners on the target
    ///   3. Bubbling phase: walk from target's parent back up to root
    ///
    /// Returns `false` if `event.prevented` is set at the end.
    fn dispatch_event(&mut self, target: NodeId, event: &mut Event) -> bool {
        event.target = Some(target);

        // Build the path from target up to (but not including) the document.
        let mut path = Vec::new();
        let mut cur = self.nodes[target].parent;
        while let Some(id) = cur {
            // Stop at document node
            if matches!(self.nodes[id].kind, NodeKind::Document(_)) {
                break;
            }
            path.push(id);
            cur = self.nodes[id].parent;
        }

        // ── Capturing phase ───────────────────────────────────────────────
        event.phase = EventPhase::Capturing;
        for &ancestor in path.iter().rev() {
            if event.stopped {
                break;
            }
            event.current_target = Some(ancestor);
            invoke_listeners(self, ancestor, event, true);
        }

        // ── Target phase ──────────────────────────────────────────────────
        if !event.stopped {
            event.phase = EventPhase::AtTarget;
            event.current_target = Some(target);
            invoke_listeners(self, target, event, true);
            if !event.stopped {
                invoke_listeners(self, target, event, false);
            }
        }

        // ── Bubbling phase ────────────────────────────────────────────────
        if !event.stopped && event.bubbles {
            event.phase = EventPhase::Bubbling;
            for &ancestor in &path {
                if event.stopped {
                    break;
                }
                event.current_target = Some(ancestor);
                invoke_listeners(self, ancestor, event, false);
            }
        }

        event.phase = EventPhase::None;
        event.current_target = None;

        !event.prevented
    }
}

/// Invoke matching listeners on a single node.
fn invoke_listeners(tree: &DomTree, node: NodeId, event: &mut Event, capture: bool) {
    if let NodeKind::Element(ref el) = tree.nodes[node].kind {
        if let Some(listeners) = el.listeners.get(&event.event_type) {
            for listener in listeners {
                if listener.capture != capture {
                    continue;
                }
                (listener.callback)(event);
                if event.stopped {
                    break;
                }
            }
        }
    }
}
