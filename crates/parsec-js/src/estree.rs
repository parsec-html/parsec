// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The tree as ESTree JSON, in the shape acorn gives for ECMAScript
//! 2025: the same node types and fields, `start` and `end` as offsets in
//! UTF-16 code units, and `loc` if asked for.
//!
//! Values JSON has no form for are `null`, as acorn's are after
//! `JSON.stringify`: a BigInt's (its digits are in `bigint`), a regular
//! expression's (its parts are in `regex`), and a number too large for a
//! double.
//!
//! The writer keeps its own stack rather than recursing, so a tree of
//! any depth (a chain of a million `.x`s is a million deep) is written
//! without touching the thread's stack.

use std::fmt::Write;

use parsec_base::{LineIndex, Span};

use crate::SourceType;
use crate::ast::{Ast, FunctionKind, List, LiteralKind, Node, NodeId};
use crate::text;

/// What [`Ast::to_estree_with`] writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EstreeOptions {
    /// A `loc` on every node, with lines from 1 and columns from 0 in
    /// UTF-16 code units, as acorn's `locations` option gives.
    pub loc: bool,
}

enum Task {
    Node(NodeId),
    Opt(Option<NodeId>),
    List(List, u32),
    Lit(&'static str),
}

struct Writer<'a> {
    ast: &'a Ast,
    src: &'a str,
    /// `None` when the source is ASCII, and byte offsets are UTF-16 ones.
    lines: Option<LineIndex<'a>>,
    loc: Option<LineIndex<'a>>,
    out: String,
    stack: Vec<Task>,
}

impl Ast {
    /// The tree as ESTree JSON. `src` is the source it was parsed from.
    ///
    /// ```
    /// let src = "a?.b";
    /// let ast = parsec_js::parse_script(src).unwrap();
    /// let json = ast.to_estree(src);
    /// assert!(json.starts_with(r#"{"type":"Program","start":0,"end":4,"body":[{"type":"ExpressionStatement""#));
    /// assert!(json.contains(r#""type":"ChainExpression""#));
    /// ```
    pub fn to_estree(&self, src: &str) -> String {
        self.to_estree_with(src, EstreeOptions::default())
    }

    pub fn to_estree_with(&self, src: &str, options: EstreeOptions) -> String {
        let mut w = Writer {
            ast: self,
            src,
            lines: (!src.is_ascii()).then(|| LineIndex::ecmascript(src)),
            loc: options.loc.then(|| LineIndex::ecmascript(src)),
            out: String::with_capacity(src.len() * 8),
            stack: vec![Task::Node(self.root())],
        };
        while let Some(task) = w.stack.pop() {
            match task {
                Task::Node(id) => w.node(id),
                Task::Opt(Some(id)) => w.node(id),
                Task::Opt(None) => w.out.push_str("null"),
                Task::Lit(s) => w.out.push_str(s),
                Task::List(list, i) => {
                    if i == 0 {
                        w.out.push('[');
                    }
                    if i as usize == list.len() {
                        w.out.push(']');
                        continue;
                    }
                    if i > 0 {
                        w.out.push(',');
                    }
                    w.stack.push(Task::List(list, i + 1));
                    w.stack.push(Task::Node(self.get(list)[i as usize]));
                }
            }
        }
        w.out
    }
}

impl<'a> Writer<'a> {
    fn offset(&self, at: u32) -> u32 {
        match &self.lines {
            None => at,
            Some(lines) => lines.utf16_offset(at),
        }
    }

    fn header(&mut self, ty: &str, span: Span) {
        let (start, end) = (self.offset(span.start), self.offset(span.end));
        let _ = write!(self.out, r#"{{"type":"{ty}","start":{start},"end":{end}"#);
        if let Some(lines) = &self.loc {
            let s = lines.line_col_utf16(span.start);
            let e = lines.line_col_utf16(span.end);
            let _ = write!(
                self.out,
                r#","loc":{{"start":{{"line":{},"column":{}}},"end":{{"line":{},"column":{}}}}}"#,
                s.line,
                s.column - 1,
                e.line,
                e.column - 1
            );
        }
    }

    fn text(&self, span: Span) -> &'a str {
        span.text(self.src)
    }

