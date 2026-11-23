// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The parser: the prolog, elements and their attributes, character data,
//! references, and the tree they make. The internal subset is in `dtd.rs`.
//!
//! Elements are kept on a stack of their own, so nesting costs no
//! recursion. Only entity expansion recurses, and its depth is capped.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use parsec_base::{Budget, Diagnostic, Span};

use crate::Options;
use crate::chars::{is_char, is_name_start, is_space, name_len};
use crate::tree::{
    AttrData, Data, Document, NONE, NS_XML, NS_XMLNS, PREFIX_XML, PREFIX_XMLNS, Str, XML_NS,
    XMLNS_NS, XmlDecl,
};

pub(crate) type R<T> = Result<T, Diagnostic>;

pub(crate) fn err(
    code: &'static str,
    span: Span,
    message: impl Into<Cow<'static, str>>,
) -> Diagnostic {
    Diagnostic::error(code, span, message)
}

/// However deep [`Options::max_entity_depth`] says entities may nest,
/// they nest no deeper than this, so expanding them can't run the
/// parser's own stack out.
pub const ENTITY_DEPTH_CEILING: u32 = 64;

/// A place in some text: the document, or an entity's replacement text.
#[derive(Clone, Copy)]
pub(crate) struct Cur<'t> {
    pub s: &'t str,
    pub i: usize,
    /// For replacement text, the reference in the document it came from:
    /// everything it produces has that span. Replacement text is already
    /// normalized, so its line ends are left alone.
    pub origin: Option<Span>,
}

impl<'t> Cur<'t> {
    pub fn new(s: &'t str, origin: Option<Span>) -> Cur<'t> {
        Cur { s, i: 0, origin }
    }

    pub fn bytes(&self) -> &'t [u8] {
        self.s.as_bytes()
    }

    pub fn peek(&self) -> Option<u8> {
        self.bytes().get(self.i).copied()
    }

    pub fn peek_at(&self, k: usize) -> Option<u8> {
        self.bytes().get(self.i + k).copied()
    }

    pub fn eof(&self) -> bool {
        self.i >= self.s.len()
    }

    pub fn at(&self, lit: &str) -> bool {
        self.bytes()[self.i..].starts_with(lit.as_bytes())
    }

    pub fn eat(&mut self, lit: &str) -> bool {
        let yes = self.at(lit);
        if yes {
            self.i += lit.len();
        }
        yes
    }

    pub fn eat_byte(&mut self, b: u8) -> bool {
        let yes = self.peek() == Some(b);
        if yes {
            self.i += 1;
        }
        yes
    }

    /// Skips `S`, saying whether there was any.
    pub fn space(&mut self) -> bool {
        let start = self.i;
        while self.peek().is_some_and(is_space) {
            self.i += 1;
        }
        self.i > start
    }

    pub fn raw(&self) -> bool {
        self.origin.is_some()
    }

    pub fn span(&self, a: usize, b: usize) -> Span {
        self.origin.unwrap_or(Span::new(a as u32, b as u32))
    }

    /// The character here, or the end.
    pub fn here(&self) -> Span {
        let len = self.s[self.i..].chars().next().map_or(0, char::len_utf8);
        self.span(self.i, self.i + len)
    }

    pub fn name(&mut self) -> Option<&'t str> {
        let n = name_len(&self.s[self.i..]);
        (n > 0).then(|| {
            let name = &self.s[self.i..self.i + n];
            self.i += n;
            name
        })
    }
}

/// Bytes that end a run of character data: markup, references, `]]>`,
/// line ends to normalize, and the start of anything that isn't a `Char`.
static TEXT: [bool; 256] = {
    let mut t = [false; 256];
    let mut b = 0;
    while b < 0x20 {
        t[b] = !matches!(b as u8, b'\t' | b'\n');
        b += 1;
    }
    t[b'<' as usize] = true;
    t[b'&' as usize] = true;
    t[b']' as usize] = true;
    t[0xEF] = true;
    t
};

/// The same for attribute values, where white space becomes a space.
static ATTR: [bool; 256] = {
    let mut t = [false; 256];
    let mut b = 0;
    while b < 0x20 {
        t[b] = true;
        b += 1;
    }
    t[b'<' as usize] = true;
    t[b'&' as usize] = true;
    t[b'"' as usize] = true;
    t[b'\'' as usize] = true;
    t[0xEF] = true;
    t
};

/// The same for comments, PIs and CDATA: only line ends and non-`Char`s.
static PLAIN: [bool; 256] = {
    let mut t = [false; 256];
    let mut b = 0;
    while b < 0x20 {
        t[b] = !matches!(b as u8, b'\t' | b'\n');
        b += 1;
    }
    t[0xEF] = true;
    t
};

/// U+FFFE and U+FFFF, the two non-`Char`s past the controls, are
/// EF BF BE and EF BF BF.
fn non_char_at(b: &[u8], i: usize) -> bool {
    b[i] == 0xEF && b.get(i + 1) == Some(&0xBF) && matches!(b.get(i + 2), Some(0xBE | 0xBF))
}

/// The declared or predefined entities, as the parser keeps them.
#[derive(Debug)]
pub(crate) struct Entity {
    /// An internal entity's replacement text.
    pub value: Option<Rc<str>>,
    /// Unparsed (`NDATA`).
    pub unparsed: bool,
    /// Declared inside a parameter entity's replacement text, which a
    /// standalone document can't rely on.
    pub in_pe: bool,
    /// Being expanded now, so a reference to it is a recursion.
    pub active: bool,
    /// Its not being read has been reported.
    pub warned: bool,
}

/// An attribute the internal subset declared.
#[derive(Debug)]
pub(crate) struct AttDef {
    pub name: u32,
    /// Declared as anything but `CDATA`, so its value's spaces collapse.
    pub tokenized: bool,
    /// Declared as an `ID`.
    pub id: bool,
    pub default: Option<String>,
    /// The default, once it's in the document's buffer.
    pub cached: Option<Str>,
}

