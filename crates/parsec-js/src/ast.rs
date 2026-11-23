// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The syntax tree: every node in one arena, addressed by [`NodeId`],
//! with its [`Span`] beside it and its children's ids inside it.
//!
//! A node holds no text. An identifier's name, a string's value, a
//! number's: each is in the source at the node's span, and
//! [`Ast::to_estree`] reads it from there. A list of children (a block's
//! statements, a call's arguments) is a [`List`], a range of one shared
//! vector, so building a tree allocates a few vectors, not one per node.
//!
//! The node kinds follow ESTree's, so a tool written for acorn's tree
//! finds what it expects, with one addition: [`Node::Paren`] keeps
//! parentheses, which ESTree drops and the grammar's early errors need.

use std::num::NonZeroU32;

use parsec_base::{Diagnostic, Span};

use crate::SourceType;

/// A node in an [`Ast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(NonZeroU32);

impl NodeId {
    pub(crate) fn new(index: usize) -> NodeId {
        NodeId(NonZeroU32::new(index as u32).expect("node 0 is never handed out"))
    }

    pub const fn index(self) -> usize {
        self.0.get() as usize
    }
}

/// A run of nodes: `len` ids from `start` in the arena's shared list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct List {
    pub(crate) start: u32,
    pub(crate) len: u32,
}

impl List {
    pub const fn len(self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// A comment, kept beside the tree rather than in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Comment {
    pub kind: CommentKind,
    /// The whole comment, its `//`, `/* */`, `<!--` or `-->` included.
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentKind {
    /// `// ...`
    Line,
    /// `/* ... */`
    Block,
    /// `#!...` on the first line.
    Hashbang,
    /// Annex B's `<!-- ...`, in scripts.
    HtmlOpen,
    /// Annex B's `--> ...` at the start of a line, in scripts.
    HtmlClose,
}

/// `var`, `let` or `const`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

impl VarKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            VarKind::Var => "var",
            VarKind::Let => "let",
            VarKind::Const => "const",
        }
    }
}

/// What a literal is. Its value is in its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiteralKind {
    String,
    Number,
    BigInt,
    Boolean,
    Null,
    RegExp,
}

/// Which of the three a function is, as ESTree names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionKind {
    Declaration,
    Expression,
    Arrow,
}

/// A function of any kind: a declaration, an expression, an arrow, or
/// the value of a method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Function {
    pub kind: FunctionKind,
    pub id: Option<NodeId>,
    pub params: List,
    /// A block, or for an arrow with an expression body, the expression.
    pub body: NodeId,
    pub is_async: bool,
    pub generator: bool,
    /// An arrow whose body is an expression, not a block.
    pub expression: bool,
}

/// A class declaration or expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Class {
    pub declaration: bool,
    pub id: Option<NodeId>,
    pub super_class: Option<NodeId>,
    /// A [`Node::ClassBody`].
    pub body: NodeId,
}

/// An object literal's, or an object pattern's, `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyKind {
    Init,
    Get,
    Set,
}

impl PropertyKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            PropertyKind::Init => "init",
            PropertyKind::Get => "get",
            PropertyKind::Set => "set",
        }
    }
}

/// A class method's `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MethodKind {
    Constructor,
    Method,
    Get,
    Set,
}

impl MethodKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            MethodKind::Constructor => "constructor",
            MethodKind::Method => "method",
            MethodKind::Get => "get",
            MethodKind::Set => "set",
        }
    }
}

macro_rules! ops {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $text:literal,)* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)*
                }
            }
        }
    };
}

ops! {
    UnaryOp {
        Minus = "-",
        Plus = "+",
        Not = "!",
        BitNot = "~",
        Typeof = "typeof",
        Void = "void",
        Delete = "delete",
    }
}

ops! {
    UpdateOp {
        Increment = "++",
        Decrement = "--",
    }
}

ops! {
    BinaryOp {
        Eq = "==",
        NotEq = "!=",
        StrictEq = "===",
        StrictNotEq = "!==",
        Lt = "<",
        LtEq = "<=",
        Gt = ">",
        GtEq = ">=",
        Shl = "<<",
        Shr = ">>",
        UShr = ">>>",
        Add = "+",
        Sub = "-",
        Mul = "*",
        Div = "/",
        Rem = "%",
        Exp = "**",
        BitOr = "|",
        BitXor = "^",
        BitAnd = "&",
        In = "in",
        Instanceof = "instanceof",
    }
}

ops! {
    LogicalOp {
        Or = "||",
        And = "&&",
        Coalesce = "??",
    }
}

