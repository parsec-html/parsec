// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Parsing again after an edit: [`Ast::reparse`] parses only the
//! smallest function body or statement around the edit, and keeps the
//! rest of the tree.
//!
//! The piece parsed again starts and ends where it did, at the same
//! characters, so nothing before it reads differently and the text after
//! it is the same text, read from the same place. What the parser knew
//! when it got there (strict code or not, the function it's in, the
//! loops and labels around it, the depth) comes from the nodes on the
//! way down. Where that isn't enough to be sure, the source is parsed
//! whole: an edit in a directive prologue or a parameter list, a
//! statement that starts with `import` or `export`, a piece that ends
//! anywhere but where it did, and a step limit, which counts the whole
//! source.
//!
//! The tree is the same as [`parse`] gives for the new source, node for
//! node; `tests/reparse.rs` checks that on random edits.

use parsec_base::{Diagnostic, Span};

use crate::ast::{
    Ast, CommentKind, FunctionKind, List, MethodKind, Node, NodeId, PropertyKind, Ref, refs,
};
use crate::lexer::{Goal, Lexer, SourceType};
use crate::parser::{Ctx, DEFAULT_MAX_DEPTH, Options, Resume, parse, parse_piece};
use crate::text;
use crate::token::{Keyword, Kind};

/// A change to a source: the bytes in `range` replaced by `text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit<'a> {
    pub range: Span,
    pub text: &'a str,
}

/// What [`Ast::reparse`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reparsed {
    /// The source with the edit made, which the tree is now of.
    pub source: String,
    /// The part of it parsed again, or `None` if it was parsed whole.
    pub reparsed: Option<Span>,
}

/// How many pieces, from the smallest out, are tried before parsing the
/// whole source.
const TRIES: usize = 3;

impl Ast {
    /// Makes `edit` to `src`, the source this tree was parsed from, and
    /// brings the tree up to date with it: the smallest function body or
    /// statement around the edit is parsed again and the rest kept, or
    /// where that can't be sure to give what [`parse`] would, the new
    /// source is parsed whole. The tree's source type is kept, and
    /// `options` gives the limits.
    ///
    /// A syntax error in the new source is the `Err`, and the tree is
    /// left as it was, of `src`.
    ///
    /// ```
    /// use parsec_js::{Edit, Options, Span, parse_script};
    ///
    /// let src = "function f() {\n  return 1;\n}\nf();\n";
    /// let mut ast = parse_script(src).unwrap();
    /// let edit = Edit { range: Span::new(24, 25), text: "42" };
    /// let done = ast.reparse(src, &edit, &Options::default()).unwrap();
    /// assert_eq!(done.source, "function f() {\n  return 42;\n}\nf();\n");
    /// // Only the `return` was parsed again.
    /// assert_eq!(done.reparsed, Some(Span::new(17, 27)));
    /// assert_eq!(ast.to_estree(&done.source), parse_script(&done.source).unwrap().to_estree(&done.source));
    /// ```
    pub fn reparse(
        &mut self,
        src: &str,
        edit: &Edit<'_>,
        options: &Options,
    ) -> Result<Reparsed, Diagnostic> {
        let (a, b) = (edit.range.start as usize, edit.range.end as usize);
        if a > b || b > src.len() || !src.is_char_boundary(a) || !src.is_char_boundary(b) {
            return Err(Diagnostic::error(
                "edit-out-of-range",
                edit.range,
                "the edit's range isn't whole characters of the source",
            ));
        }
        let mut source = String::with_capacity(src.len() - (b - a) + edit.text.len());
        source.push_str(&src[..a]);
        source.push_str(edit.text);
        source.push_str(&src[b..]);
        let Node::Program { source_type, .. } = *self.node(self.root()) else {
            unreachable!("the root is a program")
        };
        let options = Options {
            source_type,
            ..*options
        };
        // shortcut: a step limit counts the whole source's tokens, which a
        // piece can't, so with one it's always parsed whole.
        if options.limits.max_steps.is_none() && options.limits.check_input(source.len()).is_ok() {
            let delta = source.len() as i64 - src.len() as i64;
            if let Some(span) = self.try_pieces(src, &source, a as u32, b as u32, delta, &options) {
                return Ok(Reparsed {
                    source,
                    reparsed: Some(span),
                });
            }
        }
        *self = parse(&source, &options)?;
        Ok(Reparsed {
            source,
            reparsed: None,
        })
    }

