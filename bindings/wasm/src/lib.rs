// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Parsec for JavaScript. Names follow the DOM where there's a DOM name
//! for the thing (`textContent`, `outerHTML`, `nextSibling`), so code
//! written against a browser's tree reads the same; selectors are `select`
//! and `selectFirst`.
//!
//! A node is its document plus an index, so it stays valid after edits:
//! a removed node can still be read, and put back.

use std::cell::RefCell;
use std::rc::Rc;

use parsec::css::SelectorList;
use parsec::{AttrNs, Kind, NodeId, Ns, Options};
use wasm_bindgen::prelude::*;

type Shared = Rc<RefCell<parsec::Document>>;

#[wasm_bindgen(typescript_custom_section)]
const PARSE_OPTIONS: &str = r#"
/** How to parse. */
export interface ParseOptions {
    /** Parse `<noscript>` as a browser with JavaScript on does. */
    scripting?: boolean;
    /** Keep the tree at most this deep, as Chromium does at 512. */
    maxDepth?: number;
}
"#;

#[wasm_bindgen]
extern "C" {
    /// A plain object, read with the getters below.
    #[wasm_bindgen(typescript_type = "ParseOptions")]
    pub type ParseOptions;
    #[wasm_bindgen(method, getter)]
    fn scripting(this: &ParseOptions) -> Option<bool>;
    #[wasm_bindgen(method, getter, js_name = maxDepth)]
    fn max_depth(this: &ParseOptions) -> Option<u32>;
}

/// A parsed document or fragment.
#[wasm_bindgen]
pub struct Document {
    doc: Shared,
}

/// A node in a [`Document`]: an element, text, a comment, a doctype, or
/// the document itself.
#[wasm_bindgen]
pub struct Node {
    doc: Shared,
    id: NodeId,
}

fn compile(selector: &str) -> Result<SelectorList, JsError> {
    SelectorList::parse(selector).map_err(|e| JsError::new(&format!("{selector:?}: {e}")))
}

fn run(
    html: &str,
    options: Option<&ParseOptions>,
    context: Option<&str>,
) -> Result<Document, JsError> {
    let parse = Options {
        scripting: options.and_then(ParseOptions::scripting).unwrap_or(false),
        budget: None,
        max_depth: options
            .and_then(ParseOptions::max_depth)
            .map(|d| d as usize),
    };
    let doc = match context {
        Some(context) => {
            let context = if context.is_empty() { "div" } else { context };
            parsec::parse_fragment_with(html, Ns::Html, context, parse)
        }
        None => parsec::parse_with(html, parse),
    }
    .map_err(|e| JsError::new(&e.to_string()))?;
    Ok(Document {
        doc: Rc::new(RefCell::new(doc)),
    })
}

/// Parses a whole document, as a browser does.
#[wasm_bindgen]
pub fn parse(html: &str, options: Option<ParseOptions>) -> Result<Document, JsError> {
    run(html, options.as_ref(), None)
}

/// Parses `html` as the contents of a `context` element (`"div"` if not
/// given), as setting `innerHTML` does.
#[wasm_bindgen(js_name = parseFragment)]
pub fn parse_fragment(
    html: &str,
    context: Option<String>,
    options: Option<ParseOptions>,
) -> Result<Document, JsError> {
    run(
        html,
        options.as_ref(),
        Some(context.as_deref().unwrap_or("")),
    )
}

fn node(doc: &Shared, id: Option<NodeId>) -> Option<Node> {
    id.map(|id| Node {
        doc: doc.clone(),
        id,
    })
}

fn nodes(doc: &Shared, ids: Vec<NodeId>) -> Vec<Node> {
    ids.into_iter()
        .map(|id| Node {
            doc: doc.clone(),
            id,
        })
        .collect()
}

fn html_child(doc: &parsec::Document, name: &str) -> Option<NodeId> {
    let html = doc
        .children(doc.root())
        .find(|&n| doc.lower_name(n) == Some("html"))?;
    doc.children(html).find(|&n| {
        matches!(doc.kind(n), Kind::Element { ns: Ns::Html, .. }) && doc.lower_name(n) == Some(name)
    })
}

#[wasm_bindgen]
impl Document {
    /// The document node, or the fragment's root, which holds its nodes.
    #[wasm_bindgen(getter)]
    pub fn root(&self) -> Node {
        Node {
            doc: self.doc.clone(),
            id: 0,
        }
    }

