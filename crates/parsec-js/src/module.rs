// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! What a module imports and exports: the spec's ImportEntry and
//! ExportEntry records (ECMA-262 16.2.1.6), read from the tree.
//!
//! A bundler, a linter or a code-search tool can build a module graph
//! from these without resolving a file: [`ModuleRecord::requested_modules`]
//! are the edges, and the entries say which names cross each one. The
//! entries are sorted as ParseModule sorts them: exports of local names,
//! re-exports of one name (`export { a } from "m"`, and an imported name
//! exported again), and `export * from "m"`.
//!
//! ```
//! use parsec_js::module::{ImportName, ModuleRecord};
//! use parsec_js::{Options, SourceType, parse};
//!
//! let src = r#"import { readFile as read } from "node:fs";
//! import config from "./config.json" with { type: "json" };
//! export { read };
//! export default function main() {}"#;
//! let options = Options { source_type: SourceType::Module, ..Options::default() };
//! let ast = parse(src, &options).unwrap();
//! let record = ModuleRecord::new(&ast, src);
//!
//! let specifiers: Vec<_> = record.requested_modules.iter().map(|m| m.specifier.as_str()).collect();
//! assert_eq!(specifiers, ["node:fs", "./config.json"]);
//! assert_eq!(record.requested_modules[1].attributes, [("type".to_string(), "json".to_string())]);
//! assert_eq!(record.import_entries[0].import_name, ImportName::Name("readFile".into()));
//! assert_eq!(record.import_entries[0].local_name, "read");
//! // `read` is imported, so exporting it is a re-export of node:fs's
//! // readFile.
//! assert_eq!(record.indirect_export_entries[0].export_name.as_deref(), Some("read"));
//! assert_eq!(record.local_export_entries[0].local_name.as_deref(), Some("main"));
//! ```
//!
//! Strings are Rust strings: a specifier with a lone surrogate in it,
//! `"\uD800"`, has U+FFFD in its place. Export and import names can't
//! have one; the parser rejects them.

use std::collections::HashMap;

use parsec_base::Span;

use crate::ast::{Ast, Node, NodeId};
use crate::text;

/// A module a module asks for: its specifier and import attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleRequest {
    /// The string after `from`, or after `import`, decoded.
    pub specifier: String,
    /// The `with { key: "value" }` attributes, sorted by key as the spec
    /// sorts them (by UTF-16 code units).
    pub attributes: Vec<(String, String)>,
    /// Where the specifier's string literal first appears.
    pub span: Span,
}

/// What an ImportEntry imports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportName {
    /// An export by name; `default` for a default import.
    Name(String),
    /// The whole namespace, `import * as ns`.
    NamespaceObject,
}

/// One name a module imports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportEntry {
    /// Its index in [`ModuleRecord::requested_modules`].
    pub module_request: usize,
    pub import_name: ImportName,
    /// The name it's bound to here.
    pub local_name: String,
    /// The import specifier it comes from.
    pub span: Span,
}

/// What an ExportEntry re-exports from another module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportImportName {
    /// One export by name.
    Name(String),
    /// `export * as ns from "m"`: the whole namespace.
    All,
    /// `export * from "m"`: every export but `default`.
    AllButDefault,
}

/// One name a module exports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportEntry {
    /// The name it's exported as; `None` for `export * from "m"`.
    pub export_name: Option<String>,
    /// For a re-export, its index in [`ModuleRecord::requested_modules`].
    pub module_request: Option<usize>,
    pub import_name: Option<ExportImportName>,
    /// The local binding it exports; `*default*` for `export default`
    /// with an expression or an anonymous declaration.
    pub local_name: Option<String>,
    /// The node it comes from: a specifier, a declared name, an `export
    /// default` or an `export *`.
    pub span: Span,
}

/// A module's imports and exports, as ParseModule records them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleRecord {
    /// Each module requested, once, in the order first requested; two
    /// requests are one when their specifiers and attributes are the
    /// same.
    pub requested_modules: Vec<ModuleRequest>,
    pub import_entries: Vec<ImportEntry>,
    /// Exports of the module's own bindings, and of an imported
    /// namespace exported again.
    pub local_export_entries: Vec<ExportEntry>,
    /// Re-exports of one name from another module.
    pub indirect_export_entries: Vec<ExportEntry>,
    /// `export * from "m"`.
    pub star_export_entries: Vec<ExportEntry>,
}