    /// Parses the pieces around `a..b` again, the smallest first, and
    /// puts the first that works in place: the span it has now.
    fn try_pieces(
        &mut self,
        old: &str,
        new: &str,
        a: u32,
        b: u32,
        delta: i64,
        options: &Options,
    ) -> Option<Span> {
        if let Some(span) = self.edit_in_comment(old, new, a, b, delta, options.source_type) {
            return Some(span);
        }
        let path = self.path(a, b);
        let mut tries = 0;
        for k in (1..path.len()).rev() {
            let Some(body) = self.piece_kind(path[k - 1], path[k]) else {
                continue;
            };
            let span = self.span(path[k]);
            // The piece's first and last characters stay, so what's
            // before it and after it reads the same.
            if !(span.start < a && b < span.end) {
                continue;
            }
            tries += 1;
            if tries > TRIES {
                return None;
            }
            if let Some(s) = self.try_piece(old, new, &path[..=k], body, delta, options) {
                return Some(s);
            }
        }
        None
    }

    /// An edit inside a comment that leaves it one comment, ending where
    /// it did, with a line break in it only if it had one, changes no
    /// token: only the spans after it move. The comment's span now.
    fn edit_in_comment(
        &mut self,
        old: &str,
        new: &str,
        a: u32,
        b: u32,
        delta: i64,
        source: SourceType,
    ) -> Option<Span> {
        let i = self.comments.partition_point(|c| c.span.end <= a);
        let c = *self.comments.get(i)?;
        let close = match c.kind {
            CommentKind::Block => 2,
            CommentKind::Line => 0,
            _ => return None,
        };
        if !(c.span.start + 2 <= a && b + close <= c.span.end) {
            return None;
        }
        let t = Lexer::new(new, source)
            .starting_at(c.span.start, false)
            .next_token(Goal::Div);
        let end = u32::try_from(c.span.end as i64 + delta).ok()?;
        let breaks = |s: &str| s.contains(['\n', '\r', '\u{2028}', '\u{2029}']);
        if t.span != Span::new(c.span.start, end)
            || breaks(t.span.text(new)) != breaks(c.span.text(old))
        {
            return None;
        }
        let moved = |x: u32| {
            if x >= c.span.end {
                (x as i64 + delta) as u32
            } else {
                x
            }
        };
        for (_, s) in &mut self.nodes[1..] {
            *s = Span::new(moved(s.start), moved(s.end));
        }
        self.comments[i].span = t.span;
        for c in &mut self.comments[i + 1..] {
            c.span = Span::new(moved(c.span.start), moved(c.span.end));
        }
        for d in &mut self.warnings {
            d.span = Span::new(moved(d.span.start), moved(d.span.end));
        }
        Some(t.span)
    }

    /// The nodes from the root down to the smallest that holds `a..b`.
    fn path(&self, a: u32, b: u32) -> Vec<NodeId> {
        let mut path = vec![self.root()];
        loop {
            let id = *path.last().expect("the root is there");
            let mut next = None;
            self.each_child(id, &mut |c| {
                let s = self.span(c);
                if next.is_none() && s.start <= a && b <= s.end && c != id {
                    next = Some(c);
                }
            });
            match next {
                Some(c) => path.push(c),
                None => return path,
            }
        }
    }

    fn each_child(&self, id: NodeId, f: &mut dyn FnMut(NodeId)) {
        let mut node = *self.node(id);
        refs(&mut node, &mut |r| match r {
            Ref::Node(c) => f(*c),
            Ref::List(l) => {
                for &c in self.get(*l) {
                    f(c);
                }
            }
        });
    }

