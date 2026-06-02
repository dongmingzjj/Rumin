// HTML parser using html5ever
use anyhow::Result;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{RcDom, NodeData as HtmlNodeData, Handle};
use mb_dom::tree::DomTree;
use mb_dom::node::{NodeId, NamedNodeMap};

/// Information about a script element found during parsing
#[derive(Debug, Clone)]
pub struct ScriptInfo {
    pub src: Option<String>,
    pub inline_content: Option<String>,
}

pub struct HtmlParser;

impl HtmlParser {
    /// Parse an HTML string and return a DomTree
    pub fn parse(html: &str, base_url: &str) -> Result<DomTree> {
        let html5dom = parse_document(RcDom::default(), Default::default())
            .from_utf8()
            .one(html.as_bytes());

        let mut tree = DomTree::new();
        // Set URL on existing document node
        if let mb_dom::node::NodeKind::Document(ref mut doc) = tree.nodes[tree.document_node].kind {
            doc.url = base_url.to_string();
        }

        // Copy node IDs to locals to avoid borrow issues
        let html_node = tree.html_node;
        let head_node = tree.head_node;
        let body_node = tree.body_node;

        // Find <html> element in parsed tree
        if let Some(html_elem) = Self::find_element_by_tag(&html5dom.document, "html") {
            // Process the <html> element's children to populate head and body
            let html_children = html_elem.children.borrow();
            for child in html_children.iter() {
                if let HtmlNodeData::Element { ref name, .. } = child.data {
                    let tag = name.local.as_ref();
                    match tag {
                        "head" => {
                            Self::convert_children_into(&mut tree, head_node, child);
                        }
                        "body" => {
                            Self::convert_children_into(&mut tree, body_node, child);
                        }
                        _ => {
                            // Other direct children of <html> (e.g., <script>)
                            Self::convert_node_into(&mut tree, html_node, child);
                        }
                    }
                } else {
                    // Text/comment children of <html>
                    Self::convert_node_into(&mut tree, html_node, child);
                }
            }
        } else {
            // No <html> element — convert all children of the document directly
            Self::convert_children_into(&mut tree, html_node, &html5dom.document);
        }

        // Process <script> tags for script info
        // (collect_scripts is called by callers when needed)

        Ok(tree)
    }

    /// Collect script info from the tree (for external use)
    pub fn collect_scripts(tree: &DomTree) -> Vec<ScriptInfo> {
        let mut scripts = Vec::new();
        Self::walk_for_scripts(tree, tree.html_node, &mut scripts);
        scripts
    }

    fn walk_for_scripts(tree: &DomTree, node: NodeId, scripts: &mut Vec<ScriptInfo>) {
        if let Some(tag) = tree.nodes[node].tag_name() {
            if tag.eq_ignore_ascii_case("script") {
                let src = tree.nodes[node].element_data()
                    .and_then(|el| el.attributes.get_value("src"))
                    .map(|s| s.to_string());

                let mut inline_content = String::new();
                let mut child = tree.nodes[node].first_child;
                while let Some(cid) = child {
                    if let Some(text) = tree.nodes[cid].text_data() {
                        inline_content.push_str(&text.data);
                    }
                    child = tree.nodes[cid].next_sibling;
                }

                scripts.push(ScriptInfo {
                    src,
                    inline_content: if inline_content.is_empty() { None } else { Some(inline_content) },
                });
            }
        }

        let mut child = tree.nodes[node].first_child;
        while let Some(cid) = child {
            Self::walk_for_scripts(tree, cid, scripts);
            child = tree.nodes[cid].next_sibling;
        }
    }

    /// Find an element by tag name in the html5ever tree (recursively)
    fn find_element_by_tag(handle: &Handle, tag: &str) -> Option<Handle> {
        let children = handle.children.borrow();
        for child in children.iter() {
            if let HtmlNodeData::Element { ref name, .. } = child.data {
                if name.local.as_ref().eq_ignore_ascii_case(tag) {
                    return Some(child.clone());
                }
            }
            if let Some(found) = Self::find_element_by_tag(child, tag) {
                return Some(found);
            }
        }
        None
    }

    /// Convert an html5ever node and append it to parent in our DomTree
    fn convert_node_into(tree: &mut DomTree, parent: NodeId, handle: &Handle) {
        match handle.data {
            HtmlNodeData::Document => {
                // Process children directly under parent
                Self::convert_children_into(tree, parent, handle);
            }
            HtmlNodeData::Element { ref name, ref attrs, .. } => {
                let tag = name.local.as_ref().to_lowercase();
                let attributes = Self::extract_attributes(attrs);
                let node_id = tree.create_element_with_attrs(&tag, attributes);
                tree.append_child(parent, node_id);

                // Convert children
                Self::convert_children_into(tree, node_id, handle);
            }
            HtmlNodeData::Text { ref contents } => {
                let text = contents.borrow().to_string();
                let node_id = tree.create_text(&text);
                tree.append_child(parent, node_id);
            }
            HtmlNodeData::Comment { ref contents } => {
                let node_id = tree.create_comment(contents);
                tree.append_child(parent, node_id);
            }
            HtmlNodeData::Doctype { .. } | HtmlNodeData::ProcessingInstruction { .. } => {}
        }
    }

    /// Convert all children of an html5ever node
    fn convert_children_into(tree: &mut DomTree, parent: NodeId, handle: &Handle) {
        let children = handle.children.borrow();
        for child in children.iter() {
            Self::convert_node_into(tree, parent, child);
        }
    }

    /// Extract attributes from html5ever's attribute list
    fn extract_attributes(attrs: &std::cell::RefCell<Vec<html5ever::Attribute>>) -> NamedNodeMap {
        let attrs = attrs.borrow();
        let mut result = NamedNodeMap::new();
        for attr in attrs.iter() {
            let name = attr.name.local.as_ref().to_string();
            let value = attr.value.to_string();
            result.set(name, value);
        }
        result
    }
}