    /// The `<html>` element.
    #[wasm_bindgen(getter, js_name = documentElement)]
    pub fn document_element(&self) -> Option<Node> {
        let id = {
            let doc = self.doc.borrow();
            doc.children(0)
                .find(|&n| matches!(doc.kind(n), Kind::Element { .. }))
        };
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter)]
    pub fn head(&self) -> Option<Node> {
        let id = html_child(&self.doc.borrow(), "head");
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter)]
    pub fn body(&self) -> Option<Node> {
        let id = html_child(&self.doc.borrow(), "body");
        node(&self.doc, id)
    }

    /// Every element that matches, in document order.
    pub fn select(&self, selector: &str) -> Result<Vec<Node>, JsError> {
        self.root().select(selector)
    }

    /// The first element that matches, found without looking further.
    #[wasm_bindgen(js_name = selectFirst)]
    pub fn select_first(&self, selector: &str) -> Result<Option<Node>, JsError> {
        self.root().select_first(selector)
    }

    /// The whole document as markup.
    #[wasm_bindgen(getter)]
    pub fn html(&self) -> String {
        self.doc.borrow().inner_html(0)
    }

    /// Every text node's text, in tree order.
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> String {
        self.doc.borrow().text(0)
    }
}

#[wasm_bindgen]
impl Node {
    /// The DOM's `nodeType`: 1 for an element, 3 text, 8 a comment,
    /// 9 the document, 10 a doctype, 11 a fragment.
    #[wasm_bindgen(getter, js_name = nodeType)]
    pub fn node_type(&self) -> u8 {
        match self.doc.borrow().kind(self.id) {
            Kind::Element { .. } => 1,
            Kind::Text(_) => 3,
            Kind::Cdata(_) => 4,
            Kind::ProcessingInstruction { .. } => 7,
            Kind::Comment(_) => 8,
            Kind::Document => 9,
            Kind::Doctype { .. } => 10,
            Kind::Fragment => 11,
        }
    }

    /// The DOM's `nodeName`: `DIV` for an HTML element, the name as
    /// written for SVG and MathML (`foreignObject`), `#text`, `#comment`,
    /// `#document`, or the doctype's name.
    #[wasm_bindgen(getter, js_name = nodeName)]
    pub fn node_name(&self) -> String {
        match self.doc.borrow().kind(self.id) {
            Kind::Element { ns: Ns::Html, name } => name.to_ascii_uppercase(),
            Kind::Element { name, .. } => name.to_string(),
            Kind::Text(_) => "#text".into(),
            Kind::Cdata(_) => "#cdata-section".into(),
            Kind::ProcessingInstruction { target, .. } => target.to_string(),
            Kind::Comment(_) => "#comment".into(),
            Kind::Document => "#document".into(),
            Kind::Doctype { name, .. } => name.to_string(),
            Kind::Fragment => "#document-fragment".into(),
        }
    }

    /// An element's name, lowercase for HTML: `div`, `foreignObject`.
    #[wasm_bindgen(getter, js_name = localName)]
    pub fn local_name(&self) -> Option<String> {
        match self.doc.borrow().kind(self.id) {
            Kind::Element { name, .. } => Some(name.to_string()),
            _ => None,
        }
    }