    /// Whether `child` of `parent` is a piece that can be parsed alone:
    /// `Some(true)` for a function body, `Some(false)` for an item of a
    /// statement list.
    fn piece_kind(&self, parent: NodeId, child: NodeId) -> Option<bool> {
        match *self.node(parent) {
            Node::Function(f) if f.body == child && !f.expression => Some(true),
            Node::Program { .. } | Node::Block { .. } | Node::StaticBlock { .. } => Some(false),
            Node::SwitchCase { test, .. } if test != Some(child) => Some(false),
            _ => None,
        }
    }

    /// Parses `path`'s last node again from `new` and puts it in place.
    fn try_piece(
        &mut self,
        old: &str,
        new: &str,
        path: &[NodeId],
        body: bool,
        delta: i64,
        options: &Options,
    ) -> Option<Span> {
        let unit = *path.last()?;
        let parent = path[path.len() - 2];
        let span = self.span(unit);
        let end = u32::try_from(span.end as i64 + delta).ok()?;
        let walk = self.walk(old, path)?;
        if !body {
            // The statement before looked at this one's first token to
            // decide where it ends: it has to be the same kind of token,
            // and the same token unless it's a name.
            let st = options.source_type;
            let (o, n) = (
                first_token(old, st, span.start),
                first_token(new, st, span.start),
            );
            let same = o.kind == n.kind
                && o.flags == n.flags
                && (o.kind == Kind::Ident || o.text(old) == n.text(new));
            let module_item = matches!(o.kind, Kind::Keyword(Keyword::Import | Keyword::Export));
            if !same || module_item {
                return None;
            }
            // A string that starts a statement in a directive prologue
            // may be a directive, which changes what's strict after it.
            if o.kind == Kind::String && self.prologue_parent(path) {
                let items = self.list_of(parent);
                let at = items.iter().position(|&i| i == unit)?;
                if items[..at].iter().all(|&i| self.is_directive(i)) {
                    return None;
                }
            }
        }
        let Extent { n0, n1, lists } = self.extent(unit)?;
        let max_depth = options.limits.max_depth.unwrap_or(DEFAULT_MAX_DEPTH);
        // Each node above takes at most two levels: an expression, and
        // the statement, function or class it is.
        let bound = 2 * path.len() as u32;
        if bound >= max_depth {
            return None;
        }
        let top = matches!(self.node(parent), Node::Program { .. });
        let piece = parse_piece(
            new,
            options,
            Resume {
                start: span.start,
                body,
                ctx: walk.ctx,
                labels: walk.labels,
                classes: walk.classes.len(),
                depth: if top { 0 } else { bound },
                max_depth,
            },
        )?;
        if piece.end != end {
            return None;
        }
        // A private name the piece uses is declared by a class around it.
        let declared = |n: &String| walk.classes.iter().any(|c| c.contains(n));
        if !piece.private_names.iter().all(declared) {
            return None;
        }
        if body && self.directives(old, unit) != piece_directives(&piece.ast, new) {
            return None;
        }
        let (l0, l1) = match lists {
            Some(l) => l,
            // Neither has a list of its own: none move, though ids in
            // them may.
            None if !has_lists(&piece.ast) => (0, 0),
            None if !body => {
                let p = self.list_point(path)?;
                (p, p)
            }
            None => return None,
        };
        // With no entries before, an empty list elsewhere at the same
        // place could have been made before the piece or after it, and
        // only one of those moves.
        if l0 == l1 && piece.ast.lists.len() > 1 && self.empty_list_at(n1 + 1, l0) {
            return None;
        }
        self.splice(piece.ast, (n0, n1, l0, l1), span, end, delta);
        Some(Span::new(span.start, end))
    }

    /// The statements in a list-holding node.
    fn list_of(&self, id: NodeId) -> &[NodeId] {
        match *self.node(id) {
            Node::Program { body, .. } | Node::Block { body } | Node::StaticBlock { body } => {
                self.get(body)
            }
            Node::SwitchCase { consequent, .. } => self.get(consequent),
            _ => &[],
        }
    }