ops! {
    AssignOp {
        Assign = "=",
        Add = "+=",
        Sub = "-=",
        Mul = "*=",
        Div = "/=",
        Rem = "%=",
        Exp = "**=",
        Shl = "<<=",
        Shr = ">>=",
        UShr = ">>>=",
        BitOr = "|=",
        BitXor = "^=",
        BitAnd = "&=",
        Or = "||=",
        And = "&&=",
        Coalesce = "??=",
    }
}

/// A node. The variants are ESTree's node types, named without their
/// `Statement` and `Expression` suffixes where those are plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    /// The whole source. Its span is the whole input.
    Program {
        body: List,
        source_type: SourceType,
    },

    // Statements.
    /// An expression as a statement; `directive` when it's a string in a
    /// directive prologue, like `"use strict"`.
    Expression {
        expression: NodeId,
        directive: bool,
    },
    Block {
        body: List,
    },
    Empty,
    Debugger,
    With {
        object: NodeId,
        body: NodeId,
    },
    Return {
        argument: Option<NodeId>,
    },
    Labeled {
        label: NodeId,
        body: NodeId,
    },
    Break {
        label: Option<NodeId>,
    },
    Continue {
        label: Option<NodeId>,
    },
    If {
        test: NodeId,
        consequent: NodeId,
        alternate: Option<NodeId>,
    },
    Switch {
        discriminant: NodeId,
        cases: List,
    },
    SwitchCase {
        test: Option<NodeId>,
        consequent: List,
    },
    Throw {
        argument: NodeId,
    },
    Try {
        block: NodeId,
        handler: Option<NodeId>,
        finalizer: Option<NodeId>,
    },
    CatchClause {
        param: Option<NodeId>,
        body: NodeId,
    },
    While {
        test: NodeId,
        body: NodeId,
    },
    DoWhile {
        body: NodeId,
        test: NodeId,
    },
    For {
        init: Option<NodeId>,
        test: Option<NodeId>,
        update: Option<NodeId>,
        body: NodeId,
    },
    ForIn {
        left: NodeId,
        right: NodeId,
        body: NodeId,
    },
    ForOf {
        left: NodeId,
        right: NodeId,
        body: NodeId,
        is_await: bool,
    },
    VariableDeclaration {
        kind: VarKind,
        declarations: List,
    },
    VariableDeclarator {
        id: NodeId,
        init: Option<NodeId>,
    },
    Function(Function),
    Class(Class),
    ClassBody {
        body: List,
    },
    /// A class method; `value` is its function.
    Method {
        key: NodeId,
        value: NodeId,
        kind: MethodKind,
        computed: bool,
        is_static: bool,
    },
    /// A class field, ESTree's PropertyDefinition.
    Field {
        key: NodeId,
        value: Option<NodeId>,
        computed: bool,
        is_static: bool,
    },
    StaticBlock {
        body: List,
    },

    // Expressions.
    Identifier,
    /// `#name`; the span includes the `#`.
    PrivateIdentifier,
    Literal(LiteralKind),
    This,
    Super,
    /// `elements` has a [`Node::Elision`] for each hole.
    Array {
        elements: List,
        comma_after_spread: bool,
    },
    /// A hole in an array: the nothing between `[,` and `,]`.
    Elision,
    Object {
        properties: List,
        comma_after_spread: bool,
    },
    /// An object literal's or object pattern's property.
    Property {
        key: NodeId,
        value: NodeId,
        kind: PropertyKind,
        method: bool,
        shorthand: bool,
        computed: bool,
    },
    Template {
        quasis: List,
        expressions: List,
    },
    /// A template's text between delimiters; the span leaves the
    /// delimiters out, as acorn's does. `cooked` is false for a tagged
    /// template's invalid escape, whose cooked value is undefined.
    TemplateElement {
        tail: bool,
        cooked: bool,
    },
    TaggedTemplate {
        tag: NodeId,
        quasi: NodeId,
    },
    Unary {
        operator: UnaryOp,
        argument: NodeId,
    },
    Update {
        operator: UpdateOp,
        prefix: bool,
        argument: NodeId,
    },
    Binary {
        operator: BinaryOp,
        left: NodeId,
        right: NodeId,
    },
    Logical {
        operator: LogicalOp,
        left: NodeId,
        right: NodeId,
    },
    Assign {
        operator: AssignOp,
        left: NodeId,
        right: NodeId,
    },
    Conditional {
        test: NodeId,
        consequent: NodeId,
        alternate: NodeId,
    },
    Call {
        callee: NodeId,
        arguments: List,
        optional: bool,
    },
    New {
        callee: NodeId,
        arguments: List,
    },
    Member {
        object: NodeId,
        property: NodeId,
        computed: bool,
        optional: bool,
    },
    /// An optional chain, `a?.b.c`, from its start to its end.
    Chain {
        expression: NodeId,
    },
    Sequence {
        expressions: List,
    },
    Yield {
        argument: Option<NodeId>,
        delegate: bool,
    },
    Await {
        argument: NodeId,
    },
    Spread {
        argument: NodeId,
    },
    /// `new.target` or `import.meta`.
    MetaProperty {
        meta: NodeId,
        property: NodeId,
    },
    /// `import(source)` or `import(source, options)`.
    Import {
        source: NodeId,
        options: Option<NodeId>,
    },
    /// A parenthesized expression. ESTree has no such node, and
    /// [`Ast::to_estree`] writes the expression inside instead.
    Paren {
        expression: NodeId,
    },

    // Modules.
    /// `import ... from "source" with { ... }`; `attributes` are
    /// [`Node::ImportAttribute`]s.
    ImportDeclaration {
        specifiers: List,
        source: NodeId,
        attributes: List,
    },
    /// `imported as local`, or one name as both; `imported` may be a
    /// string literal.
    ImportSpecifier {
        imported: NodeId,
        local: NodeId,
    },
    ImportDefaultSpecifier {
        local: NodeId,
    },
    /// `* as local`.
    ImportNamespaceSpecifier {
        local: NodeId,
    },
    /// `type: "json"` in a `with` clause.
    ImportAttribute {
        key: NodeId,
        value: NodeId,
    },
    /// `export` with a declaration, or with `{ ... }` and maybe a source.
    ExportNamed {
        declaration: Option<NodeId>,
        specifiers: List,
        source: Option<NodeId>,
        attributes: List,
    },
    /// `local as exported`, or one name as both; either may be a string
    /// literal.
    ExportSpecifier {
        local: NodeId,
        exported: NodeId,
    },
    /// `export default`, with a declaration or an expression.
    ExportDefault {
        declaration: NodeId,
    },
    /// `export * from "source"`, or `export * as exported from ...`.
    ExportAll {
        exported: Option<NodeId>,
        source: NodeId,
        attributes: List,
    },

    // Patterns.
    ObjectPattern {
        properties: List,
    },
    ArrayPattern {
        elements: List,
    },
    Rest {
        argument: NodeId,
    },
    AssignmentPattern {
        left: NodeId,
        right: NodeId,
    },
}

