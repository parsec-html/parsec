// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Scopes and bindings: which declaration each name in a program refers
//! to, worked out from the tree alone.
//!
//! [`Scopes::analyze`] walks an [`Ast`] once and builds the scopes the
//! spec's environments would be, from the global or module scope down
//! through functions, blocks, classes, `catch` clauses, class static
//! blocks and the implicit function around each class field's
//! initializer. Each declaration becomes a [`Binding`] in the scope it
//! lives in: `var` and function declarations hoisted to their function,
//! with Annex B's function-in-block semantics in sloppy code; `let`,
//! `const` and `class` in their block. Each IdentifierReference becomes a
//! [`Reference`], resolved to its binding or left free.
//!
//! Nothing runs, so two things the analysis can't see are marked rather
//! than guessed: a scope that holds a direct `eval` call, or any scope
//! around one, is [`Scope::dynamic`], since the eval'd code can read and
//! add names there; and the body of a `with` gets a [`ScopeKind::With`]
//! scope, dynamic too, since any name in it may be a property of the
//! object.
//!
//! The names declared twice where the spec forbids it come back as
//! [`Scopes::diagnostics`], with codes `duplicate-declaration` and
//! `lexical-var-conflict`.
//!
//! ```
//! use parsec_js::scope::{BindingKind, Scopes};
//! use parsec_js::parse_script;
//!
//! let src = "let total = 0; for (const x of items) total += x; console.log(total);";
//! let ast = parse_script(src).unwrap();
//! let scopes = Scopes::analyze(&ast, src);
//! assert_eq!(scopes.free_names(), ["items", "console"]);
//! let total = scopes.references().iter().find(|r| r.name(&ast, src) == "total").unwrap();
//! let binding = scopes.binding(total.binding.unwrap());
//! assert_eq!(binding.kind, BindingKind::Let);
//! assert!(scopes.diagnostics().is_empty());
//! ```
//!
//! The walk keeps its own stack, so a tree of any depth is analyzed
//! without touching the thread's.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use parsec_base::Diagnostic;

use crate::SourceType;
use crate::ast::{AssignOp, Ast, Class, Function, FunctionKind, List, Node, NodeId, VarKind};
use crate::text;

/// A scope in [`Scopes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeId(u32);