impl ModuleRecord {
    /// The records for `ast`, parsed from `src`. A script's are empty.
    pub fn new(ast: &Ast, src: &str) -> ModuleRecord {
        let mut record = ModuleRecord::default();
        let mut requests = Requests::new();
        let Node::Program { body, .. } = *ast.node(ast.root()) else {
            unreachable!("the root is a program")
        };
        let mut exports = Vec::new();
        for &item in ast.get(body) {
            match *ast.node(item) {
                Node::ImportDeclaration {
                    specifiers,
                    source,
                    attributes,
                } => {
                    let request = record.request(&mut requests, ast, src, source, attributes);
                    for &spec in ast.get(specifiers) {
                        let (import_name, local) = match *ast.node(spec) {
                            Node::ImportSpecifier { imported, local } => {
                                (ImportName::Name(export_name(ast, src, imported)), local)
                            }
                            Node::ImportDefaultSpecifier { local } => {
                                (ImportName::Name("default".into()), local)
                            }
                            Node::ImportNamespaceSpecifier { local } => {
                                (ImportName::NamespaceObject, local)
                            }
                            _ => continue,
                        };
                        record.import_entries.push(ImportEntry {
                            module_request: request,
                            import_name,
                            local_name: name(ast, src, local),
                            span: ast.span(spec),
                        });
                    }
                }
                Node::ExportNamed {
                    declaration: Some(declaration),
                    ..
                } => {
                    for id in declared(ast, declaration) {
                        let n = name(ast, src, id);
                        exports.push(local_export(Some(n.clone()), n, ast.span(id)));
                    }
                }
                Node::ExportNamed {
                    declaration: None,
                    specifiers,
                    source,
                    attributes,
                } => {
                    let request =
                        source.map(|s| record.request(&mut requests, ast, src, s, attributes));
                    for &spec in ast.get(specifiers) {
                        let Node::ExportSpecifier { local, exported } = *ast.node(spec) else {
                            continue;
                        };
                        let local_name = export_name(ast, src, local);
                        let exported = Some(export_name(ast, src, exported));
                        let span = ast.span(spec);
                        exports.push(match request {
                            None => local_export(exported, local_name, span),
                            Some(r) => ExportEntry {
                                export_name: exported,
                                module_request: Some(r),
                                import_name: Some(ExportImportName::Name(local_name)),
                                local_name: None,
                                span,
                            },
                        });
                    }
                }
                Node::ExportDefault { declaration } => {
                    let own = match *ast.node(declaration) {
                        Node::Function(f) if f.kind == crate::ast::FunctionKind::Declaration => {
                            f.id
                        }
                        Node::Class(c) if c.declaration => c.id,
                        _ => None,
                    };
                    let local =
                        own.map_or_else(|| "*default*".to_string(), |id| name(ast, src, id));
                    exports.push(local_export(Some("default".into()), local, ast.span(item)));
                }
                Node::ExportAll {
                    exported,
                    source,
                    attributes,
                } => {
                    let request = record.request(&mut requests, ast, src, source, attributes);
                    let export_name = exported.map(|e| export_name(ast, src, e));
                    let import_name = if export_name.is_some() {
                        ExportImportName::All
                    } else {
                        ExportImportName::AllButDefault
                    };
                    exports.push(ExportEntry {
                        export_name,
                        module_request: Some(request),
                        import_name: Some(import_name),
                        local_name: None,
                        span: ast.span(item),
                    });
                }
                _ => {}
            }
        }
        // Early errors allow each local name to be imported once, so the
        // first entry is the only one.
        let mut by_local: HashMap<&str, usize> = HashMap::new();
        for (i, ie) in record.import_entries.iter().enumerate() {
            by_local.entry(ie.local_name.as_str()).or_insert(i);
        }
        for ee in exports {
            match (&ee.module_request, &ee.import_name) {
                (None, _) => {
                    let imported = ee
                        .local_name
                        .as_deref()
                        .and_then(|l| by_local.get(l))
                        .map(|&i| &record.import_entries[i]);
                    match imported {
                        None
                        | Some(ImportEntry {
                            import_name: ImportName::NamespaceObject,
                            ..
                        }) => record.local_export_entries.push(ee),
                        Some(ie) => {
                            let ImportName::Name(n) = &ie.import_name else {
                                unreachable!("matched above")
                            };
                            let entry = ExportEntry {
                                export_name: ee.export_name,
                                module_request: Some(ie.module_request),
                                import_name: Some(ExportImportName::Name(n.clone())),
                                local_name: None,
                                span: ee.span,
                            };
                            record.indirect_export_entries.push(entry);
                        }
                    }
                }
                (Some(_), Some(ExportImportName::AllButDefault)) => {
                    record.star_export_entries.push(ee)
                }
                (Some(_), _) => record.indirect_export_entries.push(ee),
            }
        }
        record
    }