    /// Whether `path`'s last node is in a list that has a directive
    /// prologue: the program's, or a function body's.
    fn prologue_parent(&self, path: &[NodeId]) -> bool {
        let parent = path[path.len() - 2];
        match self.node(parent) {
            Node::Program { .. } => true,
            Node::Block { .. } => {
                path.len() >= 3
                    && matches!(self.node(path[path.len() - 3]), Node::Function(f) if f.body == parent)
            }
            _ => false,
        }
    }

    fn is_directive(&self, id: NodeId) -> bool {
        matches!(
            self.node(id),
            Node::Expression {
                directive: true,
                ..
            }
        )
    }

    /// The text of the directives a body or program starts with.
    fn directives<'s>(&self, src: &'s str, id: NodeId) -> Vec<&'s str> {
        self.list_of(id)
            .iter()
            .take_while(|&&i| self.is_directive(i))
            .map(|&i| self.span(i).text(src))
            .collect()
    }

    /// Whether the directives at the start of `id`'s statements say
    /// `"use strict"`.
    fn says_use_strict(&self, src: &str, id: NodeId) -> bool {
        self.list_of(id)
            .iter()
            .take_while(|&&i| self.is_directive(i))
            .any(|&i| match *self.node(i) {
                Node::Expression { expression, .. } => {
                    let s = self.span(expression);
                    s.len() >= 2 && Span::new(s.start + 1, s.end - 1).text(src) == "use strict"
                }
                _ => false,
            })
    }

    /// What the parser knew on its way down `path` to its last node.
    fn walk(&self, src: &str, path: &[NodeId]) -> Option<Walk> {
        let module = matches!(
            self.node(path[0]),
            Node::Program {
                source_type: SourceType::Module,
                ..
            }
        );
        let mut w = Walk {
            ctx: Ctx {
                strict: module,
                await_: module,
                ..Ctx::default()
            },
            labels: Vec::new(),
            classes: Vec::new(),
        };
        for i in 0..path.len() - 1 {
            let (p, c) = (path[i], path[i + 1]);
            match *self.node(p) {
                Node::Program { .. } => {
                    if self.says_use_strict(src, p) {
                        w.ctx.strict = true;
                    }
                }
                Node::Block { .. } => {
                    let is_body =
                        i > 0 && matches!(self.node(path[i - 1]), Node::Function(f) if f.body == p);
                    if is_body && self.says_use_strict(src, p) {
                        w.ctx.strict = true;
                    }
                }
                Node::Function(f) => {
                    // A parameter list is checked with what follows it.
                    if c != f.body {
                        return None;
                    }
                    w.labels.clear();
                    if f.kind == FunctionKind::Arrow {
                        w.ctx = Ctx {
                            yield_: false,
                            await_: f.is_async,
                            ret: true,
                            static_block: false,
                            loops: 0,
                            breakable: 0,
                            labels: 0,
                            ..w.ctx
                        };
                    } else {
                        let mut ctx = Ctx {
                            strict: w.ctx.strict,
                            yield_: f.generator,
                            await_: f.is_async,
                            ret: true,
                            new_target: true,
                            ..Ctx::default()
                        };
                        if let Some(super_call) = self.method_of(&path[..=i]) {
                            ctx.super_prop = true;
                            ctx.super_call = super_call;
                        }
                        w.ctx = ctx;
                    }
                }
                Node::Class(class) => {
                    w.ctx.strict = true;
                    if c == class.body {
                        w.classes.push(self.private_names(src, class.body));
                    }
                }
                Node::Field { value, .. } if value == Some(c) => {
                    w.labels.clear();
                    w.ctx = Ctx {
                        yield_: false,
                        await_: false,
                        ret: false,
                        new_target: true,
                        super_prop: true,
                        super_call: false,
                        no_arguments: true,
                        static_block: false,
                        loops: 0,
                        breakable: 0,
                        labels: 0,
                        ..w.ctx
                    };
                }
                Node::StaticBlock { .. } => {
                    w.labels.clear();
                    w.ctx = Ctx {
                        strict: true,
                        new_target: true,
                        super_prop: true,
                        no_arguments: true,
                        static_block: true,
                        ..Ctx::default()
                    };
                }
                Node::While { body, .. }
                | Node::DoWhile { body, .. }
                | Node::For { body, .. }
                | Node::ForIn { body, .. }
                | Node::ForOf { body, .. }
                    if body == c =>
                {
                    w.ctx.loops += 1;
                    w.ctx.breakable += 1;
                }
                Node::Switch { discriminant, .. } if discriminant != c => w.ctx.breakable += 1,
                Node::Labeled { label, body } if body == c => {
                    let span = self.span(label);
                    let name = text::ident_name(span.text(src)).into_owned();
                    w.labels.push((name, span, self.labels_loop(body)));
                }
                _ => {}
            }
        }
        Some(w)
    }

    /// Whether a label on `id` labels a loop: `id` is one, or more labels
    /// on one.
    fn labels_loop(&self, mut id: NodeId) -> bool {
        loop {
            match *self.node(id) {
                Node::Labeled { body, .. } => id = body,
                Node::While { .. }
                | Node::DoWhile { .. }
                | Node::For { .. }
                | Node::ForIn { .. }
                | Node::ForOf { .. } => return true,
                _ => return false,
            }
        }
    }

    /// If the function at the end of `path` is a method, whether it may
    /// call `super()`: a derived class's constructor.
    fn method_of(&self, path: &[NodeId]) -> Option<bool> {
        let f = path[path.len() - 1];
        let parent = *path.get(path.len().checked_sub(2)?)?;
        match *self.node(parent) {
            Node::Method { value, kind, .. } if value == f => {
                let class = path.get(path.len().checked_sub(4)?)?;
                let derived =
                    matches!(self.node(*class), Node::Class(c) if c.super_class.is_some());
                Some(kind == MethodKind::Constructor && derived)
            }
            Node::Property {
                value,
                kind,
                method,
                ..
            } if value == f && (method || kind != PropertyKind::Init) => Some(false),
            _ => None,
        }
    }

    /// The private names a class body declares, without their `#`.
    fn private_names(&self, src: &str, body: NodeId) -> Vec<String> {
        let mut names = Vec::new();
        if let Node::ClassBody { body } = *self.node(body) {
            for &e in self.get(body) {
                let key = match *self.node(e) {
                    Node::Method { key, .. } | Node::Field { key, .. } => key,
                    _ => continue,
                };
                if let Node::PrivateIdentifier = self.node(key) {
                    let s = self.span(key);
                    let raw = Span::new(s.start + 1, s.end).text(src);
                    names.push(text::ident_name(raw).into_owned());
                }
            }
        }
        names
    }

    /// The nodes and list entries the subtree at `root` was made of, as
    /// ranges of the arena: `n0..=root`, and `l0..l1` if it has entries,
    /// or else where its empty lists say they'd have gone. `None` if
    /// they aren't ranges of its own alone.
    fn extent(&self, root: NodeId) -> Option<Extent> {
        let mut ids = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            ids.push(id.index());
            self.each_child(id, &mut |c| stack.push(c));
        }
        ids.sort_unstable();
        ids.dedup();
        let (n0, n1) = (ids[0], root.index());
        if *ids.last()? != n1 || ids.len() != n1 - n0 + 1 {
            return None;
        }
        let (mut l0, mut l1, mut total) = (usize::MAX, 0, 0);
        // An empty list's start is where the next entry would have gone;
        // one at 0 may be `List::default()` instead.
        let mut empty = None;
        for &i in &ids {
            let mut node = self.nodes[i].0;
            refs(&mut node, &mut |r| {
                if let Ref::List(l) = r {
                    if l.len > 0 {
                        l0 = l0.min(l.start as usize);
                        l1 = l1.max((l.start + l.len) as usize);
                        total += l.len as usize;
                    } else if l.start > 0 {
                        empty = Some(l.start as usize);
                    }
                }
            });
        }
        if total == 0 {
            let lists = empty.map(|e| (e, e));
            return Some(Extent { n0, n1, lists });
        }
        let lists = Some((l0, l1));
        (total == l1 - l0).then_some(Extent { n0, n1, lists })
    }

    /// Whether a node from `from` on has an empty list starting at `at`.
    fn empty_list_at(&self, from: usize, at: usize) -> bool {
        let mut found = false;
        for (node, _) in &self.nodes[from..] {
            let mut node = *node;
            refs(&mut node, &mut |r| {
                if let Ref::List(l) = r {
                    found |= l.len == 0 && l.start as usize == at;
                }
            });
            if found {
                return true;
            }
        }
        false
    }

    /// Where the list entries of `path`'s last node, a statement with
    /// none, would go: before those of the first statement after it that
    /// has some, or else before its list's own.
    fn list_point(&self, path: &[NodeId]) -> Option<usize> {
        let unit = path[path.len() - 1];
        let parent = path[path.len() - 2];
        let items = self.list_of(parent);
        let at = items.iter().position(|&i| i == unit)?;
        for &next in &items[at + 1..] {
            if let Some((l0, l1)) = self.extent(next)?.lists
                && l1 > l0
            {
                return Some(l0);
            }
        }
        let list = match *self.node(parent) {
            Node::Program { body, .. } | Node::Block { body } | Node::StaticBlock { body } => body,
            Node::SwitchCase { consequent, .. } => consequent,
            _ => return None,
        };
        Some(list.start as usize)
    }

    /// Puts `piece`'s nodes in place of `n0..=n1` and its lists in place
    /// of `l0..l1`, renumbering what comes after, and moves every span,
    /// comment and warning after the old piece's `span` by `delta`.
    fn splice(
        &mut self,
        piece: Ast,
        (n0, n1, l0, l1): (usize, usize, usize, usize),
        span: Span,
        end: u32,
        delta: i64,
    ) {
        let dn = (piece.nodes.len() - 1) as i64 - (n1 - n0 + 1) as i64;
        let dl = (piece.lists.len() - 1) as i64 - (l1 - l0) as i64;
        let moved = |x: u32| {
            if x >= span.end {
                (x as i64 + delta) as u32
            } else {
                x
            }
        };
        let old_id = |id: NodeId| {
            if id.index() >= n1 {
                NodeId::new((id.index() as i64 + dn) as usize)
            } else {
                id
            }
        };
        let new_id = |id: NodeId| NodeId::new(id.index() - 1 + n0);
        // Nodes made before the piece refer to nothing after it, and are
        // all before it in the source: only those after it change.
        for (node, s) in &mut self.nodes[n1 + 1..] {
            *s = Span::new(moved(s.start), moved(s.end));
            refs(node, &mut |r| match r {
                Ref::Node(id) => *id = old_id(*id),
                Ref::List(l) => {
                    if l.start as usize >= l1 && (l.len > 0 || l.start > 0) {
                        l.start = (l.start as i64 + dl) as u32;
                    }
                }
            });
        }
        // Entries before the piece's hold only ids before it, so this is
        // all of them that change, wherever the piece's entries go.
        for id in &mut self.lists[l1.min(l0)..] {
            *id = old_id(*id);
        }
        let mut nodes = piece.nodes;
        for (node, _) in &mut nodes[1..] {
            refs(node, &mut |r| match r {
                Ref::Node(id) => *id = new_id(*id),
                Ref::List(l) => {
                    if l.len > 0 || l.start > 0 {
                        *l = List {
                            start: l.start - 1 + l0 as u32,
                            len: l.len,
                        };
                    }
                }
            });
        }
        self.nodes.splice(n0..=n1, nodes.into_iter().skip(1));
        self.lists
            .splice(l0..l1, piece.lists.into_iter().skip(1).map(new_id));
        self.root = self.root.map(old_id);
        let keep = |s: Span| s.end <= span.start || s.start >= span.end;
        splice_sorted(
            &mut self.comments,
            piece.comments,
            |c| c.span,
            |c, s| c.span = s,
            span,
            keep,
            moved,
        );
        splice_sorted(
            &mut self.warnings,
            piece.warnings,
            |d| d.span,
            |d, s| d.span = s,
            span,
            keep,
            moved,
        );
        debug_assert!(end >= span.start);
    }
}