impl ScopeId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A binding in [`Scopes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindingId(u32);

impl BindingId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// What made a scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    /// A script's top level.
    Global,
    /// A module's top level.
    Module,
    /// A function, arrow or method: its parameters, and its body's
    /// declarations too unless it has a [`ScopeKind::FunctionBody`].
    Function,
    /// A function's body, apart from its parameters, when a parameter has
    /// a default or a computed key: what the body declares can't be seen
    /// from the parameters, as the spec's separate environment has it.
    FunctionBody,
    /// A block, a `switch`'s cases, a `for` with `let` or `const` in its
    /// head, or the block Annex B puts around `if (a) function f() {}`.
    Block,
    /// A `catch` clause: its parameter and its block's declarations.
    Catch,
    /// The body of a `with`, where any name may be the object's.
    With,
    /// A class: its own name, its heritage and its computed keys.
    Class,
    /// The implicit function a class field's initializer runs in.
    ClassField,
    /// A class `static { }` block.
    StaticBlock,
}

impl ScopeKind {
    /// Where `var` declarations land.
    const fn holds_vars(self) -> bool {
        matches!(
            self,
            ScopeKind::Global
                | ScopeKind::Module
                | ScopeKind::Function
                | ScopeKind::FunctionBody
                | ScopeKind::StaticBlock
        )
    }

    /// Code in it runs later than the code around it, if at all, so
    /// coming first in the source doesn't put a reference in the TDZ.
    const fn defers(self) -> bool {
        matches!(
            self,
            ScopeKind::Function | ScopeKind::ClassField | ScopeKind::StaticBlock
        )
    }
}

/// What declared a binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BindingKind {
    Var,
    Let,
    Const,
    /// A function declaration, or in sloppy code, the `var` binding Annex
    /// B gives a function declared in a block.
    Function,
    /// A class declaration's name in the scope around it.
    Class,
    Param,
    CatchParam,
    Import,
    /// A function expression's own name, inside it.
    FunctionName,
    /// A class's own name, inside it.
    ClassName,
    /// The `arguments` object of a function that isn't an arrow, made
    /// only when something refers to it.
    Arguments,
}

/// How a reference uses its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    Read,
    Write,
    /// `x += 1`, `x++`, `x ||= y`.
    ReadWrite,
}

/// A scope: what made it, the scope around it, and its bindings.
#[derive(Clone, Debug)]
pub struct Scope {
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>,
    /// The node that makes it: the program, a function, a block, a
    /// `switch`, a `for`, a `catch` clause, a `with`, a class, a
    /// field, a static block. For the block Annex B puts around
    /// `if (a) function f() {}`, the function.
    pub node: NodeId,
    pub strict: bool,
    /// A direct `eval` in it or in a scope inside it, or it's a `with`'s
    /// body: code the analysis can't see may read or declare names here.
    pub dynamic: bool,
    arrow: bool,
    bindings: Vec<BindingId>,
    names: HashMap<Box<str>, BindingId>,
}

impl Scope {
    /// Its bindings, in the order they were declared.
    pub fn bindings(&self) -> &[BindingId] {
        &self.bindings
    }
}

/// A name declared in a scope.
#[derive(Clone, Debug)]
pub struct Binding {
    pub name: String,
    pub kind: BindingKind,
    pub scope: ScopeId,
    /// The identifier that declares it first; `None` for `arguments`.
    pub node: Option<NodeId>,
    /// `let`, `const`, `class`, an import, a catch parameter, or a
    /// function declared in a block or at a module's top level: the
    /// names that can't be declared twice.
    lexical: bool,
    /// A plain function declaration, not a generator or async: sloppy
    /// code may declare one twice in a block.
    plain: bool,
    /// Where it's initialized: a reference before this, outside a nested
    /// function, is in its temporal dead zone. 0 for a binding that has
    /// none.
    tdz_end: u32,
}

/// An IdentifierReference, and what it resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    /// The [`Node::Identifier`].
    pub node: NodeId,
    /// The scope it's in.
    pub scope: ScopeId,
    /// `None` for a free name: a global, or a name nothing declares.
    pub binding: Option<BindingId>,
    pub access: Access,
    /// It refers to a `let`, `const` or `class` binding from before the
    /// declaration, in the same function: running it would throw.
    pub tdz: bool,
}

impl Reference {
    /// The name it refers to, escapes decoded.
    pub fn name<'a>(&self, ast: &Ast, src: &'a str) -> Cow<'a, str> {
        text::ident_name(ast.span(self.node).text(src))
    }
}

/// Every scope, binding and reference in a program. See the
/// [module docs](self).
#[derive(Clone, Debug)]
pub struct Scopes {
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    /// For each node, a reference (`REF | index`) or a declaring
    /// identifier (a binding's index), or `NONE`.
    by_node: Vec<u32>,
    scope_by_node: HashMap<NodeId, ScopeId>,
    free: Vec<String>,
    diagnostics: Vec<Diagnostic>,
}

const NONE: u32 = u32::MAX;
const REF: u32 = 1 << 31;

impl Scopes {
    /// Analyzes `ast`, parsed from `src`.
    pub fn analyze(ast: &Ast, src: &str) -> Scopes {
        let mut b = Builder {
            ast,
            src,
            s: Scopes {
                scopes: Vec::new(),
                bindings: Vec::new(),
                references: Vec::new(),
                by_node: vec![NONE; ast.len() + 1],
                scope_by_node: HashMap::new(),
                free: Vec::new(),
                diagnostics: Vec::new(),
            },
            cur: ScopeId(0),
            stack: Vec::new(),
            buf: Vec::new(),
            hoisted: HashSet::new(),
            annex_b: Vec::new(),
        };
        b.run();
        b.s
    }

    /// The global or module scope.
    pub fn root(&self) -> ScopeId {
        ScopeId(0)
    }

    pub fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id.index()]
    }

    /// Every scope, each after the scopes around it.
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    pub fn binding(&self, id: BindingId) -> &Binding {
        &self.bindings[id.index()]
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Every reference, in source order.
    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    /// The scope `node` makes, if it makes one.
    pub fn scope_of(&self, node: NodeId) -> Option<ScopeId> {
        self.scope_by_node.get(&node).copied()
    }

    /// The reference `node` is, if it's an IdentifierReference.
    pub fn reference(&self, node: NodeId) -> Option<&Reference> {
        match self.by_node.get(node.index()) {
            Some(&i) if i != NONE && i & REF != 0 => Some(&self.references[(i & !REF) as usize]),
            _ => None,
        }
    }

    /// The binding `node` declares, if it's a binding identifier.
    pub fn declaration(&self, node: NodeId) -> Option<BindingId> {
        match self.by_node.get(node.index()) {
            Some(&i) if i != NONE && i & REF == 0 => Some(BindingId(i)),
            _ => None,
        }
    }

    /// The binding a reference to `name` in `scope` would resolve to.
    pub fn lookup(&self, mut scope: ScopeId, name: &str) -> Option<BindingId> {
        loop {
            let s = self.scope(scope);
            if let Some(&b) = s.names.get(name) {
                return Some(b);
            }
            scope = s.parent?;
        }
    }

    /// The names references use that nothing in the program declares,
    /// each once, in the order they first appear.
    pub fn free_names(&self) -> &[String] {
        &self.free
    }

    /// Names declared twice, or both lexically and with `var`, where the
    /// spec makes that an early error.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

/// How a binding pattern's names are declared.
#[derive(Clone, Copy)]
struct Decl {
    kind: BindingKind,
    tdz_end: u32,
}

enum Task {
    Stmt(NodeId),
    Expr(NodeId),
    Target(NodeId, Access),
    Bind(NodeId, Decl),
    /// Carry on in this scope.
    Scope(ScopeId),
}

struct Builder<'a> {
    ast: &'a Ast,
    src: &'a str,
    s: Scopes,
    cur: ScopeId,
    stack: Vec<Task>,
    /// The current node's children, in source order, before they go on
    /// the stack reversed.
    buf: Vec<Task>,
    /// Names a `var` declares in a scope inside each of these, which a
    /// lexical declaration there would conflict with.
    hoisted: HashSet<(ScopeId, Box<str>)>,
    /// Sloppy functions declared in blocks: the block's scope and the
    /// binding there.
    annex_b: Vec<(ScopeId, BindingId)>,
}

impl<'a> Builder<'a> {
    fn run(&mut self) {
        let root = self.ast.root();
        let Node::Program { body, source_type } = *self.ast.node(root) else {
            unreachable!("the root is a program")
        };
        let module = source_type == SourceType::Module;
        let kind = if module {
            ScopeKind::Module
        } else {
            ScopeKind::Global
        };
        let strict = module || self.use_strict(body);
        self.cur = self.new_scope(kind, root, strict, None);
        self.push_list(body, Task::Stmt);
        self.flush();
        while let Some(task) = self.stack.pop() {
            match task {
                Task::Stmt(id) => self.stmt(id),
                Task::Expr(id) => self.expr(id),
                Task::Target(id, access) => self.target(id, access),
                Task::Bind(id, decl) => self.bind(id, decl),
                Task::Scope(s) => self.cur = s,
            }
            self.flush();
        }
        self.annex_b();
        self.resolve();
    }

    // The walk.

    fn flush(&mut self) {
        self.stack.extend(self.buf.drain(..).rev());
    }

    fn push(&mut self, task: Task) {
        self.buf.push(task);
    }

    fn push_opt(&mut self, id: Option<NodeId>, f: fn(NodeId) -> Task) {
        if let Some(id) = id {
            self.buf.push(f(id));
        }
    }

    fn push_list(&mut self, list: List, f: fn(NodeId) -> Task) {
        for &id in self.ast.get(list) {
            self.buf.push(f(id));
        }
    }

    /// Runs the tasks queued after this in `scope`, then goes back.
    fn enter(&mut self, scope: ScopeId) {
        // Queued first, so it runs last.
        self.stack.push(Task::Scope(self.cur));
        self.cur = scope;
    }

    fn node(&self, id: NodeId) -> Node {
        *self.ast.node(id)
    }

    fn name(&self, id: NodeId) -> Cow<'a, str> {
        text::ident_name(self.ast.span(id).text(self.src))
    }

    fn scope(&self, id: ScopeId) -> &Scope {
        &self.s.scopes[id.index()]
    }

    fn new_scope(
        &mut self,
        kind: ScopeKind,
        node: NodeId,
        strict: bool,
        parent: Option<ScopeId>,
    ) -> ScopeId {
        let id = ScopeId(self.s.scopes.len() as u32);
        self.s.scopes.push(Scope {
            kind,
            parent,
            node,
            strict,
            dynamic: kind == ScopeKind::With,
            arrow: false,
            bindings: Vec::new(),
            names: HashMap::new(),
        });
        // The block Annex B puts around a function gives way to the
        // function's own scope, made after it.
        self.s.scope_by_node.insert(node, id);
        id
    }

    /// A scope inside the current one.
    fn child(&mut self, kind: ScopeKind, node: NodeId) -> ScopeId {
        let strict = self.scope(self.cur).strict;
        self.new_scope(kind, node, strict, Some(self.cur))
    }

    /// Whether a statement list's directive prologue says "use strict".
    fn use_strict(&self, list: List) -> bool {
        for &id in self.ast.get(list) {
            let Node::Expression {
                expression,
                directive: true,
            } = self.node(id)
            else {
                return false;
            };
            let span = self.ast.span(expression);
            if self.src.get(span.start as usize + 1..span.end as usize - 1) == Some("use strict") {
                return true;
            }
        }
        false
    }

    fn stmt(&mut self, id: NodeId) {
        match self.node(id) {
            Node::Expression { expression, .. } => self.push(Task::Expr(expression)),
            Node::Block { body } => {
                let s = self.child(ScopeKind::Block, id);
                self.enter(s);
                self.push_list(body, Task::Stmt);
            }
            Node::With { object, body } => {
                self.push(Task::Expr(object));
                let s = self.child(ScopeKind::With, id);
                self.push(Task::Scope(s));
                self.push(Task::Stmt(body));
                self.push(Task::Scope(self.cur));
            }
            Node::Return { argument } => self.push_opt(argument, Task::Expr),
            Node::Labeled { body, .. } => self.push(Task::Stmt(body)),
            Node::If {
                test,
                consequent,
                alternate,
            } => {
                self.push(Task::Expr(test));
                self.if_body(consequent);
                if let Some(alternate) = alternate {
                    self.if_body(alternate);
                }
            }
            Node::Switch {
                discriminant,
                cases,
            } => {
                self.push(Task::Expr(discriminant));
                let s = self.child(ScopeKind::Block, id);
                self.push(Task::Scope(s));
                for &case in self.ast.get(cases) {
                    if let Node::SwitchCase { test, consequent } = self.node(case) {
                        self.push_opt(test, Task::Expr);
                        self.push_list(consequent, Task::Stmt);
                    }
                }
                self.push(Task::Scope(self.cur));
            }
            Node::Throw { argument } => self.push(Task::Expr(argument)),
            Node::Try {
                block,
                handler,
                finalizer,
            } => {
                self.push(Task::Stmt(block));
                if let Some(handler) = handler
                    && let Node::CatchClause { param, body } = self.node(handler)
                {
                    let s = self.child(ScopeKind::Catch, handler);
                    self.push(Task::Scope(s));
                    if let Some(param) = param {
                        self.push(Task::Bind(param, decl(BindingKind::CatchParam)));
                    }
                    // The block's declarations share the clause's scope,
                    // where they meet the parameter.
                    if let Node::Block { body } = self.node(body) {
                        self.push_list(body, Task::Stmt);
                    }
                    self.push(Task::Scope(self.cur));
                }
                self.push_opt(finalizer, Task::Stmt);
            }
            Node::While { test, body } => {
                self.push(Task::Expr(test));
                self.push(Task::Stmt(body));
            }
            Node::DoWhile { body, test } => {
                self.push(Task::Stmt(body));
                self.push(Task::Expr(test));
            }
            Node::For {
                init,
                test,
                update,
                body,
            } => {
                if init.is_some_and(|i| self.lexical_declaration(i)) {
                    let s = self.child(ScopeKind::Block, id);
                    self.enter(s);
                }
                if let Some(init) = init {
                    if matches!(self.node(init), Node::VariableDeclaration { .. }) {
                        self.push(Task::Stmt(init));
                    } else {
                        self.push(Task::Expr(init));
                    }
                }
                self.push_opt(test, Task::Expr);
                self.push_opt(update, Task::Expr);
                self.push(Task::Stmt(body));
            }
            Node::ForIn { left, right, body }
            | Node::ForOf {
                left, right, body, ..
            } => {
                if let Node::VariableDeclaration { kind, declarations } = self.node(left) {
                    if kind != VarKind::Var {
                        let s = self.child(ScopeKind::Block, id);
                        self.enter(s);
                    }
                    // The head's names are in their TDZ until the
                    // expression after `in` or `of` has run.
                    let tdz_end = self.ast.span(right).end;
                    for &d in self.ast.get(declarations) {
                        if let Node::VariableDeclarator { id: target, init } = self.node(d) {
                            self.push(Task::Bind(
                                target,
                                Decl {
                                    kind: var_kind(kind),
                                    tdz_end,
                                },
                            ));
                            self.push_opt(init, Task::Expr);
                        }
                    }
                } else {
                    self.push(Task::Target(left, Access::Write));
                }
                self.push(Task::Expr(right));
                self.push(Task::Stmt(body));
            }
            Node::VariableDeclaration { kind, declarations } => {
                for &d in self.ast.get(declarations) {
                    if let Node::VariableDeclarator { id: target, init } = self.node(d) {
                        let tdz_end = self.ast.span(d).end;
                        self.push(Task::Bind(
                            target,
                            Decl {
                                kind: var_kind(kind),
                                tdz_end,
                            },
                        ));
                        self.push_opt(init, Task::Expr);
                    }
                }
            }
            Node::Function(f) if f.kind == FunctionKind::Declaration => {
                self.function_declaration(id, f);
            }
            Node::Class(c) if c.declaration => self.class(id, c),
            Node::ExportNamed {
                declaration,
                specifiers,
                source,
                ..
            } => {
                self.push_opt(declaration, Task::Stmt);
                // `export { a as b }` refers to `a`; with `from`, it
                // names another module's export.
                if source.is_none() {
                    for &spec in self.ast.get(specifiers) {
                        if let Node::ExportSpecifier { local, .. } = self.node(spec)
                            && matches!(self.node(local), Node::Identifier)
                        {
                            self.push(Task::Expr(local));
                        }
                    }
                }
            }
            Node::ExportDefault { declaration } => match self.node(declaration) {
                Node::Function(f) if f.kind == FunctionKind::Declaration => {
                    self.push(Task::Stmt(declaration));
                }
                Node::Class(c) if c.declaration => self.push(Task::Stmt(declaration)),
                _ => self.push(Task::Expr(declaration)),
            },
            Node::ImportDeclaration { specifiers, .. } => {
                for &spec in self.ast.get(specifiers) {
                    let local = match self.node(spec) {
                        Node::ImportSpecifier { local, .. }
                        | Node::ImportDefaultSpecifier { local }
                        | Node::ImportNamespaceSpecifier { local } => local,
                        _ => continue,
                    };
                    self.push(Task::Bind(local, decl(BindingKind::Import)));
                }
            }
            Node::Empty | Node::Debugger | Node::Break { .. } | Node::Continue { .. } => {}
            Node::ExportAll { .. } => {}
            _ => self.push(Task::Expr(id)),
        }
    }

    /// An `if`'s branch, where sloppy code may have a function
    /// declaration (B.3.3), which is as if it were in a block of its own.
    fn if_body(&mut self, id: NodeId) {
        if let Node::Function(f) = self.node(id)
            && f.kind == FunctionKind::Declaration
        {
            let outer = self.cur;
            let s = self.child(ScopeKind::Block, id);
            self.cur = s;
            self.function_declaration(id, f);
            self.cur = outer;
            self.push(Task::Scope(outer));
            return;
        }
        self.push(Task::Stmt(id));
    }

    fn lexical_declaration(&self, id: NodeId) -> bool {
        matches!(
            self.node(id),
            Node::VariableDeclaration {
                kind: VarKind::Let | VarKind::Const,
                ..
            }
        )
    }

    fn expr(&mut self, id: NodeId) {
        match self.node(id) {
            Node::Identifier => self.reference(id, Access::Read),
            Node::Array { elements, .. } => self.push_list(elements, Task::Expr),
            Node::Object { properties, .. } => {
                for &p in self.ast.get(properties) {
                    match self.node(p) {
                        Node::Property {
                            key,
                            value,
                            computed,
                            ..
                        } => {
                            if computed {
                                self.push(Task::Expr(key));
                            }
                            self.push(Task::Expr(value));
                        }
                        _ => self.push(Task::Expr(p)),
                    }
                }
            }
            Node::Template { expressions, .. } => self.push_list(expressions, Task::Expr),
            Node::TaggedTemplate { tag, quasi } => {
                self.push(Task::Expr(tag));
                self.push(Task::Expr(quasi));
            }
            Node::Unary { argument, .. }
            | Node::Spread { argument }
            | Node::Await { argument }
            | Node::Paren {
                expression: argument,
            }
            | Node::Chain {
                expression: argument,
            } => self.push(Task::Expr(argument)),
            Node::Update { argument, .. } => self.push(Task::Target(argument, Access::ReadWrite)),
            Node::Binary { left, right, .. } | Node::Logical { left, right, .. } => {
                self.push(Task::Expr(left));
                self.push(Task::Expr(right));
            }
            Node::Assign {
                operator,
                left,
                right,
            } => {
                let access = if operator == AssignOp::Assign {
                    Access::Write
                } else {
                    Access::ReadWrite
                };
                self.push(Task::Target(left, access));
                self.push(Task::Expr(right));
            }
            Node::Conditional {
                test,
                consequent,
                alternate,
            } => {
                self.push(Task::Expr(test));
                self.push(Task::Expr(consequent));
                self.push(Task::Expr(alternate));
            }
            Node::Call {
                callee,
                arguments,
                optional,
            } => {
                let inner = self.ast.unparenthesized(callee);
                if !optional
                    && matches!(self.node(inner), Node::Identifier)
                    && self.name(inner) == "eval"
                {
                    self.direct_eval();
                }
                self.push(Task::Expr(callee));
                self.push_list(arguments, Task::Expr);
            }
            Node::New { callee, arguments } => {
                self.push(Task::Expr(callee));
                self.push_list(arguments, Task::Expr);
            }
            Node::Member {
                object,
                property,
                computed,
                ..
            } => {
                self.push(Task::Expr(object));
                if computed {
                    self.push(Task::Expr(property));
                }
            }
            Node::Sequence { expressions } => self.push_list(expressions, Task::Expr),
            Node::Yield { argument, .. } => self.push_opt(argument, Task::Expr),
            Node::Import { source, options } => {
                self.push(Task::Expr(source));
                self.push_opt(options, Task::Expr);
            }
            Node::Function(f) => self.function(id, f),
            Node::Class(c) => self.class(id, c),
            Node::ObjectPattern { .. }
            | Node::ArrayPattern { .. }
            | Node::AssignmentPattern { .. }
            | Node::Rest { .. } => self.target(id, Access::Write),
            _ => {}
        }
    }

    /// An assignment's target: names in it are written.
    fn target(&mut self, id: NodeId, access: Access) {
        match self.node(id) {
            Node::Identifier => self.reference(id, access),
            Node::Paren { expression } => self.push(Task::Target(expression, access)),
            Node::ObjectPattern { properties } => {
                for &p in self.ast.get(properties) {
                    match self.node(p) {
                        Node::Property {
                            key,
                            value,
                            computed,
                            ..
                        } => {
                            if computed {
                                self.push(Task::Expr(key));
                            }
                            self.push(Task::Target(value, Access::Write));
                        }
                        Node::Rest { argument } => self.push(Task::Target(argument, Access::Write)),
                        _ => {}
                    }
                }
            }
            Node::ArrayPattern { elements } => {
                for &e in self.ast.get(elements) {
                    if !matches!(self.node(e), Node::Elision) {
                        self.push(Task::Target(e, Access::Write));
                    }
                }
            }
            Node::Rest { argument } => self.push(Task::Target(argument, Access::Write)),
            Node::AssignmentPattern { left, right } => {
                self.push(Task::Target(left, Access::Write));
                self.push(Task::Expr(right));
            }
            _ => self.push(Task::Expr(id)),
        }
    }

    /// A binding pattern: names in it are declared as `d` says.
    fn bind(&mut self, id: NodeId, d: Decl) {
        match self.node(id) {
            Node::Identifier => self.declare(id, d, true),
            Node::ObjectPattern { properties } => {
                for &p in self.ast.get(properties) {
                    match self.node(p) {
                        Node::Property {
                            key,
                            value,
                            computed,
                            ..
                        } => {
                            if computed {
                                self.push(Task::Expr(key));
                            }
                            self.push(Task::Bind(value, d));
                        }
                        Node::Rest { argument } => self.push(Task::Bind(argument, d)),
                        _ => {}
                    }
                }
            }
            Node::ArrayPattern { elements } => {
                for &e in self.ast.get(elements) {
                    if !matches!(self.node(e), Node::Elision) {
                        self.push(Task::Bind(e, d));
                    }
                }
            }
            Node::Rest { argument } => self.push(Task::Bind(argument, d)),
            Node::AssignmentPattern { left, right } => {
                self.push(Task::Bind(left, d));
                self.push(Task::Expr(right));
            }
            _ => {}
        }
    }

    fn reference(&mut self, id: NodeId, access: Access) {
        let index = self.s.references.len() as u32;
        self.s.references.push(Reference {
            node: id,
            scope: self.cur,
            binding: None,
            access,
            tdz: false,
        });
        self.s.by_node[id.index()] = REF | index;
    }

    fn direct_eval(&mut self) {
        let mut s = Some(self.cur);
        while let Some(id) = s {
            let scope = &mut self.s.scopes[id.index()];
            scope.dynamic = true;
            s = scope.parent;
        }
    }

    fn function_declaration(&mut self, id: NodeId, f: Function) {
        if let Some(name) = f.id {
            let kind = self.scope(self.cur).kind;
            let plain = !f.is_async && !f.generator;
            if kind.holds_vars() && kind != ScopeKind::Module {
                self.declare_var(name, BindingKind::Function);
            } else {
                let b = self.declare_lexical(name, BindingKind::Function, plain, 0);
                if let Some(b) = b
                    && plain
                    && !self.scope(self.cur).strict
                    && kind != ScopeKind::Module
                {
                    self.annex_b.push((self.cur, b));
                }
            }
        }
        self.function(id, f);
    }

    /// A function's scope, its parameters and its body.
    fn function(&mut self, id: NodeId, f: Function) {
        let outer = self.cur;
        let body_list = match self.node(f.body) {
            Node::Block { body } if !f.expression => Some(body),
            _ => None,
        };
        let strict = self.scope(outer).strict || body_list.is_some_and(|l| self.use_strict(l));
        let s = self.new_scope(ScopeKind::Function, id, strict, Some(outer));
        self.s.scopes[s.index()].arrow = f.kind == FunctionKind::Arrow;
        if f.kind == FunctionKind::Expression
            && let Some(name) = f.id
        {
            self.cur = s;
            self.declare(name, decl(BindingKind::FunctionName), true);
            self.cur = outer;
        }
        self.push(Task::Scope(s));
        let params = self.ast.get(f.params);
        let separate = params.iter().any(|&p| self.has_expression(p));
        self.push_list(f.params, |p| Task::Bind(p, decl(BindingKind::Param)));
        match body_list {
            Some(list) => {
                if separate {
                    let b = self.new_scope(ScopeKind::FunctionBody, f.body, strict, Some(s));
                    self.push(Task::Scope(b));
                }
                self.push_list(list, Task::Stmt);
            }
            None => self.push(Task::Expr(f.body)),
        }
        self.push(Task::Scope(outer));
    }

    /// Whether a parameter has an expression in it: a default, or a
    /// computed key.
    fn has_expression(&self, id: NodeId) -> bool {
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            match self.node(id) {
                Node::AssignmentPattern { .. } => return true,
                Node::ObjectPattern { properties } => {
                    for &p in self.ast.get(properties) {
                        match self.node(p) {
                            Node::Property { computed: true, .. } => return true,
                            Node::Property { value, .. } => stack.push(value),
                            Node::Rest { argument } => stack.push(argument),
                            _ => {}
                        }
                    }
                }
                Node::ArrayPattern { elements } => stack.extend(self.ast.get(elements)),
                Node::Rest { argument } => stack.push(argument),
                _ => {}
            }
        }
        false
    }

    fn class(&mut self, id: NodeId, c: Class) {
        let end = self.ast.span(id).end;
        if c.declaration
            && let Some(name) = c.id
        {
            self.declare_lexical(name, BindingKind::Class, false, end);
        }
        let outer = self.cur;
        let s = self.new_scope(ScopeKind::Class, id, true, Some(outer));
        self.push(Task::Scope(s));
        if let Some(name) = c.id {
            self.cur = s;
            self.declare(
                name,
                Decl {
                    kind: BindingKind::ClassName,
                    tdz_end: end,
                },
                !c.declaration,
            );
            self.cur = outer;
        }
        self.push_opt(c.super_class, Task::Expr);
        let Node::ClassBody { body } = self.node(c.body) else {
            unreachable!("a class has a body")
        };
        for &element in self.ast.get(body) {
            match self.node(element) {
                Node::Method {
                    key,
                    value,
                    computed,
                    ..
                } => {
                    if computed {
                        self.push(Task::Expr(key));
                    }
                    self.push(Task::Expr(value));
                }
                Node::Field {
                    key,
                    value,
                    computed,
                    ..
                } => {
                    if computed {
                        self.push(Task::Expr(key));
                    }
                    if let Some(value) = value {
                        let f = self.new_scope(ScopeKind::ClassField, element, true, Some(s));
                        self.push(Task::Scope(f));
                        self.push(Task::Expr(value));
                        self.push(Task::Scope(s));
                    }
                }
                Node::StaticBlock { body } => {
                    let b = self.new_scope(ScopeKind::StaticBlock, element, true, Some(s));
                    self.push(Task::Scope(b));
                    self.push_list(body, Task::Stmt);
                    self.push(Task::Scope(s));
                }
                _ => {}
            }
        }
        self.push(Task::Scope(outer));
    }

    // Declarations.

    /// Declares the identifier `id` as `d` says, in the current scope or,
    /// for a `var`, the function's. `map` records `id` as declaring it.
    fn declare(&mut self, id: NodeId, d: Decl, map: bool) {
        match d.kind {
            BindingKind::Var => self.declare_var(id, BindingKind::Var),
            BindingKind::Let | BindingKind::Const | BindingKind::Import => {
                self.declare_lexical(id, d.kind, false, d.tdz_end);
            }
            _ => {
                let name = self.name(id);
                let existing = self.scope(self.cur).names.get(&*name).copied();
                match existing {
                    // A parameter's name, given twice in sloppy code.
                    Some(b)
                        if self.s.bindings[b.index()].kind == d.kind
                            && d.kind == BindingKind::Param =>
                    {
                        self.s.by_node[id.index()] = b.0;
                    }
                    _ => {
                        let lexical =
                            matches!(d.kind, BindingKind::CatchParam | BindingKind::ClassName);
                        let b = self.add(Binding {
                            lexical,
                            tdz_end: d.tdz_end,
                            ..new_binding(self.cur, &name, d.kind, Some(id))
                        });
                        if map {
                            self.s.by_node[id.index()] = b.0;
                        }
                    }
                }
            }
        }
    }

    fn add(&mut self, binding: Binding) -> BindingId {
        let b = BindingId(self.s.bindings.len() as u32);
        let s = &mut self.s.scopes[binding.scope.index()];
        s.bindings.push(b);
        s.names.insert(binding.name.as_str().into(), b);
        self.s.bindings.push(binding);
        b
    }

    /// Whether `scope` is a `catch` clause whose parameter is just a
    /// name, which a `var` in its block may declare again (B.3.4).
    fn simple_catch(&self, scope: ScopeId) -> bool {
        let s = self.scope(scope);
        s.kind == ScopeKind::Catch
            && matches!(
                self.node(s.node),
                Node::CatchClause { param: Some(p), .. } if matches!(self.node(p), Node::Identifier)
            )
    }

    fn declare_var(&mut self, id: NodeId, kind: BindingKind) {
        let name = self.name(id);
        let mut s = self.cur;
        loop {
            let scope = self.scope(s);
            let holds = scope.kind.holds_vars();
            if let Some(&b) = scope.names.get(&*name) {
                let had = &self.s.bindings[b.index()];
                if had.lexical {
                    let allowed = had.kind == BindingKind::CatchParam && self.simple_catch(s);
                    if !allowed {
                        let first = had.node;
                        self.conflict("lexical-var-conflict", id, &name, first);
                        return;
                    }
                } else if holds && had.kind != BindingKind::FunctionName {
                    self.s.by_node[id.index()] = b.0;
                    return;
                }
            }
            if holds {
                let b = self.add(new_binding(s, &name, kind, Some(id)));
                self.s.by_node[id.index()] = b.0;
                return;
            }
            let parent = scope.parent.expect("a var scope is at the top");
            self.hoisted.insert((s, name.as_ref().into()));
            s = parent;
        }
    }

    /// Declares a lexical name in the current scope. `None` if it's an
    /// error, or a sloppy function declared again.
    fn declare_lexical(
        &mut self,
        id: NodeId,
        kind: BindingKind,
        plain: bool,
        tdz_end: u32,
    ) -> Option<BindingId> {
        let name = self.name(id);
        let s = self.cur;
        let scope = self.scope(s);
        if let Some(&b) = scope.names.get(&*name) {
            let had = &self.s.bindings[b.index()];
            if had.kind != BindingKind::FunctionName {
                let first = had.node;
                if had.lexical {
                    let sloppy_functions = kind == BindingKind::Function
                        && had.kind == BindingKind::Function
                        && plain
                        && had.plain
                        && !scope.strict;
                    if sloppy_functions {
                        self.s.by_node[id.index()] = b.0;
                        return None;
                    }
                    self.conflict("duplicate-declaration", id, &name, first);
                } else if had.kind == BindingKind::Param {
                    self.conflict("duplicate-declaration", id, &name, first);
                } else {
                    self.conflict("lexical-var-conflict", id, &name, first);
                }
                return None;
            }
        }
        if self.hoisted.contains(&(s, name.as_ref().into())) {
            self.conflict("lexical-var-conflict", id, &name, None);
            return None;
        }
        if scope.kind == ScopeKind::FunctionBody
            && let Some(parent) = scope.parent
            && let Some(&b) = self.scope(parent).names.get(&*name)
            && self.s.bindings[b.index()].kind == BindingKind::Param
        {
            let first = self.s.bindings[b.index()].node;
            self.conflict("duplicate-declaration", id, &name, first);
            return None;
        }
        let b = self.add(Binding {
            lexical: true,
            plain,
            tdz_end,
            ..new_binding(s, &name, kind, Some(id))
        });
        self.s.by_node[id.index()] = b.0;
        Some(b)
    }

    fn conflict(&mut self, code: &'static str, id: NodeId, name: &str, first: Option<NodeId>) {
        let message = if code == "duplicate-declaration" {
            format!("`{name}` is already declared in this scope")
        } else {
            format!("`{name}` is declared both with `var` and as a lexical name in this scope")
        };
        let mut d = Diagnostic::error(code, self.ast.span(id), message);
        if let Some(first) = first {
            d = d.with_label(self.ast.span(first), "declared here first");
        }
        self.s.diagnostics.push(d);
    }

    /// B.3.2: a sloppy function declared in a block is also a `var` in
    /// its function, unless a `var` of its name would be an error there
    /// or the name is a parameter's.
    fn annex_b(&mut self) {
        for (block, b) in std::mem::take(&mut self.annex_b) {
            let name = self.s.bindings[b.index()].name.clone();
            let node = self.s.bindings[b.index()].node;
            let mut s = self.scope(block).parent.expect("a block is in a scope");
            let target = loop {
                let scope = self.scope(s);
                if let Some(&had) = scope.names.get(name.as_str()) {
                    let had = &self.s.bindings[had.index()];
                    let blocks = had.lexical
                        && !(had.kind == BindingKind::CatchParam && self.simple_catch(s))
                        || had.kind == BindingKind::Param;
                    if blocks {
                        break None;
                    }
                }
                if scope.kind == ScopeKind::FunctionBody
                    && let Some(parent) = scope.parent
                    && let Some(&had) = self.scope(parent).names.get(name.as_str())
                    && self.s.bindings[had.index()].kind == BindingKind::Param
                {
                    break None;
                }
                if scope.kind.holds_vars() {
                    break Some(s);
                }
                s = scope.parent.expect("a var scope is at the top");
            };
            if let Some(target) = target {
                let exists = self
                    .scope(target)
                    .names
                    .get(name.as_str())
                    .is_some_and(|&had| {
                        self.s.bindings[had.index()].kind != BindingKind::FunctionName
                    });
                if !exists {
                    self.add(Binding {
                        plain: true,
                        ..new_binding(target, &name, BindingKind::Function, node)
                    });
                }
            }
        }
    }

    // Resolution.

    fn resolve(&mut self) {
        let mut free: HashSet<String> = HashSet::new();
        for i in 0..self.s.references.len() {
            let r = self.s.references[i];
            let span = self.ast.span(r.node);
            let name = text::ident_name(span.text(self.src));
            let mut s = r.scope;
            let mut deferred = false;
            let found = loop {
                let scope = self.scope(s);
                if let Some(&b) = scope.names.get(&*name) {
                    break Some((b, deferred));
                }
                if name == "arguments" && scope.kind == ScopeKind::Function && !scope.arrow {
                    let b = self.add(new_binding(s, "arguments", BindingKind::Arguments, None));
                    break Some((b, deferred));
                }
                deferred |= scope.kind.defers();
                match scope.parent {
                    Some(p) => s = p,
                    None => break None,
                }
            };
            match found {
                Some((b, deferred)) => {
                    let binding = &self.s.bindings[b.index()];
                    let tdz = !deferred && span.start < binding.tdz_end;
                    let r = &mut self.s.references[i];
                    r.binding = Some(b);
                    r.tdz = tdz;
                }
                None => {
                    if !free.contains(&*name) {
                        free.insert(name.to_string());
                        self.s.free.push(name.into_owned());
                    }
                }
            }
        }
    }
}

/// A binding that isn't lexical and has no TDZ.
fn new_binding(scope: ScopeId, name: &str, kind: BindingKind, node: Option<NodeId>) -> Binding {
    Binding {
        name: name.to_string(),
        kind,
        scope,
        node,
        lexical: false,
        plain: false,
        tdz_end: 0,
    }
}

const fn decl(kind: BindingKind) -> Decl {
    Decl { kind, tdz_end: 0 }
}

const fn var_kind(kind: VarKind) -> BindingKind {
    match kind {
        VarKind::Var => BindingKind::Var,
        VarKind::Let => BindingKind::Let,
        VarKind::Const => BindingKind::Const,
    }
}