    /// The index of the request for `source` with `attributes`, added if
    /// it's new.
    fn request(
        &mut self,
        requests: &mut Requests,
        ast: &Ast,
        src: &str,
        source: NodeId,
        attributes: crate::ast::List,
    ) -> usize {
        let mut attrs: Vec<(Vec<u16>, Vec<u16>)> = ast
            .get(attributes)
            .iter()
            .filter_map(|&a| match *ast.node(a) {
                Node::ImportAttribute { key, value } => {
                    let key = match ast.node(key) {
                        Node::Literal(_) => string_units(ast, src, key),
                        _ => name(ast, src, key).encode_utf16().collect(),
                    };
                    Some((key, string_units(ast, src, value)))
                }
                _ => None,
            })
            .collect();
        attrs.sort();
        // Compared as UTF-16, so two lone surrogates that both read as
        // U+FFFD stay two requests.
        let key = (string_units(ast, src, source), attrs);
        if let Some(&i) = requests.get(&key) {
            return i;
        }
        let (specifier, attrs) = &key;
        self.requested_modules.push(ModuleRequest {
            specifier: String::from_utf16_lossy(specifier),
            attributes: attrs
                .iter()
                .map(|(k, v)| (String::from_utf16_lossy(k), String::from_utf16_lossy(v)))
                .collect(),
            span: ast.span(source),
        });
        let i = self.requested_modules.len() - 1;
        requests.insert(key, i);
        i
    }
}

fn local_export(export_name: Option<String>, local_name: String, span: Span) -> ExportEntry {
    ExportEntry {
        export_name,
        module_request: None,
        import_name: None,
        local_name: Some(local_name),
        span,
    }
}

/// An identifier's name, escapes decoded.
fn name(ast: &Ast, src: &str, id: NodeId) -> String {
    text::ident_name(ast.span(id).text(src)).into_owned()
}

/// A string literal's value.
/// A request's specifier and sorted attributes, as UTF-16, to its index.
type Requests = HashMap<(Vec<u16>, Vec<(Vec<u16>, Vec<u16>)>), usize>;

fn string(ast: &Ast, src: &str, id: NodeId) -> String {
    String::from_utf16_lossy(&string_units(ast, src, id))
}

/// A string literal's value, as the UTF-16 code units the spec compares.
fn string_units(ast: &Ast, src: &str, id: NodeId) -> Vec<u16> {
    let span = ast.span(id);
    let body = Span::new(span.start + 1, span.end - 1).text(src);
    text::string_value(body)
}

/// A ModuleExportName: an identifier name or a string.
fn export_name(ast: &Ast, src: &str, id: NodeId) -> String {
    match ast.node(id) {
        Node::Literal(_) => string(ast, src, id),
        _ => name(ast, src, id),
    }
}

/// The identifiers a declaration after `export` declares, in order.
fn declared(ast: &Ast, declaration: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    match *ast.node(declaration) {
        Node::VariableDeclaration { declarations, .. } => {
            let mut stack: Vec<NodeId> = ast
                .get(declarations)
                .iter()
                .rev()
                .filter_map(|&d| match *ast.node(d) {
                    Node::VariableDeclarator { id, .. } => Some(id),
                    _ => None,
                })
                .collect();
            while let Some(id) = stack.pop() {
                let before = stack.len();
                match *ast.node(id) {
                    Node::Identifier => out.push(id),
                    Node::ObjectPattern { properties } => {
                        for &p in ast.get(properties) {
                            match *ast.node(p) {
                                Node::Property { value, .. } => stack.push(value),
                                Node::Rest { argument } => stack.push(argument),
                                _ => {}
                            }
                        }
                    }
                    Node::ArrayPattern { elements } => {
                        stack.extend(
                            ast.get(elements)
                                .iter()
                                .filter(|&&e| !matches!(ast.node(e), Node::Elision)),
                        );
                    }
                    Node::Rest { argument } => stack.push(argument),
                    Node::AssignmentPattern { left, .. } => stack.push(left),
                    _ => {}
                }
                // In source order: the children just pushed, first on top.
                stack[before..].reverse();
            }
        }
        Node::Function(f) => out.extend(f.id),
        Node::Class(c) => out.extend(c.id),
        _ => {}
    }
    out
}