/// A parsed source: its nodes, the comments beside them, and the
/// warnings the parse had.
#[derive(Clone, Debug)]
pub struct Ast {
    /// Each node with its span, in one vector: making a node is one
    /// push, and reading both is one cache line.
    pub(crate) nodes: Vec<(Node, Span)>,
    pub(crate) lists: Vec<NodeId>,
    pub(crate) comments: Vec<Comment>,
    pub(crate) root: Option<NodeId>,
    pub(crate) warnings: Vec<Diagnostic>,
}

impl Ast {
    pub(crate) fn new(capacity: usize) -> Ast {
        let mut nodes = Vec::with_capacity(capacity + 1);
        // Index 0 is never a node, so an `Option<NodeId>` is 4 bytes.
        nodes.push((Node::Empty, Span::default()));
        Ast {
            nodes,
            lists: Vec::with_capacity(capacity / 2),
            comments: Vec::new(),
            root: None,
            warnings: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, span: Span, node: Node) -> NodeId {
        let id = NodeId::new(self.nodes.len());
        self.nodes.push((node, span));
        id
    }

    pub(crate) fn list(&mut self, ids: impl IntoIterator<Item = NodeId>) -> List {
        let start = self.lists.len() as u32;
        self.lists.extend(ids);
        List {
            start,
            len: self.lists.len() as u32 - start,
        }
    }

    /// The [`Node::Program`].
    pub fn root(&self) -> NodeId {
        self.root.expect("a parsed tree has a program")
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()].0
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()].0
    }

    pub fn span(&self, id: NodeId) -> Span {
        self.nodes[id.index()].1
    }

    pub(crate) fn set_span(&mut self, id: NodeId, span: Span) {
        self.nodes[id.index()].1 = span;
    }

    /// The nodes in `list`.
    pub fn get(&self, list: List) -> &[NodeId] {
        &self.lists[list.start as usize..(list.start + list.len) as usize]
    }

    /// How many nodes there are.
    pub fn len(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every node, in the order they were made: children before their
    /// parents, and the program last.
    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        (1..self.nodes.len()).map(NodeId::new)
    }

    /// The comments, in source order.
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// What the parse noticed that isn't an error, like a legacy octal
    /// literal in sloppy code.
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// The node inside any parentheses around `id`.
    pub fn unparenthesized(&self, mut id: NodeId) -> NodeId {
        while let Node::Paren { expression } = *self.node(id) {
            id = expression;
        }
        id
    }
}