/// What the DTD declared.
#[derive(Debug, Default)]
pub(crate) struct Dtd {
    pub general: HashMap<String, Entity>,
    pub params: HashMap<String, Entity>,
    /// By element name id.
    pub attlists: HashMap<u32, Vec<AttDef>>,
    /// A parameter entity wasn't read, so from here on entity and
    /// attribute-list declarations aren't processed: the entity may have
    /// declared them first.
    pub skip: bool,
    pub skip_warned: bool,
}

struct Open {
    node: u32,
    name: u32,
    /// The namespace scope's mark when it opened.
    mark: usize,
    /// Its start tag's `<`.
    start: usize,
}

struct Pending {
    name: u32,
    value: Str,
    span: Span,
    specified: bool,
    ns: u32,
}

/// Namespace bindings by prefix id, and how to undo them.
struct Scope {
    current: Vec<u32>,
    undo: Vec<(u32, u32)>,
}

impl Scope {
    fn new() -> Scope {
        let mut current = vec![NONE; 3];
        current[0] = 0;
        current[PREFIX_XML as usize] = NS_XML;
        Scope {
            current,
            undo: Vec::new(),
        }
    }

    fn bind(&mut self, prefix: u32, uri: u32) {
        let p = prefix as usize;
        if p >= self.current.len() {
            self.current.resize(p + 1, NONE);
        }
        self.undo.push((prefix, self.current[p]));
        self.current[p] = uri;
    }

    fn get(&self, prefix: u32) -> u32 {
        self.current.get(prefix as usize).copied().unwrap_or(NONE)
    }

    fn pop(&mut self, mark: usize) {
        while self.undo.len() > mark {
            let (p, old) = self.undo.pop().unwrap();
            self.current[p as usize] = old;
        }
    }
}

pub(crate) struct Parser<'s, 'o> {
    pub src: &'s str,
    pub opts: &'o Options,
    pub doc: Document,
    pub dtd: Dtd,
    stack: Vec<Open>,
    scope: Scope,
    pending: Vec<Pending>,
    sorted: Vec<(u32, u32)>,
    steps: Budget,
    pub expansion: Budget,
    pub entity_depth: u32,
    pub standalone: bool,
    pub has_external_subset: bool,
    pub has_pe_refs: bool,
    /// Inside the `<!DOCTYPE>`.
    pub in_dtd: bool,
    /// Undeclared entities already reported as not loaded.
    warned: HashSet<String>,
}

impl<'s, 'o> Parser<'s, 'o> {
    pub fn new(src: &'s str, opts: &'o Options) -> Parser<'s, 'o> {
        let allowance = opts
            .entity_allowance
            .saturating_add(opts.entity_ratio.saturating_mul(src.len() as u64));
        Parser {
            src,
            opts,
            doc: Document::new(opts.namespaces, src.len()),
            dtd: Dtd::default(),
            stack: Vec::new(),
            scope: Scope::new(),
            pending: Vec::new(),
            sorted: Vec::new(),
            steps: opts.limits.budget(),
            expansion: Budget::new(allowance),
            entity_depth: 0,
            standalone: false,
            has_external_subset: false,
            has_pe_refs: false,
            in_dtd: false,
            warned: HashSet::new(),
        }
    }

    /// Whether a reference to an undeclared entity is allowed: the DTD
    /// has parts this parser doesn't read, which might declare it.
    pub fn lenient(&self) -> bool {
        !self.standalone && (self.has_external_subset || self.has_pe_refs)
    }

    pub fn step(&mut self, n: u64, span: Span) -> R<()> {
        self.steps.spend(n).map_err(|_| {
            err(
                "step-limit",
                span,
                "the parse did more work than its step limit allows",
            )
        })
    }

    pub fn warn(&mut self, d: Diagnostic) {
        self.doc.warnings.push(d);
    }

    /// The text written to the buffer since `start`.
    pub fn str_from(&self, start: usize, span: Span) -> R<Str> {
        let end = self.doc.text.len();
        if end > u32::MAX as usize {
            return Err(err("too-large", span, "the document's text is over 4 GiB"));
        }
        Ok(Str {
            start: start as u32,
            len: (end - start) as u32,
        })
    }

    fn node(&mut self, parent: u32, span: Span, data: Data) -> R<u32> {
        if self.doc.nodes.len() >= NONE as usize - 1 {
            return Err(err("too-large", span, "the document has too many nodes"));
        }
        self.step(1, span)?;
        Ok(self.doc.append(parent, span, data))
    }

    fn parent(&self) -> u32 {
        self.stack.last().map_or(0, |o| o.node)
    }

    /// Interns an element or attribute name, checking it's a `QName`
    /// when namespaces are on.
    pub fn qname(&mut self, name: &str, span: Span) -> R<u32> {
        if let Some(id) = self.doc.names.get(name) {
            return Ok(id);
        }
        let mut colon = NONE;
        if self.opts.namespaces
            && let Some(c) = name.find(':')
        {
            let local = &name[c + 1..];
            if c == 0 || local.contains(':') || !local.chars().next().is_some_and(is_name_start) {
                return Err(err(
                    "invalid-qname",
                    span,
                    format!(
                        "`{name}` isn't a qualified name: a prefix, one colon, and a local name"
                    ),
                ));
            }
            colon = c as u32;
        }
        Ok(self.doc.names.insert(name, colon))
    }

    /// A name that can't have a colon with namespaces on: an entity's, a
    /// PI target, a notation's.
    pub fn no_colon(&self, name: &str, span: Span, what: &str) -> R<()> {
        if self.opts.namespaces && name.contains(':') {
            return Err(err(
                "colon-in-name",
                span,
                format!("{what} `{name}` can't contain a colon"),
            ));
        }
        Ok(())
    }