/// Whether a piece made any list, empty or not.
fn has_lists(piece: &Ast) -> bool {
    piece.nodes[1..].iter().any(|(node, _)| {
        let mut node = *node;
        let mut any = false;
        refs(&mut node, &mut |r| {
            if let Ref::List(l) = r {
                any |= l.len > 0 || l.start > 0;
            }
        });
        any
    })
}

/// The arena ranges a subtree was made of: nodes `n0..=n1`, and list
/// entries `lists`, empty where its empty lists put it, or `None` if it
/// has no list.
struct Extent {
    n0: usize,
    n1: usize,
    lists: Option<(usize, usize)>,
}

/// What [`Ast::walk`] found.
struct Walk {
    ctx: Ctx,
    labels: Vec<(String, Span, bool)>,
    /// The private names each class body around declares.
    classes: Vec<Vec<String>>,
}

/// The directives a parsed function body starts with.
fn piece_directives<'s>(piece: &Ast, src: &'s str) -> Vec<&'s str> {
    piece.directives(src, piece.root())
}

/// The token at `start`, read as the parser first reads a token.
fn first_token(src: &str, source: SourceType, start: u32) -> crate::token::Token {
    Lexer::new(src, source)
        .starting_at(start, false)
        .next_token(Goal::Div)
}