    /// Writes `id`'s header and scalars, and queues its children.
    fn node(&mut self, id: NodeId) {
        let span = self.ast.span(id);
        let node = *self.ast.node(id);
        let mut tasks: Vec<Task> = Vec::new();
        macro_rules! lit {
            ($s:expr) => {
                tasks.push(Task::Lit($s))
            };
        }
        macro_rules! child {
            ($key:literal, $id:expr) => {{
                tasks.push(Task::Lit(concat!(",\"", $key, "\":")));
                tasks.push(Task::Node($id));
            }};
        }
        macro_rules! opt {
            ($key:literal, $id:expr) => {{
                tasks.push(Task::Lit(concat!(",\"", $key, "\":")));
                tasks.push(Task::Opt($id));
            }};
        }
        macro_rules! list {
            ($key:literal, $list:expr) => {{
                tasks.push(Task::Lit(concat!(",\"", $key, "\":")));
                tasks.push(Task::List($list, 0));
            }};
        }
        let flag = |b: bool| if b { "true" } else { "false" };
        match node {
            Node::Paren { expression } => {
                self.stack.push(Task::Node(expression));
                return;
            }
            Node::Elision => {
                self.out.push_str("null");
                return;
            }
            Node::Program { body, source_type } => {
                self.header("Program", span);
                list!("body", body);
                lit!(match source_type {
                    SourceType::Script => r#","sourceType":"script""#,
                    SourceType::Module => r#","sourceType":"module""#,
                });
            }
            Node::Expression {
                expression,
                directive,
            } => {
                self.header("ExpressionStatement", span);
                child!("expression", expression);
                if directive {
                    let s = self.ast.span(expression);
                    let raw = Span::new(s.start + 1, s.end - 1);
                    self.out.push_str(r#","directive":"#);
                    text::write_json_str(&mut self.out, raw.text(self.src));
                }
            }
            Node::Block { body } => {
                self.header("BlockStatement", span);
                list!("body", body);
            }
            Node::Empty => self.header("EmptyStatement", span),
            Node::Debugger => self.header("DebuggerStatement", span),
            Node::With { object, body } => {
                self.header("WithStatement", span);
                child!("object", object);
                child!("body", body);
            }
            Node::Return { argument } => {
                self.header("ReturnStatement", span);
                opt!("argument", argument);
            }
            Node::Labeled { label, body } => {
                self.header("LabeledStatement", span);
                child!("body", body);
                child!("label", label);
            }
            Node::Break { label } => {
                self.header("BreakStatement", span);
                opt!("label", label);
            }
            Node::Continue { label } => {
                self.header("ContinueStatement", span);
                opt!("label", label);
            }
            Node::If {
                test,
                consequent,
                alternate,
            } => {
                self.header("IfStatement", span);
                child!("test", test);
                child!("consequent", consequent);
                opt!("alternate", alternate);
            }
            Node::Switch {
                discriminant,
                cases,
            } => {
                self.header("SwitchStatement", span);
                child!("discriminant", discriminant);
                list!("cases", cases);
            }
            Node::SwitchCase { test, consequent } => {
                self.header("SwitchCase", span);
                opt!("test", test);
                list!("consequent", consequent);
            }
            Node::Throw { argument } => {
                self.header("ThrowStatement", span);
                child!("argument", argument);
            }
            Node::Try {
                block,
                handler,
                finalizer,
            } => {
                self.header("TryStatement", span);
                child!("block", block);
                opt!("handler", handler);
                opt!("finalizer", finalizer);
            }
            Node::CatchClause { param, body } => {
                self.header("CatchClause", span);
                opt!("param", param);
                child!("body", body);
            }
            Node::While { test, body } => {
                self.header("WhileStatement", span);
                child!("test", test);
                child!("body", body);
            }
            Node::DoWhile { body, test } => {
                self.header("DoWhileStatement", span);
                child!("body", body);
                child!("test", test);
            }
            Node::For {
                init,
                test,
                update,
                body,
            } => {
                self.header("ForStatement", span);
                opt!("init", init);
                opt!("test", test);
                opt!("update", update);
                child!("body", body);
            }
            Node::ForIn { left, right, body } => {
                self.header("ForInStatement", span);
                child!("left", left);
                child!("right", right);
                child!("body", body);
            }
            Node::ForOf {
                left,
                right,
                body,
                is_await,
            } => {
                self.header("ForOfStatement", span);
                lit!(if is_await {
                    r#","await":true"#
                } else {
                    r#","await":false"#
                });
                child!("left", left);
                child!("right", right);
                child!("body", body);
            }
            Node::VariableDeclaration { kind, declarations } => {
                self.header("VariableDeclaration", span);
                list!("declarations", declarations);
                let _ = write!(self.out, r#","kind":"{}""#, kind.as_str());
            }
            Node::VariableDeclarator { id, init } => {
                self.header("VariableDeclarator", span);
                child!("id", id);
                opt!("init", init);
            }
            Node::Function(f) => {
                self.header(
                    match f.kind {
                        FunctionKind::Declaration => "FunctionDeclaration",
                        FunctionKind::Expression => "FunctionExpression",
                        FunctionKind::Arrow => "ArrowFunctionExpression",
                    },
                    span,
                );
                let _ = write!(
                    self.out,
                    r#","expression":{},"generator":{},"async":{}"#,
                    flag(f.expression),
                    flag(f.generator),
                    flag(f.is_async)
                );
                opt!("id", f.id);
                list!("params", f.params);
                child!("body", f.body);
            }
            Node::Class(c) => {
                self.header(
                    if c.declaration {
                        "ClassDeclaration"
                    } else {
                        "ClassExpression"
                    },
                    span,
                );
                opt!("id", c.id);
                opt!("superClass", c.super_class);
                child!("body", c.body);
            }
            Node::ClassBody { body } => {
                self.header("ClassBody", span);
                list!("body", body);
            }
            Node::Method {
                key,
                value,
                kind,
                computed,
                is_static,
            } => {
                self.header("MethodDefinition", span);
                let _ = write!(
                    self.out,
                    r#","static":{},"computed":{},"kind":"{}""#,
                    flag(is_static),
                    flag(computed),
                    kind.as_str()
                );
                child!("key", key);
                child!("value", value);
            }
            Node::Field {
                key,
                value,
                computed,
                is_static,
            } => {
                self.header("PropertyDefinition", span);
                let _ = write!(
                    self.out,
                    r#","static":{},"computed":{}"#,
                    flag(is_static),
                    flag(computed)
                );
                child!("key", key);
                opt!("value", value);
            }
            Node::StaticBlock { body } => {
                self.header("StaticBlock", span);
                list!("body", body);
            }
            Node::Identifier => {
                self.header("Identifier", span);
                self.out.push_str(r#","name":"#);
                let name = text::ident_name(self.text(span));
                text::write_json_str(&mut self.out, &name);
            }
            Node::PrivateIdentifier => {
                self.header("PrivateIdentifier", span);
                self.out.push_str(r#","name":"#);
                let name = text::ident_name(self.text(Span::new(span.start + 1, span.end)));
                text::write_json_str(&mut self.out, &name);
            }
            Node::Literal(kind) => {
                self.literal(kind, span);
                return;
            }
            Node::This => self.header("ThisExpression", span),
            Node::Super => self.header("Super", span),
            Node::Array { elements, .. } => {
                self.header("ArrayExpression", span);
                list!("elements", elements);
            }
            Node::Object { properties, .. } => {
                self.header("ObjectExpression", span);
                list!("properties", properties);
            }
            Node::Property {
                key,
                value,
                kind,
                method,
                shorthand,
                computed,
            } => {
                self.header("Property", span);
                let _ = write!(
                    self.out,
                    r#","method":{},"shorthand":{},"computed":{},"kind":"{}""#,
                    flag(method),
                    flag(shorthand),
                    flag(computed),
                    kind.as_str()
                );
                child!("key", key);
                child!("value", value);
            }
            Node::Template {
                quasis,
                expressions,
            } => {
                self.header("TemplateLiteral", span);
                list!("expressions", expressions);
                list!("quasis", quasis);
            }
            Node::TemplateElement { tail, cooked } => {
                self.header("TemplateElement", span);
                let body = self.text(span);
                self.out.push_str(r#","value":{"raw":"#);
                text::write_json_str(&mut self.out, &text::template_raw(body));
                self.out.push_str(r#","cooked":"#);
                match text::template_cooked(body).filter(|_| cooked) {
                    Some(units) => text::write_json_units(&mut self.out, &units),
                    None => self.out.push_str("null"),
                }
                let _ = write!(self.out, r#"}},"tail":{}"#, flag(tail));
            }
            Node::TaggedTemplate { tag, quasi } => {
                self.header("TaggedTemplateExpression", span);
                child!("tag", tag);
                child!("quasi", quasi);
            }
            Node::Unary { operator, argument } => {
                self.header("UnaryExpression", span);
                let _ = write!(
                    self.out,
                    r#","operator":"{}","prefix":true"#,
                    operator.as_str()
                );
                child!("argument", argument);
            }
            Node::Update {
                operator,
                prefix,
                argument,
            } => {
                self.header("UpdateExpression", span);
                let _ = write!(
                    self.out,
                    r#","operator":"{}","prefix":{}"#,
                    operator.as_str(),
                    flag(prefix)
                );
                child!("argument", argument);
            }
            Node::Binary {
                operator,
                left,
                right,
            } => {
                self.header("BinaryExpression", span);
                let _ = write!(self.out, r#","operator":"{}""#, operator.as_str());
                child!("left", left);
                child!("right", right);
            }
            Node::Logical {
                operator,
                left,
                right,
            } => {
                self.header("LogicalExpression", span);
                let _ = write!(self.out, r#","operator":"{}""#, operator.as_str());
                child!("left", left);
                child!("right", right);
            }
            Node::Assign {
                operator,
                left,
                right,
            } => {
                self.header("AssignmentExpression", span);
                let _ = write!(self.out, r#","operator":"{}""#, operator.as_str());
                child!("left", left);
                child!("right", right);
            }
            Node::Conditional {
                test,
                consequent,
                alternate,
            } => {
                self.header("ConditionalExpression", span);
                child!("test", test);
                child!("consequent", consequent);
                child!("alternate", alternate);
            }
            Node::Call {
                callee,
                arguments,
                optional,
            } => {
                self.header("CallExpression", span);
                let _ = write!(self.out, r#","optional":{}"#, flag(optional));
                child!("callee", callee);
                list!("arguments", arguments);
            }
            Node::New { callee, arguments } => {
                self.header("NewExpression", span);
                child!("callee", callee);
                list!("arguments", arguments);
            }
            Node::Member {
                object,
                property,
                computed,
                optional,
            } => {
                self.header("MemberExpression", span);
                let _ = write!(
                    self.out,
                    r#","computed":{},"optional":{}"#,
                    flag(computed),
                    flag(optional)
                );
                child!("object", object);
                child!("property", property);
            }
            Node::Chain { expression } => {
                self.header("ChainExpression", span);
                child!("expression", expression);
            }
            Node::Sequence { expressions } => {
                self.header("SequenceExpression", span);
                list!("expressions", expressions);
            }
            Node::Yield { argument, delegate } => {
                self.header("YieldExpression", span);
                let _ = write!(self.out, r#","delegate":{}"#, flag(delegate));
                opt!("argument", argument);
            }
            Node::Await { argument } => {
                self.header("AwaitExpression", span);
                child!("argument", argument);
            }
            Node::Spread { argument } => {
                self.header("SpreadElement", span);
                child!("argument", argument);
            }
            Node::MetaProperty { meta, property } => {
                self.header("MetaProperty", span);
                child!("meta", meta);
                child!("property", property);
            }
            Node::Import { source, options } => {
                self.header("ImportExpression", span);
                child!("source", source);
                opt!("options", options);
            }
            Node::ImportDeclaration {
                specifiers,
                source,
                attributes,
            } => {
                self.header("ImportDeclaration", span);
                list!("specifiers", specifiers);
                child!("source", source);
                list!("attributes", attributes);
            }
            Node::ImportSpecifier { imported, local } => {
                self.header("ImportSpecifier", span);
                child!("imported", imported);
                child!("local", local);
            }
            Node::ImportDefaultSpecifier { local } => {
                self.header("ImportDefaultSpecifier", span);
                child!("local", local);
            }
            Node::ImportNamespaceSpecifier { local } => {
                self.header("ImportNamespaceSpecifier", span);
                child!("local", local);
            }
            Node::ImportAttribute { key, value } => {
                self.header("ImportAttribute", span);
                child!("key", key);
                child!("value", value);
            }
            Node::ExportNamed {
                declaration,
                specifiers,
                source,
                attributes,
            } => {
                self.header("ExportNamedDeclaration", span);
                opt!("declaration", declaration);
                list!("specifiers", specifiers);
                opt!("source", source);
                list!("attributes", attributes);
            }
            Node::ExportSpecifier { local, exported } => {
                self.header("ExportSpecifier", span);
                child!("local", local);
                child!("exported", exported);
            }
            Node::ExportDefault { declaration } => {
                self.header("ExportDefaultDeclaration", span);
                child!("declaration", declaration);
            }
            Node::ExportAll {
                exported,
                source,
                attributes,
            } => {
                self.header("ExportAllDeclaration", span);
                opt!("exported", exported);
                child!("source", source);
                list!("attributes", attributes);
            }
            Node::ObjectPattern { properties } => {
                self.header("ObjectPattern", span);
                list!("properties", properties);
            }
            Node::ArrayPattern { elements } => {
                self.header("ArrayPattern", span);
                list!("elements", elements);
            }
            Node::Rest { argument } => {
                self.header("RestElement", span);
                child!("argument", argument);
            }
            Node::AssignmentPattern { left, right } => {
                self.header("AssignmentPattern", span);
                child!("left", left);
                child!("right", right);
            }
        }
        tasks.push(Task::Lit("}"));
        self.stack.extend(tasks.into_iter().rev());
    }

    fn literal(&mut self, kind: LiteralKind, span: Span) {
        self.header("Literal", span);
        let raw = self.text(span);
        self.out.push_str(r#","value":"#);
        match kind {
            LiteralKind::String => {
                let body = Span::new(span.start + 1, span.end - 1).text(self.src);
                text::write_json_units(&mut self.out, &text::string_value(body));
            }
            LiteralKind::Number => text::write_js_number(&mut self.out, text::number_value(raw)),
            LiteralKind::Boolean | LiteralKind::Null => self.out.push_str(raw),
            LiteralKind::BigInt | LiteralKind::RegExp => self.out.push_str("null"),
        }
        self.out.push_str(r#","raw":"#);
        text::write_json_str(&mut self.out, raw);
        match kind {
            LiteralKind::BigInt => {
                self.out.push_str(r#","bigint":"#);
                text::write_json_str(&mut self.out, &text::bigint_decimal(raw));
            }
            LiteralKind::RegExp => {
                let slash = raw.rfind('/').unwrap_or(0);
                self.out.push_str(r#","regex":{"pattern":"#);
                text::write_json_str(&mut self.out, raw.get(1..slash).unwrap_or(""));
                self.out.push_str(r#","flags":"#);
                text::write_json_str(&mut self.out, &raw[slash + 1..]);
                self.out.push('}');
            }
            _ => {}
        }
        self.out.push('}');
    }
}
