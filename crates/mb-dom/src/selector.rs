// CSS selector engine
//
// Supports:
//   - Tag:                div
//   - Class:              .foo
//   - ID:                 #bar
//   - Attribute:          [attr], [attr=val]
//   - Compound:           div.class#id
//   - Child combinator:   parent > child
//   - Descendant comb.:   ancestor descendant
//   - Pseudo:             :first-child, :last-child

use crate::node::{NodeId, NodeKind};
use crate::tree::DomTree;

// ── Selector AST ──────────────────────────────────────────────────────────────

/// A simple selector (the part between combinators).
#[derive(Debug, Clone, Default)]
pub struct SimpleSelector {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attr_name: Option<String>,
    pub attr_value: Option<String>, // None = presence check
    pub first_child: bool,
    pub last_child: bool,
}

impl SimpleSelector {
    /// Check whether this simple selector matches `node` in the given tree.
    pub fn matches(&self, tree: &DomTree, node: NodeId) -> bool {
        let n = &tree.nodes[node];

        let el = match &n.kind {
            NodeKind::Element(e) => e,
            _ => return false,
        };

        // Tag (case-insensitive — DOM stores uppercase, selectors use lowercase)
        if let Some(ref tag) = self.tag {
            if !el.tag_name.eq_ignore_ascii_case(tag) {
                return false;
            }
        }

        // ID
        if let Some(ref id) = self.id {
            match el.attributes.get_value("id") {
                Some(v) if v == id => {}
                _ => return false,
            }
        }

        // Classes
        for cls in &self.classes {
            if !el.class_list.iter().any(|c| c == cls) {
                return false;
            }
        }

        // Attribute
        if let Some(ref attr_name) = self.attr_name {
            match &self.attr_value {
                Some(val) => match el.attributes.get_value(attr_name) {
                    Some(v) if v == val => {}
                    _ => return false,
                },
                None => {
                    if !el.attributes.has(attr_name) {
                        return false;
                    }
                }
            }
        }

        // :first-child
        if self.first_child {
            if let Some(parent) = n.parent {
                if tree.nodes[parent].first_child != Some(node) {
                    return false;
                }
            } else {
                return false;
            }
        }

        // :last-child
        if self.last_child {
            if let Some(parent) = n.parent {
                if tree.nodes[parent].last_child != Some(node) {
                    return false;
                }
            } else {
                return false;
            }
        }

        true
    }
}

/// Combinator between selector parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// " " — any descendant
    Descendant,
    /// ">" — direct child
    Child,
}

/// A compound selector: [simple] [combinator] [simple] ...
#[derive(Debug, Clone)]
pub struct Selector {
    pub parts: Vec<(SimpleSelector, Option<Combinator>)>,
}

impl Selector {
    /// Check whether the selector matches `node`.
    pub fn matches(&self, tree: &DomTree, node: NodeId) -> bool {
        if self.parts.is_empty() {
            return false;
        }
        self.match_from(tree, node, self.parts.len() - 1)
    }

    /// Recursive matching: rightmost part must match `node`, then work left.
    fn match_from(&self, tree: &DomTree, node: NodeId, part_idx: usize) -> bool {
        let (ref simple, _) = self.parts[part_idx];
        if !simple.matches(tree, node) {
            return false;
        }

        if part_idx == 0 {
            return true;
        }

        let (_, prev_combinator) = self.parts[part_idx - 1];
        let combinator = prev_combinator.unwrap_or(Combinator::Descendant);

        match combinator {
            Combinator::Child => {
                if let Some(parent) = tree.nodes[node].parent {
                    self.match_from(tree, parent, part_idx - 1)
                } else {
                    false
                }
            }
            Combinator::Descendant => {
                let mut ancestor = tree.nodes[node].parent;
                while let Some(a) = ancestor {
                    if self.match_from(tree, a, part_idx - 1) {
                        return true;
                    }
                    ancestor = tree.nodes[a].parent;
                }
                false
            }
        }
    }
}

// ── SelectorEngine ────────────────────────────────────────────────────────────

/// Parses and matches CSS selectors.
pub struct SelectorEngine {
    pub selectors: Vec<Selector>,
}

impl SelectorEngine {
    /// Parse a selector string (may contain comma-separated selectors).
    pub fn parse(input: &str) -> Self {
        let selectors = input
            .split(',')
            .map(|s| parse_single_selector(s.trim()))
            .collect();
        Self { selectors }
    }

    /// Simple static entry point matching the old stub API.
    pub fn select(tree: &DomTree, selector: &str, root: NodeId) -> Vec<NodeId> {
        let engine = Self::parse(selector);
        engine.query_selector_all(tree, root)
    }

    /// Return the first matching node (depth-first from root, excluding root).
    pub fn query_selector(&self, tree: &DomTree, root: NodeId) -> Option<NodeId> {
        self.dfs_first(tree, root)
    }

    /// Return all matching nodes.
    pub fn query_selector_all(&self, tree: &DomTree, root: NodeId) -> Vec<NodeId> {
        let mut results = Vec::new();
        self.dfs_all(tree, root, &mut results);
        results
    }

