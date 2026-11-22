// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Parsec's tree and html5ever's, each as the html5lib-tests tree dump,
//! so the two can be compared line for line.

use html5ever::tendril::TendrilSink;
use html5ever::{local_name, ns};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use parsec::{Options, SerializeOptions};

/// Parsec's dump of `html`, parsed as html5ever is: scripting off, and
/// `<template shadowrootmode>` an ordinary template, as html5ever has no
/// declarative shadow roots.
pub fn parsec(html: &str) -> String {
    let doc = parsec::parse_with(
        html,
        Options {
            scripting: false,
            declarative_shadow_roots: Some(false),
            ..Options::default()
        },
    )
    .unwrap_or_else(|e| panic!("{e}"));
    doc.serialize_with(
        doc.root(),
        &SerializeOptions {
            html5test: true,
            ..SerializeOptions::default()
        },
    )
}

/// html5ever's dump of `html`, with scripting off, as bench/ parses it.
pub fn html5ever(html: &str) -> String {
    let mut opts = html5ever::ParseOpts::default();
    opts.tree_builder.scripting_enabled = false;
    let dom = html5ever::parse_document(RcDom::default(), opts).one(html);
    let mut out = String::new();
    for child in dom.document.children.borrow().iter() {
        dump(child, 0, &mut out);
    }
    out
}

fn dump(node: &Handle, depth: usize, out: &mut String) {
    let line = |out: &mut String, depth: usize| {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("| ");
        out.push_str(&"  ".repeat(depth));
    };
    match &node.data {
        NodeData::Document => {}
        NodeData::Doctype {
            name,
            public_id,
            system_id,
        } => {
            line(out, depth);
            if public_id.is_empty() && system_id.is_empty() {
                out.push_str(&format!("<!DOCTYPE {name}>"));
            } else {
                out.push_str(&format!(
                    "<!DOCTYPE {name} \"{public_id}\" \"{system_id}\">"
                ));
            }
        }
        NodeData::Text { contents } => {
            line(out, depth);
            out.push_str(&format!("\"{}\"", contents.borrow()));
        }
        NodeData::Comment { contents } => {
            line(out, depth);
            out.push_str(&format!("<!-- {contents} -->"));
        }
        NodeData::ProcessingInstruction { target, contents } => {
            line(out, depth);
            out.push_str(&format!("<?{target} {contents}?>"));
        }
        NodeData::Element {
            name,
            attrs,
            template_contents,
            ..
        } => {
            let prefix = match name.ns {
                ns!(svg) => "svg ",
                ns!(mathml) => "math ",
                _ => "",
            };
            line(out, depth);
            out.push_str(&format!("<{prefix}{}>", name.local));
            let mut list: Vec<String> = attrs
                .borrow()
                .iter()
                .map(|a| {
                    let local = &a.name.local;
                    let name = match a.name.ns {
                        ns!(xlink) => format!("xlink {local}"),
                        ns!(xml) => format!("xml {local}"),
                        ns!(xmlns) if *local == local_name!("xmlns") => "xmlns".to_string(),
                        ns!(xmlns) => format!("xmlns {local}"),
                        _ => local.to_string(),
                    };
                    format!("{name}=\"{}\"", a.value)
                })
                .collect();
            list.sort();
            for a in list {
                line(out, depth + 1);
                out.push_str(&a);
            }
            if let Some(contents) = template_contents.borrow().as_ref() {
                line(out, depth + 1);
                out.push_str("content");
                for child in contents.children.borrow().iter() {
                    dump(child, depth + 2, out);
                }
            }
        }
    }
    for child in node.children.borrow().iter() {
        dump(child, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dumps_agree_where_the_parsers_do() {
        for html in [
            "<!DOCTYPE html><p class=a id=b>one<b>two</p>three",
            "<table><td>x<template><tr><td>y</template></table>",
            "<svg viewBox='0 0 1 1'><a xlink:href=#x><foreignObject><p>z</svg><math><mi>",
            "<!-- c --><title>&amp;</title><!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01//EN\" \"\">",
            "<noscript><p>shown</p></noscript>",
        ] {
            assert_eq!(parsec(html), html5ever(html), "{html}");
        }
    }

    #[test]
    fn the_dump_is_html5lib_s() {
        assert_eq!(
            html5ever("<p a=1>x"),
            "| <html>\n|   <head>\n|   <body>\n|     <p>\n|       a=\"1\"\n|       \"x\""
        );
    }
}