    pub fn need_space(&self, cur: &mut Cur, what: &'static str) -> R<()> {
        if cur.space() {
            return Ok(());
        }
        Err(self.expected(cur, what))
    }

    /// An error for something else being where `what` should be.
    pub fn expected(&self, cur: &Cur, what: &str) -> Diagnostic {
        if cur.eof() {
            return err(
                "unexpected-eof",
                cur.here(),
                format!("the input ends where {what} should be"),
            );
        }
        if cur.peek() == Some(b'%') && self.in_dtd {
            return err(
                "pe-in-markup-declaration",
                cur.here(),
                "a parameter entity reference can't be inside a declaration in the internal subset",
            );
        }
        let c = cur.s[cur.i..].chars().next().unwrap_or(' ');
        err(
            "unexpected-char",
            cur.here(),
            format!("expected {what}, found `{}`", c.escape_debug()),
        )
    }

    /// Copies `cur.s[a..b]` to the buffer, checking each character is a
    /// `Char` and normalizing line ends outside replacement text.
    pub fn push_checked(&mut self, cur: &Cur, a: usize, b: usize) -> R<()> {
        let bytes = cur.bytes();
        let (mut from, mut i) = (a, a);
        while i < b {
            if !PLAIN[bytes[i] as usize] {
                i += 1;
                continue;
            }
            match bytes[i] {
                b'\r' => {
                    if !cur.raw() {
                        self.doc.text.push_str(&cur.s[from..i]);
                        self.doc.text.push('\n');
                        i += 1;
                        if bytes.get(i) == Some(&b'\n') && i < b {
                            i += 1;
                        }
                        from = i;
                        continue;
                    }
                }
                0xEF if !non_char_at(bytes, i) => {}
                _ => return Err(self.not_char(cur, i)),
            }
            i += 1;
        }
        self.doc.text.push_str(&cur.s[from..b]);
        Ok(())
    }

    /// Checks `cur.s[a..b]` has only `Char`s, copying nothing.
    pub fn check_chars(&self, cur: &Cur, a: usize, b: usize) -> R<()> {
        let bytes = cur.bytes();
        for i in a..b {
            let bad = match bytes[i] {
                b'\r' => false,
                0xEF => non_char_at(bytes, i),
                c => PLAIN[c as usize],
            };
            if bad {
                return Err(self.not_char(cur, i));
            }
        }
        Ok(())
    }

    pub fn not_char(&self, cur: &Cur, i: usize) -> Diagnostic {
        let c = cur.s[i..].chars().next().unwrap_or('\0');
        err(
            "invalid-char",
            cur.span(i, i + c.len_utf8()),
            format!("U+{:04X} isn't allowed in XML", c as u32),
        )
    }

    // The document.

    pub fn document(&mut self) -> R<()> {
        self.opts.limits.check_input(self.src.len())?;
        let src = self.src;
        let mut cur = Cur::new(src, None);
        // A byte order mark that came through as a character.
        cur.eat("\u{FEFF}");
        if cur.at("<?xml") && cur.peek_at(5).is_some_and(is_space) {
            self.xml_decl(&mut cur)?;
        }
        let mut root = false;
        loop {
            cur.space();
            if cur.eof() {
                break;
            }
            if cur.at("<!--") {
                self.comment(&mut cur, 0)?;
            } else if cur.at("<?") {
                self.pi(&mut cur, Some(0))?;
            } else if cur.at("<!DOCTYPE") {
                if root || self.doc.doctype.is_some() {
                    return Err(err(
                        "misplaced-doctype",
                        cur.span(cur.i, cur.i + 9),
                        "a document has one `<!DOCTYPE>`, before its document element",
                    ));
                }
                self.doctype(&mut cur)?;
            } else if cur.peek() == Some(b'<')
                && !root
                && !matches!(cur.peek_at(1), Some(b'!' | b'/'))
            {
                if !self.start_tag(&mut cur)? {
                    self.content(&mut cur, 0)?;
                }
                root = true;
            } else if root {
                return Err(err(
                    "content-after-root",
                    cur.here(),
                    "only comments, processing instructions and white space can follow the document element",
                ));
            } else {
                return Err(err(
                    "expected-root",
                    cur.here(),
                    "expected the document element",
                ));
            }
        }
        if !root {
            return Err(err(
                "no-root",
                cur.span(src.len(), src.len()),
                "the document has no document element",
            ));
        }
        Ok(())
    }