    fn dfs_first(&self, tree: &DomTree, node: NodeId) -> Option<NodeId> {
        if tree.nodes[node].is_element() {
            for sel in &self.selectors {
                if sel.matches(tree, node) {
                    return Some(node);
                }
            }
        }

        let mut child = tree.nodes[node].first_child;
        while let Some(id) = child {
            if let Some(found) = self.dfs_first(tree, id) {
                return Some(found);
            }
            child = tree.nodes[id].next_sibling;
        }
        None
    }

    fn dfs_all(&self, tree: &DomTree, node: NodeId, results: &mut Vec<NodeId>) {
        if tree.nodes[node].is_element() {
            for sel in &self.selectors {
                if sel.matches(tree, node) {
                    results.push(node);
                    break;
                }
            }
        }

        let mut child = tree.nodes[node].first_child;
        while let Some(id) = child {
            self.dfs_all(tree, id, results);
            child = tree.nodes[id].next_sibling;
        }
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

fn parse_single_selector(input: &str) -> Selector {
    let mut parts: Vec<(SimpleSelector, Option<Combinator>)> = Vec::new();
    let mut current = SimpleSelector::default();
    let mut pending_combinator: Option<Combinator> = None;

    let tokens = tokenize_selector(input);

    for token in tokens {
        match token {
            SelToken::Tag(t) => {
                if !is_default(&current) {
                    parts.push((current, pending_combinator.take()));
                    current = SimpleSelector::default();
                }
                current.tag = Some(t.to_ascii_uppercase());
            }
            SelToken::Id(id) => {
                current.id = Some(id);
            }
            SelToken::Class(cls) => {
                current.classes.push(cls);
            }
            SelToken::Attr(name, val) => {
                current.attr_name = Some(name);
                current.attr_value = val;
            }
            SelToken::Pseudo(pseudo) => {
                match pseudo.as_str() {
                    "first-child" => current.first_child = true,
                    "last-child" => current.last_child = true,
                    _ => {}
                }
            }
            SelToken::Combinator(c) => {
                pending_combinator = Some(c);
            }
        }
    }

    if !is_default(&current) {
        parts.push((current, pending_combinator));
    }

    Selector { parts }
}

fn is_default(s: &SimpleSelector) -> bool {
    s.tag.is_none()
        && s.id.is_none()
        && s.classes.is_empty()
        && s.attr_name.is_none()
        && !s.first_child
        && !s.last_child
}

#[derive(Debug)]
enum SelToken {
    Tag(String),
    Id(String),
    Class(String),
    Attr(String, Option<String>),
    Pseudo(String),
    Combinator(Combinator),
}

fn tokenize_selector(input: &str) -> Vec<SelToken> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        match chars[i] {
            ' ' | '\t' | '\n' | '\r' => {
                if let Some(last) = tokens.last() {
                    match last {
                        SelToken::Combinator(_) => {}
                        _ => tokens.push(SelToken::Combinator(Combinator::Descendant)),
                    }
                }
                i += 1;
            }
            '>' => {
                tokens.push(SelToken::Combinator(Combinator::Child));
                i += 1;
            }
            '#' => {
                i += 1;
                let id = read_ident(&chars, &mut i);
                tokens.push(SelToken::Id(id));
            }
            '.' => {
                i += 1;
                let cls = read_ident(&chars, &mut i);
                tokens.push(SelToken::Class(cls));
            }
            '[' => {
                i += 1;
                let name = read_until(&chars, &mut i, |c| c == '=' || c == ']');
                let val = if i < len && chars[i] == '=' {
                    i += 1;
                    let v = read_until(&chars, &mut i, |c| c == ']');
                    i += 1;
                    Some(v)
                } else {
                    i += 1;
                    None
                };
                tokens.push(SelToken::Attr(name, val));
            }
            ':' => {
                i += 1;
                let pseudo = read_ident(&chars, &mut i);
                tokens.push(SelToken::Pseudo(pseudo));
            }
            c if c.is_ascii_alphabetic() || c == '_' || c == '-' || c == '*' => {
                let tag = read_ident(&chars, &mut i);
                tokens.push(SelToken::Tag(tag));
            }
            _ => {
                i += 1;
            }
        }
    }

    // Remove trailing descendant combinator.
    if matches!(tokens.last(), Some(SelToken::Combinator(Combinator::Descendant))) {
        tokens.pop();
    }

    tokens
}

fn read_ident(chars: &[char], pos: &mut usize) -> String {
    let start = *pos;
    while *pos < chars.len()
        && (chars[*pos].is_ascii_alphanumeric()
            || chars[*pos] == '_'
            || chars[*pos] == '-')
    {
        *pos += 1;
    }
    chars[start..*pos].iter().collect()
}

fn read_until(chars: &[char], pos: &mut usize, predicate: impl Fn(char) -> bool) -> String {
    let start = *pos;
    while *pos < chars.len() && !predicate(chars[*pos]) {
        *pos += 1;
    }
    chars[start..*pos].iter().collect()
}