    #[wasm_bindgen(getter, js_name = parentNode)]
    pub fn parent_node(&self) -> Option<Node> {
        let id = self.doc.borrow().parent(self.id);
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter, js_name = firstChild)]
    pub fn first_child(&self) -> Option<Node> {
        let id = self.doc.borrow().first_child(self.id);
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter, js_name = lastChild)]
    pub fn last_child(&self) -> Option<Node> {
        let id = self.doc.borrow().last_child(self.id);
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter, js_name = nextSibling)]
    pub fn next_sibling(&self) -> Option<Node> {
        let id = self.doc.borrow().next_sibling(self.id);
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter, js_name = previousSibling)]
    pub fn previous_sibling(&self) -> Option<Node> {
        let id = self.doc.borrow().prev_sibling(self.id);
        node(&self.doc, id)
    }

    /// Every child, text and comments included.
    #[wasm_bindgen(getter, js_name = childNodes)]
    pub fn child_nodes(&self) -> Vec<Node> {
        let ids = self.doc.borrow().children(self.id).collect();
        nodes(&self.doc, ids)
    }

    /// The children that are elements.
    #[wasm_bindgen(getter)]
    pub fn children(&self) -> Vec<Node> {
        let ids = {
            let doc = self.doc.borrow();
            doc.children(self.id)
                .filter(|&n| matches!(doc.kind(n), Kind::Element { .. }))
                .collect()
        };
        nodes(&self.doc, ids)
    }

    /// A template element's contents, which selectors and `childNodes`
    /// don't reach, as in a browser.
    #[wasm_bindgen(getter)]
    pub fn content(&self) -> Option<Node> {
        let id = self.doc.borrow().template_contents(self.id);
        node(&self.doc, id)
    }

    /// The elements under this node that match, in document order.
    /// Combinators look past it, as `querySelectorAll` does.
    pub fn select(&self, selector: &str) -> Result<Vec<Node>, JsError> {
        let list = compile(selector)?;
        let ids = list.select(&self.doc.borrow(), self.id);
        Ok(nodes(&self.doc, ids))
    }

    #[wasm_bindgen(js_name = selectFirst)]
    pub fn select_first(&self, selector: &str) -> Result<Option<Node>, JsError> {
        let list = compile(selector)?;
        let id = list.select_first(&self.doc.borrow(), self.id);
        Ok(node(&self.doc, id))
    }

    /// Whether this element matches.
    pub fn matches(&self, selector: &str) -> Result<bool, JsError> {
        let list = compile(selector)?;
        let doc = self.doc.borrow();
        Ok(matches!(doc.kind(self.id), Kind::Element { .. }) && list.matches(&doc, self.id, None))
    }

    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, name: &str) -> Option<String> {
        let doc = self.doc.borrow();
        let html = matches!(doc.kind(self.id), Kind::Element { ns: Ns::Html, .. });
        let name = if html {
            name.to_ascii_lowercase()
        } else {
            name.to_string()
        };
        doc.attrs_written(self.id)
            .find(|&(_, n, _)| n == name)
            .map(|(_, _, v)| v.to_string())
    }

    #[wasm_bindgen(js_name = hasAttribute)]
    pub fn has_attribute(&self, name: &str) -> bool {
        self.get_attribute(name).is_some()
    }

    /// The attributes' names as written, in source order (`xlink:href`).
    #[wasm_bindgen(js_name = getAttributeNames)]
    pub fn get_attribute_names(&self) -> Vec<String> {
        self.doc
            .borrow()
            .attrs_written(self.id)
            .map(|(_, n, _)| n.to_string())
            .collect()
    }

    /// Sets an attribute, as `setAttribute` does. Only elements have them.
    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, name: &str, value: &str) -> Result<(), JsError> {
        let mut doc = self.doc.borrow_mut();
        if !matches!(doc.kind(self.id), Kind::Element { .. }) {
            return Err(JsError::new("only an element has attributes"));
        }
        doc.set_attr(self.id, name, value);
        Ok(())
    }

    /// Removes an attribute; false if there wasn't one.
    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, name: &str) -> bool {
        self.doc.borrow_mut().remove_attr(self.id, name)
    }

    /// The `id` attribute, or `""`.
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> String {
        let doc = self.doc.borrow();
        doc.attrs(self.id)
            .find(|&(ns, n, _)| ns == AttrNs::None && n == "id")
            .map_or(String::new(), |(_, _, v)| v.to_string())
    }

    /// The text of this node and everything under it, as `textContent`.
    #[wasm_bindgen(getter, js_name = textContent)]
    pub fn text_content(&self) -> String {
        self.doc.borrow().text(self.id)
    }

    /// The node and its contents as markup.
    #[wasm_bindgen(getter, js_name = outerHTML)]
    pub fn outer_html(&self) -> String {
        self.doc.borrow().outer_html(self.id)
    }

    /// The contents as markup.
    #[wasm_bindgen(getter, js_name = innerHTML)]
    pub fn inner_html(&self) -> String {
        self.doc.borrow().inner_html(self.id)
    }

    /// Takes the node out of the tree. It can still be read, and put back.
    pub fn remove(&self) {
        self.doc.borrow_mut().remove(self.id);
    }

    /// Moves `child`, a node of the same document, to the end of this
    /// one's children.
    #[wasm_bindgen(js_name = appendChild)]
    pub fn append_child(&self, child: &Node) -> Result<(), JsError> {
        self.insert(child, None)
    }

    /// Moves `child`, a node of the same document, in before `reference`,
    /// one of this node's children.
    #[wasm_bindgen(js_name = insertBefore)]
    pub fn insert_before(&self, child: &Node, reference: &Node) -> Result<(), JsError> {
        self.insert(child, Some(reference))
    }

    /// Whether `other` is this same node.
    #[wasm_bindgen(js_name = isSameNode)]
    pub fn is_same_node(&self, other: &Node) -> bool {
        Rc::ptr_eq(&self.doc, &other.doc) && self.id == other.id
    }
}

impl Node {
    fn insert(&self, child: &Node, reference: Option<&Node>) -> Result<(), JsError> {
        if !Rc::ptr_eq(&self.doc, &child.doc)
            || reference
                .as_ref()
                .is_some_and(|r| !Rc::ptr_eq(&self.doc, &r.doc))
        {
            return Err(JsError::new(
                "nodes can only move within their own document",
            ));
        }
        self.doc
            .borrow_mut()
            .insert_before(self.id, child.id, reference.map(|r| r.id))
            .map_err(|e| JsError::new(&e.to_string()))
    }
}