    fn quoted<'t>(&self, cur: &mut Cur<'t>, what: &str) -> R<(&'t str, usize)> {
        let q = cur.peek();
        if !matches!(q, Some(b'"' | b'\'')) {
            return Err(self.expected(cur, &format!("{what} in quotes")));
        }
        let start = cur.i + 1;
        match cur.s[start..].find(q.unwrap() as char) {
            Some(n) => {
                cur.i = start + n + 1;
                Ok((&cur.s[start..start + n], start))
            }
            None => Err(err(
                "unexpected-eof",
                cur.span(cur.i, cur.s.len()),
                format!("{what} has no closing quote"),
            )),
        }
    }

    fn eq(&self, cur: &mut Cur) -> R<()> {
        cur.space();
        if !cur.eat_byte(b'=') {
            return Err(self.expected(cur, "`=`"));
        }
        cur.space();
        Ok(())
    }

    fn xml_decl(&mut self, cur: &mut Cur) -> R<()> {
        let start = cur.i;
        cur.i += 5;
        self.need_space(cur, "`version`")?;
        if !cur.eat("version") {
            return Err(self.expected(cur, "`version`"));
        }
        self.eq(cur)?;
        let (version, at) = self.quoted(cur, "the version")?;
        let digits = version.strip_prefix("1.").unwrap_or("");
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err(
                "invalid-version",
                cur.span(at, at + version.len()),
                format!("`{version}` isn't an XML 1.x version number"),
            ));
        }
        let mut decl = XmlDecl {
            version: version.to_string(),
            encoding: None,
            standalone: None,
            span: Span::default(),
        };
        let mut sp = cur.space();
        if cur.at("encoding") {
            if !sp {
                return Err(self.expected(cur, "white space"));
            }
            cur.i += 8;
            self.eq(cur)?;
            let (enc, at) = self.quoted(cur, "the encoding")?;
            let b = enc.as_bytes();
            if b.is_empty()
                || !b[0].is_ascii_alphabetic()
                || !b
                    .iter()
                    .all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            {
                return Err(err(
                    "invalid-encoding-name",
                    cur.span(at, at + enc.len()),
                    format!("`{enc}` isn't an encoding name"),
                ));
            }
            decl.encoding = Some(enc.to_string());
            sp = cur.space();
        }
        if cur.at("standalone") {
            if !sp {
                return Err(self.expected(cur, "white space"));
            }
            cur.i += 10;
            self.eq(cur)?;
            let (sd, at) = self.quoted(cur, "standalone")?;
            decl.standalone = Some(match sd {
                "yes" => true,
                "no" => false,
                _ => {
                    return Err(err(
                        "invalid-standalone",
                        cur.span(at, at + sd.len()),
                        "standalone is `yes` or `no`",
                    ));
                }
            });
            self.standalone = sd == "yes";
            cur.space();
        }
        if !cur.eat("?>") {
            return Err(self.expected(cur, "`?>` to end the XML declaration"));
        }
        decl.span = cur.span(start, cur.i);
        self.doc.decl = Some(decl);
        Ok(())
    }

    // Markup that can be anywhere.

    pub fn comment(&mut self, cur: &mut Cur, parent: u32) -> R<()> {
        let start = cur.i;
        cur.i += 4;
        let Some(n) = cur.s[cur.i..].find("--") else {
            return Err(err(
                "unexpected-eof",
                cur.span(start, cur.s.len()),
                "the comment has no `-->`",
            ));
        };
        let end = cur.i + n;
        if cur.peek_at(n + 2) != Some(b'>') {
            return Err(err(
                "double-hyphen-in-comment",
                cur.span(end, end + 2),
                "`--` can't be inside a comment",
            ));
        }
        let text_start = self.doc.text.len();
        let body = cur.i;
        cur.i = end + 3;
        let span = cur.span(start, cur.i);
        if parent == NONE {
            // In the DTD: checked, not kept.
            return self.check_chars(cur, body, end);
        }
        self.push_checked(cur, body, end)?;
        let s = self.str_from(text_start, span)?;
        self.node(parent, span, Data::Comment(s))?;
        Ok(())
    }

    /// A processing instruction, added to `parent`, or only checked.
    pub fn pi(&mut self, cur: &mut Cur, parent: Option<u32>) -> R<()> {
        let start = cur.i;
        cur.i += 2;
        let name_at = cur.i;
        let Some(target) = cur.name() else {
            return Err(self.expected(cur, "a processing instruction's target"));
        };
        let tspan = cur.span(name_at, cur.i);
        if target.eq_ignore_ascii_case("xml") {
            return Err(err(
                "reserved-pi-target",
                tspan,
                if target == "xml" {
                    "the XML declaration can only be at the very start of the document"
                } else {
                    "`xml` in any case is reserved, and can't be a processing instruction's target"
                },
            ));
        }
        self.no_colon(target, tspan, "a processing instruction's target")?;
        let data_start;
        let data_end;
        if cur.eat("?>") {
            data_start = cur.i;
            data_end = cur.i;
        } else {
            if !cur.space() {
                return Err(self.expected(cur, "white space or `?>`"));
            }
            data_start = cur.i;
            let Some(n) = cur.s[cur.i..].find("?>") else {
                return Err(err(
                    "unexpected-eof",
                    cur.span(start, cur.s.len()),
                    "the processing instruction has no `?>`",
                ));
            };
            data_end = cur.i + n;
            cur.i = data_end + 2;
        }
        let span = cur.span(start, cur.i);
        let Some(parent) = parent else {
            return self.check_chars(cur, data_start, data_end);
        };
        let t0 = self.doc.text.len();
        self.doc.text.push_str(target);
        let target = self.str_from(t0, span)?;
        let d0 = self.doc.text.len();
        self.push_checked(cur, data_start, data_end)?;
        let data = self.str_from(d0, span)?;
        self.node(parent, span, Data::Pi { target, data })?;
        Ok(())
    }

    // Elements.

    /// A start tag, at `<`. Says whether it was an empty-element tag.
    fn start_tag(&mut self, cur: &mut Cur) -> R<bool> {
        let lt = cur.i;
        cur.i += 1;
        let Some(qname) = cur.name() else {
            return Err(err(
                "invalid-name",
                cur.here(),
                "`<` has to start a tag; write `&lt;` for a literal one",
            ));
        };
        let name = self.qname(qname, cur.span(lt + 1, cur.i))?;
        self.pending.clear();
        let empty = loop {
            let sp = cur.space();
            match cur.peek() {
                Some(b'>') => {
                    cur.i += 1;
                    break false;
                }
                Some(b'/') if cur.peek_at(1) == Some(b'>') => {
                    cur.i += 2;
                    break true;
                }
                None => {
                    return Err(err(
                        "unexpected-eof",
                        cur.span(lt, cur.s.len()),
                        format!("the start tag of `{qname}` has no `>`"),
                    ));
                }
                _ if !sp => return Err(self.expected(cur, "white space, `>` or `/>`")),
                _ => {}
            }
            let a = cur.i;
            let Some(aname) = cur.name() else {
                return Err(self.expected(cur, "an attribute name, `>` or `/>`"));
            };
            let aid = self.qname(aname, cur.span(a, cur.i))?;
            self.eq(cur)?;
            let value = self.attr_value(cur)?;
            self.pending.push(Pending {
                name: aid,
                value,
                span: cur.span(a, cur.i),
                specified: true,
                ns: 0,
            });
        };
        let tag = cur.span(lt, cur.i);
        if self.opts.limits.too_deep(self.stack.len() as u32 + 1) {
            return Err(err(
                "too-deep",
                tag,
                format!(
                    "elements nest deeper than the limit of {}",
                    self.opts.limits.max_depth.unwrap_or(0)
                ),
            ));
        }
        self.step(self.pending.len() as u64, tag)?;
        self.attributes(name, tag)?;
        let mark = self.scope.undo.len();
        let ns = if self.opts.namespaces {
            self.namespaces(name, tag)?
        } else {
            0
        };
        let attrs = self.doc.attrs.len() as u32;
        for p in &self.pending {
            self.doc.attrs.push(AttrData {
                name: p.name,
                ns: p.ns,
                value: p.value,
                span: p.span,
                specified: p.specified,
            });
        }
        let node = self.node(
            self.parent(),
            tag,
            Data::Element {
                name,
                ns,
                attrs,
                attrs_len: self.pending.len() as u32,
            },
        )?;
        if empty {
            self.scope.pop(mark);
        } else {
            self.stack.push(Open {
                node,
                name,
                mark,
                start: lt,
            });
        }
        Ok(empty)
    }

    /// Checks the start tag's attributes are unique, and brings in what
    /// the DTD says about them.
    fn attributes(&mut self, element: u32, tag: Span) -> R<()> {
        let n = self.pending.len();
        let defs = self.dtd.attlists.contains_key(&element);
        if n < 2 && !defs {
            return Ok(());
        }
        self.sorted.clear();
        self.sorted.extend(
            self.pending
                .iter()
                .enumerate()
                .map(|(i, p)| (p.name, i as u32)),
        );
        self.sorted.sort_unstable();
        for w in self.sorted.windows(2) {
            if w[0].0 == w[1].0 {
                let later = w[0].1.max(w[1].1) as usize;
                let p = &self.pending[later];
                return Err(err(
                    "duplicate-attribute",
                    p.span,
                    format!(
                        "`{}` appears twice in the same start tag",
                        self.doc.names.qname_str(p.name)
                    ),
                ));
            }
        }
        let Some(defs) = self.dtd.attlists.get_mut(&element) else {
            return Ok(());
        };
        for def in defs.iter_mut() {
            match self.sorted.binary_search_by_key(&def.name, |&(id, _)| id) {
                Ok(at) => {
                    if def.tokenized {
                        let p = &mut self.pending[self.sorted[at].1 as usize];
                        let value = &self.doc.text[p.value.start as usize..p.value.end() as usize];
                        let collapsed = collapse(value);
                        if collapsed != value {
                            let start = self.doc.text.len();
                            self.doc.text.push_str(&collapsed);
                            p.value = Str {
                                start: start as u32,
                                len: collapsed.len() as u32,
                            };
                        }
                    }
                }
                Err(_) => {
                    let Some(default) = &def.default else {
                        continue;
                    };
                    let value = match def.cached {
                        Some(s) => s,
                        None => {
                            let start = self.doc.text.len();
                            self.doc.text.push_str(default);
                            let s = Str {
                                start: start as u32,
                                len: default.len() as u32,
                            };
                            def.cached = Some(s);
                            s
                        }
                    };
                    // Stored once, but every element it's applied to hands it
                    // out again, so it counts as an expansion each time: a
                    // big default on many elements is a blowup like an
                    // entity's.
                    self.expansion.spend(u64::from(value.len)).map_err(|_| {
                        err(
                            "entity-expansion-limit",
                            tag,
                            "the DTD's attribute defaults add more text than the limit allows",
                        )
                    })?;
                    self.pending.push(Pending {
                        name: def.name,
                        value,
                        span: tag,
                        specified: false,
                        ns: 0,
                    });
                }
            }
        }
        if self.doc.text.len() > u32::MAX as usize {
            return Err(err("too-large", tag, "the document's text is over 4 GiB"));
        }
        Ok(())
    }

    /// Binds the start tag's namespace declarations and resolves its
    /// names, returning the element's namespace.
    fn namespaces(&mut self, element: u32, tag: Span) -> R<u32> {
        for i in 0..self.pending.len() {
            let p = &self.pending[i];
            let info = self.doc.names.qname_info(p.name);
            let (prefix, span) = (info.prefix, p.span);
            let declared = if &*info.qname == "xmlns" {
                0
            } else if prefix == PREFIX_XMLNS {
                let local = self.doc.names.local(p.name).to_string();
                self.doc.names.prefix(&local)
            } else {
                continue;
            };
            let p = &mut self.pending[i];
            p.ns = NS_XMLNS;
            let value = &self.doc.text[p.value.start as usize..p.value.end() as usize];
            let bad = |m: &str| Err(err("invalid-namespace-declaration", span, m.to_string()));
            if declared == PREFIX_XMLNS {
                return bad("the `xmlns` prefix can't be declared");
            }
            if declared == PREFIX_XML {
                if value != XML_NS {
                    return bad("the `xml` prefix can only be bound to its own namespace");
                }
                continue;
            }
            if value == XML_NS {
                return bad("the XML namespace can only be bound to the `xml` prefix");
            }
            if value == XMLNS_NS {
                return bad("the `xmlns` namespace can't be declared");
            }
            if value.is_empty() && declared != 0 {
                return bad("in Namespaces 1.0 a prefix can't be undeclared with `\"\"`");
            }
            let uri = self.doc.names.uri(value);
            self.scope.bind(declared, uri);
        }
        let resolve = |this: &Self, name: u32, span: Span| -> R<u32> {
            let prefix = this.doc.names.qname_info(name).prefix;
            if prefix == PREFIX_XMLNS {
                return Err(err(
                    "invalid-namespace-declaration",
                    span,
                    "`xmlns` is reserved, and can't be an element's prefix",
                ));
            }
            match this.scope.get(prefix) {
                NONE => Err(err(
                    "undeclared-prefix",
                    span,
                    format!(
                        "the prefix of `{}` isn't bound to a namespace",
                        this.doc.names.qname_str(name)
                    ),
                )),
                uri => Ok(uri),
            }
        };
        let ns = resolve(self, element, tag)?;
        let mut prefixed = 0;
        for i in 0..self.pending.len() {
            let p = &self.pending[i];
            if p.ns == NS_XMLNS || self.doc.names.qname_info(p.name).prefix == 0 {
                continue;
            }
            let uri = resolve(self, p.name, p.span)?;
            self.pending[i].ns = uri;
            prefixed += 1;
        }
        if prefixed > 1 {
            // No two attributes with the same namespace and local name.
            let names = &self.doc.names;
            let mut seen: Vec<(u32, &str, Span)> = self
                .pending
                .iter()
                .filter(|p| p.ns != NS_XMLNS && names.qname_info(p.name).prefix != 0)
                .map(|p| (p.ns, names.local(p.name), p.span))
                .collect();
            seen.sort_unstable_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
            for w in seen.windows(2) {
                if (w[0].0, w[0].1) == (w[1].0, w[1].1) {
                    return Err(err(
                        "duplicate-attribute",
                        w[0].2.max(w[1].2),
                        format!(
                            "two attributes are `{}` in the namespace `{}`",
                            w[0].1,
                            names.uri_str(w[0].0)
                        ),
                    ));
                }
            }
        }
        Ok(ns)
    }

    /// An attribute value, at its opening quote, normalized into the
    /// buffer.
    pub(crate) fn attr_value(&mut self, cur: &mut Cur) -> R<Str> {
        let q = match cur.peek() {
            Some(q @ (b'"' | b'\'')) => q,
            _ => return Err(self.expected(cur, "an attribute value in quotes")),
        };
        let open = cur.i;
        cur.i += 1;
        let start = self.doc.text.len();
        let bytes = cur.bytes();
        let (mut from, mut i) = (cur.i, cur.i);
        loop {
            while i < bytes.len() && !ATTR[bytes[i] as usize] {
                i += 1;
            }
            let Some(&c) = bytes.get(i) else {
                return Err(err(
                    "unexpected-eof",
                    cur.span(open, bytes.len()),
                    "the attribute value has no closing quote",
                ));
            };
            match c {
                _ if c == q => break,
                b'"' | b'\'' => i += 1,
                b'<' => {
                    return Err(err(
                        "lt-in-attribute",
                        cur.span(i, i + 1),
                        "`<` can't be in an attribute value; write `&lt;`",
                    ));
                }
                b'\t' | b'\n' => {
                    self.doc.text.push_str(&cur.s[from..i]);
                    self.doc.text.push(' ');
                    i += 1;
                    from = i;
                }
                b'\r' => {
                    self.doc.text.push_str(&cur.s[from..i]);
                    self.doc.text.push(' ');
                    i += 1;
                    if !cur.raw() && bytes.get(i) == Some(&b'\n') {
                        i += 1;
                    }
                    from = i;
                }
                b'&' => {
                    self.doc.text.push_str(&cur.s[from..i]);
                    cur.i = i;
                    self.reference_in_attr(cur)?;
                    i = cur.i;
                    from = i;
                }
                0xEF if !non_char_at(bytes, i) => i += 1,
                _ => return Err(self.not_char(cur, i)),
            }
        }
        self.doc.text.push_str(&cur.s[from..i]);
        cur.i = i + 1;
        self.str_from(start, cur.span(open, cur.i))
    }

    /// A character reference, at `&#`.
    pub fn char_ref(&self, cur: &mut Cur) -> R<char> {
        let start = cur.i;
        cur.i += 2;
        let hex = cur.eat_byte(b'x');
        let digits = cur.i;
        let mut n: u32 = 0;
        while let Some(b) = cur.peek() {
            let d = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' if hex => b - b'a' + 10,
                b'A'..=b'F' if hex => b - b'A' + 10,
                _ => break,
            };
            n = n
                .saturating_mul(if hex { 16 } else { 10 })
                .saturating_add(d as u32);
            cur.i += 1;
        }
        if cur.i == digits || !cur.eat_byte(b';') {
            return Err(err(
                "invalid-char-ref",
                cur.span(start, cur.i),
                "a character reference is `&#` and decimal digits, or `&#x` and hex digits, then `;`",
            ));
        }
        match char::from_u32(n).filter(|&c| is_char(c)) {
            Some(c) => Ok(c),
            None => Err(err(
                "invalid-char-ref",
                cur.span(start, cur.i),
                "the character reference is to a character XML doesn't allow",
            )),
        }
    }

    /// An entity reference's name, at `&`.
    pub fn entity_ref_name<'t>(&self, cur: &mut Cur<'t>) -> R<(&'t str, Span)> {
        let start = cur.i;
        cur.i += 1;
        let Some(name) = cur.name() else {
            return Err(err(
                "invalid-reference",
                cur.span(start, cur.i),
                "`&` has to start a reference; write `&amp;` for a literal one",
            ));
        };
        if !cur.eat_byte(b';') {
            return Err(err(
                "invalid-reference",
                cur.span(start, cur.i),
                format!("the reference to `{name}` has no `;`"),
            ));
        }
        Ok((name, cur.span(start, cur.i)))
    }

    /// Starts expanding a declared internal entity: checks recursion,
    /// depth and budget, and marks it active. Gives its text.
    pub fn enter(&mut self, name: &str, span: Span, parameter: bool) -> R<Rc<str>> {
        let standalone = self.standalone;
        let depth = self.entity_depth + 1;
        let max = self.opts.max_entity_depth.min(ENTITY_DEPTH_CEILING);
        let table = if parameter {
            &mut self.dtd.params
        } else {
            &mut self.dtd.general
        };
        let e = table.get_mut(name).expect("entered a declared entity");
        if e.active {
            return Err(err(
                "recursive-entity",
                span,
                format!("the entity `{name}` refers to itself"),
            ));
        }
        if standalone && e.in_pe && !parameter {
            return Err(err(
                "undeclared-entity",
                span,
                format!(
                    "`{name}` is declared in a parameter entity, which a standalone document can't use"
                ),
            ));
        }
        if depth > max {
            return Err(err(
                "entity-too-deep",
                span,
                format!("entities nest deeper than the limit of {max}"),
            ));
        }
        let value = e.value.clone().expect("an internal entity");
        e.active = true;
        self.entity_depth = depth;
        self.expansion.spend(value.len() as u64).map_err(|_| {
            err(
                "entity-expansion-limit",
                span,
                "entity references expand to more text than the limit allows",
            )
        })?;
        self.step(1, span)?;
        Ok(value)
    }

    pub fn leave(&mut self, name: &str, parameter: bool) {
        let table = if parameter {
            &mut self.dtd.params
        } else {
            &mut self.dtd.general
        };
        if let Some(e) = table.get_mut(name) {
            e.active = false;
        }
        self.entity_depth -= 1;
    }

    /// Reports a reference to an entity that wasn't read, once per name.
    fn not_loaded(&mut self, name: &str, span: Span, declared: bool) {
        let first = match self.dtd.general.get_mut(name) {
            Some(e) if declared => !std::mem::replace(&mut e.warned, true),
            _ => self.warned.insert(name.to_string()),
        };
        if first {
            let message = if declared {
                format!("`{name}` is an external entity, which Parsec doesn't read")
            } else {
                format!("`{name}` isn't declared in the part of the DTD Parsec read")
            };
            self.warn(Diagnostic::warning("entity-not-loaded", span, message));
        }
    }

    /// A reference inside an attribute value, at `&`, its replacement
    /// appended to the buffer.
    fn reference_in_attr(&mut self, cur: &mut Cur) -> R<()> {
        if cur.at("&#") {
            let c = self.char_ref(cur)?;
            self.doc.text.push(c);
            return Ok(());
        }
        let (name, span) = self.entity_ref_name(cur)?;
        if let Some(c) = predefined(name) {
            self.doc.text.push(c);
            return Ok(());
        }
        let span = cur.origin.unwrap_or(span);
        match self.dtd.general.get(name) {
            None if self.lenient() => {
                self.not_loaded(name, span, false);
                return Ok(());
            }
            None => {
                return Err(err(
                    "undeclared-entity",
                    span,
                    format!("the entity `{name}` isn't declared"),
                ));
            }
            Some(e) if e.value.is_none() => {
                return Err(err(
                    "external-entity-in-attribute",
                    span,
                    format!(
                        "`{name}` is an external entity, which an attribute value can't refer to"
                    ),
                ));
            }
            Some(_) => {}
        }
        let text = self.enter(name, span, false)?;
        let mut sub = Cur::new(&text, Some(span));
        let bytes = sub.bytes();
        let mut from = 0;
        while sub.i < bytes.len() {
            match bytes[sub.i] {
                b'<' => {
                    return Err(err(
                        "lt-in-attribute",
                        span,
                        format!("`{name}` holds a `<`, so it can't be used in an attribute value"),
                    ));
                }
                b'\t' | b'\n' | b'\r' => {
                    self.doc.text.push_str(&text[from..sub.i]);
                    self.doc.text.push(' ');
                    sub.i += 1;
                    from = sub.i;
                }
                b'&' => {
                    self.doc.text.push_str(&text[from..sub.i]);
                    self.reference_in_attr(&mut sub)?;
                    from = sub.i;
                }
                _ => sub.i += 1,
            }
        }
        self.doc.text.push_str(&text[from..]);
        self.leave(name, false);
        Ok(())
    }

    /// Adds text written to the buffer from `start` to the open element,
    /// joining it to text just before.
    fn add_text(&mut self, start: usize, span: Span) -> R<()> {
        if self.doc.text.len() == start {
            return Ok(());
        }
        let s = self.str_from(start, span)?;
        let parent = self.parent();
        let last = self.doc.nodes[parent as usize].last;
        if last != NONE {
            let node = &mut self.doc.nodes[last as usize];
            if let Data::Text(t) = &mut node.data
                && t.end() == s.start
            {
                t.len += s.len;
                node.span = node.span.to(span);
                return Ok(());
            }
        }
        self.node(parent, span, Data::Text(s))?;
        Ok(())
    }

    /// Content, until the element open at `base` closes (in the document)
    /// or the replacement text ends (in an entity), which has to leave the
    /// same elements open as it found.
    fn content(&mut self, cur: &mut Cur, base: usize) -> R<()> {
        loop {
            self.char_data(cur)?;
            let Some(c) = cur.peek() else {
                if cur.raw() {
                    if self.stack.len() != base {
                        let open = self.stack.last().unwrap();
                        return Err(err(
                            "unbalanced-entity",
                            cur.span(0, 0),
                            format!(
                                "the entity opens `{}` and doesn't close it",
                                self.doc.names.qname_str(open.name)
                            ),
                        ));
                    }
                    return Ok(());
                }
                let open = self.stack.last().unwrap();
                return Err(err(
                    "unexpected-eof",
                    cur.span(open.start, open.start + 1),
                    format!(
                        "the document ends before `{}` is closed",
                        self.doc.names.qname_str(open.name)
                    ),
                ));
            };
            if c == b'&' {
                self.reference(cur)?;
                continue;
            }
            // `<`
            match cur.peek_at(1) {
                Some(b'/') => {
                    if self.stack.len() == base {
                        return Err(err(
                            "unbalanced-entity",
                            cur.span(cur.i, cur.i + 2),
                            "the entity closes an element it didn't open",
                        ));
                    }
                    self.end_tag(cur)?;
                    if self.stack.len() == base && !cur.raw() {
                        return Ok(());
                    }
                }
                Some(b'!') if cur.at("<!--") => self.comment(cur, self.parent())?,
                Some(b'!') if cur.at("<![CDATA[") => self.cdata(cur)?,
                Some(b'?') => self.pi(cur, Some(self.parent()))?,
                Some(b'!') => {
                    return Err(err(
                        "invalid-markup",
                        cur.span(cur.i, cur.i + 2),
                        "only a comment or a CDATA section can start with `<!` in content",
                    ));
                }
                _ => {
                    self.start_tag(cur)?;
                }
            }
        }
    }

    /// Character data up to the next `<`, `&` or the end.
    fn char_data(&mut self, cur: &mut Cur) -> R<()> {
        let bytes = cur.bytes();
        let start = cur.i;
        let buf = self.doc.text.len();
        let (mut from, mut i) = (start, start);
        loop {
            while i < bytes.len() && !TEXT[bytes[i] as usize] {
                i += 1;
            }
            let Some(&c) = bytes.get(i) else { break };
            match c {
                b'<' | b'&' => break,
                b']' => {
                    if bytes[i..].starts_with(b"]]>") {
                        return Err(err(
                            "cdata-end-in-text",
                            cur.span(i, i + 3),
                            "`]]>` can't be in character data; write `]]&gt;`",
                        ));
                    }
                    i += 1;
                }
                b'\r' if !cur.raw() => {
                    self.doc.text.push_str(&cur.s[from..i]);
                    self.doc.text.push('\n');
                    i += 1;
                    if bytes.get(i) == Some(&b'\n') {
                        i += 1;
                    }
                    from = i;
                }
                b'\r' => i += 1,
                0xEF if !non_char_at(bytes, i) => i += 1,
                _ => return Err(self.not_char(cur, i)),
            }
        }
        self.doc.text.push_str(&cur.s[from..i]);
        cur.i = i;
        self.add_text(buf, cur.span(start, i))
    }

    fn cdata(&mut self, cur: &mut Cur) -> R<()> {
        let start = cur.i;
        cur.i += 9;
        let Some(n) = cur.s[cur.i..].find("]]>") else {
            return Err(err(
                "unexpected-eof",
                cur.span(start, cur.s.len()),
                "the CDATA section has no `]]>`",
            ));
        };
        let body = cur.i;
        cur.i += n + 3;
        let span = cur.span(start, cur.i);
        let t0 = self.doc.text.len();
        self.push_checked(cur, body, body + n)?;
        let s = self.str_from(t0, span)?;
        self.node(self.parent(), span, Data::Cdata(s))?;
        Ok(())
    }

    fn end_tag(&mut self, cur: &mut Cur) -> R<()> {
        let start = cur.i;
        cur.i += 2;
        let name = cur.name().unwrap_or("");
        cur.space();
        if !cur.eat_byte(b'>') {
            return Err(self.expected(cur, "`>` to end the end tag"));
        }
        let open = self.stack.pop().unwrap();
        let want = self.doc.names.qname_str(open.name);
        if name != want {
            return Err(err(
                "mismatched-end-tag",
                cur.span(start, cur.i),
                format!("`</{name}>` closes `<{want}>`"),
            )
            .with_label(
                cur.span(open.start, open.start + 1 + want.len()),
                "opened here",
            ));
        }
        self.scope.pop(open.mark);
        if !cur.raw() {
            self.doc.nodes[open.node as usize].span.end = cur.i as u32;
        }
        Ok(())
    }

    /// A reference in content, at `&`.
    fn reference(&mut self, cur: &mut Cur) -> R<()> {
        let start = cur.i;
        if cur.at("&#") {
            let c = self.char_ref(cur)?;
            let buf = self.doc.text.len();
            self.doc.text.push(c);
            return self.add_text(buf, cur.span(start, cur.i));
        }
        let (name, span) = self.entity_ref_name(cur)?;
        if let Some(c) = predefined(name) {
            let buf = self.doc.text.len();
            self.doc.text.push(c);
            return self.add_text(buf, span);
        }
        let span = cur.origin.unwrap_or(span);
        match self.dtd.general.get(name) {
            None if self.lenient() => {
                self.not_loaded(name, span, false);
                return self.entity_ref_node(name, span);
            }
            None => {
                return Err(err(
                    "undeclared-entity",
                    span,
                    format!("the entity `{name}` isn't declared"),
                ));
            }
            Some(e) if e.unparsed => {
                return Err(err(
                    "unparsed-entity-reference",
                    span,
                    format!(
                        "`{name}` is an unparsed entity, which only an ENTITY attribute can name"
                    ),
                ));
            }
            Some(e) if e.value.is_none() => {
                self.not_loaded(name, span, true);
                return self.entity_ref_node(name, span);
            }
            Some(_) => {}
        }
        let text = self.enter(name, span, false)?;
        let mut sub = Cur::new(&text, Some(span));
        let base = self.stack.len();
        self.content(&mut sub, base)?;
        self.leave(name, false);
        Ok(())
    }

    fn entity_ref_node(&mut self, name: &str, span: Span) -> R<()> {
        let t0 = self.doc.text.len();
        self.doc.text.push_str(name);
        let s = self.str_from(t0, span)?;
        self.node(self.parent(), span, Data::EntityRef(s))?;
        Ok(())
    }

    pub fn doctype_node(&mut self, span: Span) -> R<()> {
        self.node(0, span, Data::Doctype)?;
        Ok(())
    }
}

/// The five entities every XML document has.
pub(crate) fn predefined(name: &str) -> Option<char> {
    Some(match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "apos" => '\'',
        "quot" => '"',
        _ => return None,
    })
}

/// A tokenized attribute's value: spaces trimmed, and each run one space.
pub(crate) fn collapse(value: &str) -> String {
    value
        .split(' ')
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