/// Replaces the items of sorted `old` inside `span` with `new`'s, and
/// moves those after it.
fn splice_sorted<T>(
    old: &mut Vec<T>,
    new: Vec<T>,
    span_of: impl Fn(&T) -> Span,
    set: impl Fn(&mut T, Span),
    span: Span,
    keep: impl Fn(Span) -> bool,
    moved: impl Fn(u32) -> u32,
) {
    let from = old.partition_point(|t| span_of(t).start < span.start);
    let to = from + old[from..].iter().take_while(|t| !keep(span_of(t))).count();
    for t in &mut old[to..] {
        let s = span_of(t);
        set(t, Span::new(moved(s.start), moved(s.end)));
    }
    old.splice(from..to, new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_script;

    /// The tree after `edit`, and the one a parse of the new source
    /// gives, are the same arena, node for node and list for list.
    fn same(src: &str, at: u32, remove: u32, text: &str) -> Option<(Span, String)> {
        let options = Options::default();
        let mut ast = parse_script(src).unwrap();
        let edit = Edit {
            range: Span::new(at, at + remove),
            text,
        };
        let done = ast.reparse(src, &edit, &options).unwrap();
        let full = parse(&done.source, &options).unwrap();
        assert_eq!(ast.nodes, full.nodes, "{:?}", done.source);
        assert_eq!(ast.lists, full.lists, "{:?}", done.source);
        assert_eq!(ast.comments, full.comments);
        assert_eq!(ast.warnings, full.warnings);
        assert_eq!(ast.root, full.root);
        done.reparsed.map(|s| (s, done.source))
    }

    const SRC: &str = "var a = 1; // one\nfunction f(x) {\n  if (x) { g(x, [1, 2]); }\n  /* two */ return new Foo;\n}\nclass C extends B { #p = 1; constructor() { super(); this.#p++; } }\nlabel: for (;;) { while (a) { continue label; } }\nf(() => { return a ? 010 : b; });\n";

    #[test]
    fn a_statement_is_parsed_again_in_place() {
        // Inside `g(x, [1, 2])`: the call's statement, in the block, is
        // the piece.
        let at = SRC.find("[1, 2]").unwrap() as u32 + 1;
        let (span, new) = same(SRC, at, 1, "100, h(3)").unwrap();
        assert_eq!(span.text(&new), "g(x, [100, h(3), 2]);");
        // A comment added and one taken away.
        let at = SRC.find("/* two */").unwrap() as u32;
        assert!(same(SRC, at + 3, 3, "three").is_some());
        assert!(same(SRC, at + 2, 0, "*/ /*").is_some());
    }

    #[test]
    fn what_was_known_there_is_known_again() {
        // `super()`, a private name, a labelled `continue` and a legacy
        // octal in sloppy code, each parsed again where they're allowed.
        for needle in ["super();", "this.#p++", "continue label", "010"] {
            let at = SRC.find(needle).unwrap() as u32;
            assert!(same(SRC, at, 0, " ").is_some(), "{needle}");
        }
        // A private name no class declares: the whole source decides, and
        // it's an error.
        let mut ast = parse_script(SRC).unwrap();
        let at = SRC.find("this.#p").unwrap() as u32 + 6;
        let edit = Edit {
            range: Span::new(at, at + 1),
            text: "q",
        };
        let err = ast.reparse(SRC, &edit, &Options::default()).unwrap_err();
        assert_eq!(err.code, "undeclared-private-name");
        assert_eq!(
            ast.to_estree(SRC),
            parse_script(SRC).unwrap().to_estree(SRC)
        );
    }

    #[test]
    fn edits_that_reach_outside_fall_back() {
        // Opening a comment, a template or a string: the piece no longer
        // ends where it did.
        for text in ["/*", "`", "'"] {
            let at = SRC.find("g(x").unwrap() as u32;
            let mut ast = parse_script(SRC).unwrap();
            let edit = Edit {
                range: Span::new(at, at),
                text,
            };
            if let Ok(done) = ast.reparse(SRC, &edit, &Options::default()) {
                assert_eq!(done.reparsed, None);
            }
        }
        // `"use strict"` in a prologue changes everything after it.
        let src = "function f() {\n  'use stric';\n  with (a) {}\n}\n";
        let mut ast = parse_script(src).unwrap();
        let edit = Edit {
            range: Span::new(27, 27),
            text: "t",
        };
        let err = ast.reparse(src, &edit, &Options::default()).unwrap_err();
        assert_eq!(err.code, "with-in-strict");
        // An edit that joins two statements.
        assert_eq!(same("a = b\n;(c)\n", 6, 1, ""), None);
    }

    #[test]
    fn a_bad_range_is_refused() {
        let mut ast = parse_script("é").unwrap();
        for range in [Span::new(1, 2), Span::new(0, 3)] {
            let edit = Edit { range, text: "" };
            let err = ast.reparse("é", &edit, &Options::default()).unwrap_err();
            assert_eq!(err.code, "edit-out-of-range");
        }
    }

    #[test]
    fn random_edits_keep_the_arena_exact() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut rand = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let pieces = [
            " ", "\n", "x", "1", ";", "{", "}", "(", ")", "/", "'", "`", "${", "a.b", "f()", "=>",
        ];
        let mut reused = 0;
        for _ in 0..3000 {
            let at = rand(SRC.len()) as u32;
            let remove = rand(3).min(SRC.len() - at as usize) as u32;
            let text = pieces[rand(pieces.len())];
            let options = Options::default();
            let mut ast = parse_script(SRC).unwrap();
            let edit = Edit {
                range: Span::new(at, at + remove),
                text,
            };
            if !SRC.is_char_boundary((at + remove) as usize) {
                continue;
            }
            match (
                ast.reparse(SRC, &edit, &options),
                parse(&edit_src(SRC, &edit), &options),
            ) {
                (Ok(done), Ok(full)) => {
                    assert_eq!(ast.nodes, full.nodes, "{:?}", done.source);
                    assert_eq!(ast.lists, full.lists, "{:?}", done.source);
                    assert_eq!(ast.comments, full.comments, "{:?}", done.source);
                    assert_eq!(ast.root, full.root);
                    reused += usize::from(done.reparsed.is_some());
                }
                (Err(a), Err(b)) => assert_eq!(a, b),
                (r, f) => panic!(
                    "{edit:?}: {:?} but {:?}",
                    r.map(|d| d.reparsed),
                    f.map(|_| ())
                ),
            }
        }
        assert!(reused > 300, "{reused}");
    }

    fn edit_src(src: &str, e: &Edit<'_>) -> String {
        format!(
            "{}{}{}",
            &src[..e.range.start as usize],
            e.text,
            &src[e.range.end as usize..]
        )
    }
}
