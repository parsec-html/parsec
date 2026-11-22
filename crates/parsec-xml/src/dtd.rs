// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The `<!DOCTYPE>` and its internal subset, read as XML 1.0 asks of a
//! processor that doesn't validate: entity declarations, general and
//! parameter, and attribute-list declarations are processed; element and
//! notation declarations are checked; the external subset and external
//! entities are never read, and saying so is a warning, not an error.

use std::rc::Rc;

use parsec_base::{Diagnostic, Span};

use crate::chars::{is_pubid, nmtoken_len};
use crate::parse::{AttDef, Cur, Entity, Parser, R, collapse, err};
use crate::tree::{Doctype, EntityDecl, Notation};

/// Where a run of declarations ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    /// The internal subset's `]`.
    Bracket,
    /// The end of a parameter entity's replacement text.
    Text,
    /// A conditional section's `]]>`.
    Section,
}

impl Parser<'_, '_> {
    pub(crate) fn doctype(&mut self, cur: &mut Cur) -> R<()> {
        let start = cur.i;
        cur.i += 9;
        self.in_dtd = true;
        self.need_space(cur, "the document element's name")?;
        let Some(name) = cur.name() else {
            return Err(self.expected(cur, "the document element's name"));
        };
        let mut doctype = Doctype {
            name: name.to_string(),
            public_id: None,
            system_id: None,
            internal_subset: None,
            span: Span::default(),
        };
        let mut sp = cur.space();
        if cur.at("SYSTEM") || cur.at("PUBLIC") {
            if !sp {
                return Err(self.expected(cur, "white space"));
            }
            let at = cur.i;
            (doctype.public_id, doctype.system_id) = self.external_id(cur, false)?;
            self.has_external_subset = true;
            self.warn(Diagnostic::warning(
                "external-dtd-not-loaded",
                cur.span(at, cur.i),
                "the external subset isn't read: Parsec never fetches or opens a DTD",
            ));
            sp = cur.space();
        }
        let _ = sp;
        if cur.eat_byte(b'[') {
            let subset = cur.i;
            self.decls(cur, End::Bracket)?;
            doctype.internal_subset = Some(cur.s[subset..cur.i - 1].to_string());
            cur.space();
        }
        if !cur.eat_byte(b'>') {
            return Err(self.expected(cur, "`>` to end the `<!DOCTYPE>`"));
        }
        doctype.span = cur.span(start, cur.i);
        self.doctype_node(doctype.span)?;
        self.doc.doctype = Some(doctype);
        self.in_dtd = false;
        Ok(())
    }

    fn decls(&mut self, cur: &mut Cur, end: End) -> R<()> {
        loop {
            cur.space();
            let Some(c) = cur.peek() else {
                if end == End::Text {
                    return Ok(());
                }
                return Err(err(
                    "unexpected-eof",
                    cur.here(),
                    if end == End::Bracket {
                        "the internal subset has no `]`"
                    } else {
                        "the conditional section has no `]]>`"
                    },
                ));
            };
            match c {
                b']' if end == End::Bracket => {
                    cur.i += 1;
                    return Ok(());
                }
                b']' if end == End::Section && cur.at("]]>") => {
                    cur.i += 3;
                    return Ok(());
                }
                b'%' => self.pe_reference(cur)?,
                b'<' if cur.at("<!ENTITY") => self.entity_decl(cur)?,
                b'<' if cur.at("<!ATTLIST") => self.attlist_decl(cur)?,
                b'<' if cur.at("<!ELEMENT") => self.element_decl(cur)?,
                b'<' if cur.at("<!NOTATION") => self.notation_decl(cur)?,
                b'<' if cur.at("<!--") => self.comment(cur, crate::tree::NONE)?,
                b'<' if cur.at("<?") => self.pi(cur, None)?,
                // Only a parameter entity's text, in place of external
                // markup, can hold a conditional section.
                b'<' if cur.at("<![") && cur.origin.is_some() => self.conditional(cur)?,
                _ => return Err(self.expected(cur, "a markup declaration")),
            }
        }
    }

    /// A parameter entity reference between declarations, at `%`.
    fn pe_reference(&mut self, cur: &mut Cur) -> R<()> {
        let start = cur.i;
        cur.i += 1;
        let Some(name) = cur.name() else {
            return Err(self.expected(cur, "a parameter entity's name after `%`"));
        };
        if !cur.eat_byte(b';') {
            return Err(self.expected(cur, "`;`"));
        }
        let span = cur.origin.unwrap_or(Span::new(start as u32, cur.i as u32));
        self.has_pe_refs = true;
        let internal = match self.dtd.params.get(name) {
            None if self.standalone => {
                return Err(err(
                    "undeclared-entity",
                    span,
                    format!("the parameter entity `{name}` isn't declared"),
                ));
            }
            None => false,
            Some(e) => e.value.is_some(),
        };
        if !internal {
            self.warn(Diagnostic::warning(
                "entity-not-loaded",
                span,
                format!("the parameter entity `{name}` isn't read"),
            ));
            // It may have declared anything, so what follows can't be
            // relied on (XML 1.0, 5.1).
            if !self.standalone {
                self.dtd.skip = true;
            }
            return Ok(());
        }
        let text = self.enter(name, span, true)?;
        let mut sub = Cur::new(&text, Some(span));
        self.decls(&mut sub, End::Text)?;
        self.leave(name, true);
        Ok(())
    }

    fn skipped(&mut self, span: Span) {
        if !self.dtd.skip_warned {
            self.dtd.skip_warned = true;
            self.warn(Diagnostic::warning(
                "declarations-skipped",
                span,
                "entity and attribute-list declarations after a parameter entity that wasn't read are skipped",
            ));
        }
    }

    fn entity_decl(&mut self, cur: &mut Cur) -> R<()> {
        let start = cur.i;
        cur.i += 8;
        self.need_space(cur, "white space")?;
        let parameter = cur.eat_byte(b'%');
        if parameter {
            self.need_space(cur, "white space")?;
        }
        let name_at = cur.i;
        let Some(name) = cur.name() else {
            return Err(self.expected(cur, "the entity's name"));
        };
        self.no_colon(name, cur.span(name_at, cur.i), "an entity's name")?;
        self.need_space(cur, "the entity's value or external ID")?;
        let mut decl = EntityDecl {
            name: name.to_string(),
            parameter,
            value: None,
            public_id: None,
            system_id: None,
            notation: None,
            span: Span::default(),
        };
        if matches!(cur.peek(), Some(b'"' | b'\'')) {
            decl.value = Some(self.entity_value(cur)?);
        } else {
            (decl.public_id, decl.system_id) = self.external_id(cur, false)?;
            let sp = cur.space();
            if !parameter && cur.at("NDATA") {
                if !sp {
                    return Err(self.expected(cur, "white space"));
                }
                cur.i += 5;
                self.need_space(cur, "the notation's name")?;
                let at = cur.i;
                let Some(n) = cur.name() else {
                    return Err(self.expected(cur, "the notation's name"));
                };
                self.no_colon(n, cur.span(at, cur.i), "a notation's name")?;
                decl.notation = Some(n.to_string());
            }
        }
        cur.space();
        if !cur.eat_byte(b'>') {
            return Err(self.expected(cur, "`>` to end the entity declaration"));
        }
        decl.span = cur.span(start, cur.i);
        if self.dtd.skip {
            self.skipped(decl.span);
            return Ok(());
        }
        let table = if parameter {
            &mut self.dtd.params
        } else {
            &mut self.dtd.general
        };
        // The first declaration of a name is the one that counts.
        if table.contains_key(name) {
            return Ok(());
        }
        table.insert(
            name.to_string(),
            Entity {
                value: decl.value.as_deref().map(Rc::from),
                unparsed: decl.notation.is_some(),
                in_pe: cur.origin.is_some(),
                active: false,
                warned: false,
            },
        );
        self.doc.entities.push(decl);
        Ok(())
    }

    /// An entity's literal value, at its quote, as its replacement text:
    /// character references replaced, entity references kept.
    fn entity_value(&mut self, cur: &mut Cur) -> R<String> {
        let q = cur.peek().unwrap();
        let open = cur.i;
        cur.i += 1;
        let mut out = String::new();
        let mut from = cur.i;
        loop {
            let Some(c) = cur.peek() else {
                return Err(err(
                    "unexpected-eof",
                    cur.span(open, cur.s.len()),
                    "the entity value has no closing quote",
                ));
            };
            match c {
                _ if c == q => break,
                b'%' => {
                    return Err(err(
                        "pe-in-markup-declaration",
                        cur.here(),
                        "a parameter entity reference can't be in an entity value in the internal subset",
                    ));
                }
                b'&' => {
                    self.check_into(cur, from, &mut out)?;
                    if cur.at("&#") {
                        out.push(self.char_ref(cur)?);
                    } else {
                        let at = cur.i;
                        self.entity_ref_name(cur)?;
                        out.push_str(&cur.s[at..cur.i]);
                    }
                    from = cur.i;
                }
                _ => cur.i += 1,
            }
        }
        self.check_into(cur, from, &mut out)?;
        cur.i += 1;
        Ok(out)
    }

    /// Checks and copies `cur.s[from..cur.i]` to `out`, normalizing line
    /// ends.
    fn check_into(&mut self, cur: &Cur, from: usize, out: &mut String) -> R<()> {
        let start = self.doc.text.len();
        self.push_checked(cur, from, cur.i)?;
        out.push_str(&self.doc.text[start..]);
        self.doc.text.truncate(start);
        Ok(())
    }

    /// `SYSTEM "..."` or `PUBLIC "..." "..."`; for a notation, the system
    /// literal after a public one is optional.
    fn external_id(
        &mut self,
        cur: &mut Cur,
        notation: bool,
    ) -> R<(Option<String>, Option<String>)> {
        if cur.eat("SYSTEM") {
            self.need_space(cur, "a system literal")?;
            return Ok((None, Some(self.literal(cur, false)?)));
        }
        if !cur.eat("PUBLIC") {
            return Err(self.expected(cur, "`SYSTEM` or `PUBLIC`"));
        }
        self.need_space(cur, "a public ID")?;
        let public = self.literal(cur, true)?;
        let before = cur.i;
        let sp = cur.space();
        if matches!(cur.peek(), Some(b'"' | b'\'')) {
            if !sp {
                return Err(self.expected(cur, "white space"));
            }
            return Ok((Some(public), Some(self.literal(cur, false)?)));
        }
        if !notation {
            return Err(self.expected(cur, "a system literal"));
        }
        cur.i = before;
        Ok((Some(public), None))
    }

    fn literal(&mut self, cur: &mut Cur, public: bool) -> R<String> {
        let q = match cur.peek() {
            Some(q @ (b'"' | b'\'')) => q,
            _ => return Err(self.expected(cur, "a quoted literal")),
        };
        let open = cur.i;
        let Some(n) = cur.s[open + 1..].find(q as char) else {
            return Err(err(
                "unexpected-eof",
                cur.span(open, cur.s.len()),
                "the literal has no closing quote",
            ));
        };
        let (a, b) = (open + 1, open + 1 + n);
        cur.i = b + 1;
        if public {
            if let Some((at, c)) = cur.s[a..b].char_indices().find(|&(_, c)| !is_pubid(c)) {
                return Err(err(
                    "invalid-public-id",
                    cur.span(a + at, a + at + c.len_utf8()),
                    format!("`{}` can't be in a public ID", c.escape_debug()),
                ));
            }
            return Ok(cur.s[a..b].to_string());
        }
        self.check_chars(cur, a, b)?;
        Ok(cur.s[a..b].to_string())
    }

    fn attlist_decl(&mut self, cur: &mut Cur) -> R<()> {
        let start = cur.i;
        cur.i += 9;
        self.need_space(cur, "an element name")?;
        let at = cur.i;
        let Some(element) = cur.name() else {
            return Err(self.expected(cur, "an element name"));
        };
        let element = self.qname(element, cur.span(at, cur.i))?;
        let mut defs = Vec::new();
        loop {
            let sp = cur.space();
            if cur.eat_byte(b'>') {
                break;
            }
            if !sp {
                return Err(self.expected(cur, "white space or `>`"));
            }
            let at = cur.i;
            let Some(name) = cur.name() else {
                return Err(self.expected(cur, "an attribute name or `>`"));
            };
            let name = self.qname(name, cur.span(at, cur.i))?;
            self.need_space(cur, "an attribute type")?;
            let mut id = false;
            let tokenized = if cur.eat("CDATA") {
                false
            } else if let Some(k) = [
                "IDREFS", "IDREF", "ID", "ENTITIES", "ENTITY", "NMTOKENS", "NMTOKEN",
            ]
            .into_iter()
            .find(|k| cur.eat(k))
            {
                id = k == "ID";
                true
            } else if cur.eat("NOTATION") {
                self.need_space(cur, "`(`")?;
                self.enumeration(cur, false)?;
                true
            } else if cur.peek() == Some(b'(') {
                self.enumeration(cur, true)?;
                true
            } else {
                return Err(self.expected(cur, "an attribute type"));
            };
            self.need_space(cur, "the attribute's default")?;
            let default = if cur.eat("#REQUIRED") || cur.eat("#IMPLIED") {
                None
            } else {
                if cur.eat("#FIXED") {
                    self.need_space(cur, "the fixed value")?;
                }
                let s = self.attr_value(cur)?;
                let value = self.doc.text[s.start as usize..].to_string();
                self.doc.text.truncate(s.start as usize);
                Some(if tokenized { collapse(&value) } else { value })
            };
            defs.push(AttDef {
                name,
                tokenized,
                id,
                default,
                cached: None,
            });
        }
        let span = cur.span(start, cur.i);
        if self.dtd.skip {
            self.skipped(span);
            return Ok(());
        }
        let list = self.dtd.attlists.entry(element).or_default();
        for def in defs {
            // The first declaration of an attribute is the one that counts.
            if !list.iter().any(|d| d.name == def.name) {
                if def.id {
                    // An element type has one ID attribute; the first counts.
                    self.doc.id_attrs.entry(element).or_insert(def.name);
                }
                list.push(def);
            }
        }
        Ok(())
    }

    /// `(a | b | c)` of names, or of name tokens, at `(`.
    fn enumeration(&mut self, cur: &mut Cur, nmtokens: bool) -> R<()> {
        if !cur.eat_byte(b'(') {
            return Err(self.expected(cur, "`(`"));
        }
        loop {
            cur.space();
            let at = cur.i;
            if nmtokens {
                let n = nmtoken_len(&cur.s[cur.i..]);
                if n == 0 {
                    return Err(self.expected(cur, "a name token"));
                }
                cur.i += n;
            } else {
                let Some(n) = cur.name() else {
                    return Err(self.expected(cur, "a notation name"));
                };
                self.no_colon(n, cur.span(at, cur.i), "a notation's name")?;
            }
            cur.space();
            if cur.eat_byte(b')') {
                return Ok(());
            }
            if !cur.eat_byte(b'|') {
                return Err(self.expected(cur, "`|` or `)`"));
            }
        }
    }

    fn element_decl(&mut self, cur: &mut Cur) -> R<()> {
        cur.i += 9;
        self.need_space(cur, "an element name")?;
        let at = cur.i;
        let Some(name) = cur.name() else {
            return Err(self.expected(cur, "an element name"));
        };
        self.qname(name, cur.span(at, cur.i))?;
        self.need_space(cur, "a content model")?;
        if !(cur.eat("EMPTY") || cur.eat("ANY")) {
            if cur.peek() != Some(b'(') {
                return Err(self.expected(cur, "`EMPTY`, `ANY` or `(`"));
            }
            self.content_model(cur)?;
        }
        cur.space();
        if !cur.eat_byte(b'>') {
            return Err(self.expected(cur, "`>` to end the element declaration"));
        }
        Ok(())
    }

    /// A content model, at its `(`: mixed content, or children's, whose
    /// groups nest on a stack rather than by recursion.
    fn content_model(&mut self, cur: &mut Cur) -> R<()> {
        cur.i += 1;
        cur.space();
        if cur.eat("#PCDATA") {
            let mut names = false;
            loop {
                cur.space();
                if cur.eat_byte(b'|') {
                    cur.space();
                    let at = cur.i;
                    let Some(n) = cur.name() else {
                        return Err(self.expected(cur, "an element name"));
                    };
                    self.qname(n, cur.span(at, cur.i))?;
                    names = true;
                } else if cur.eat(")*") || !names && cur.eat_byte(b')') {
                    return Ok(());
                } else {
                    return Err(
                        self.expected(cur, if names { "`|` or `)*`" } else { "`|` or `)`" })
                    );
                }
            }
        }
        // For each open group, its separator once it has one.
        let mut groups: Vec<u8> = vec![0];
        loop {
            // A content particle.
            cur.space();
            if cur.eat_byte(b'(') {
                groups.push(0);
                continue;
            }
            let at = cur.i;
            let Some(n) = cur.name() else {
                return Err(self.expected(cur, "an element name or `(`"));
            };
            self.qname(n, cur.span(at, cur.i))?;
            if matches!(cur.peek(), Some(b'?' | b'*' | b'+')) {
                cur.i += 1;
            }
            // What follows it: a separator, or the end of groups.
            loop {
                cur.space();
                match cur.peek() {
                    Some(c @ (b'|' | b',')) => {
                        let sep = groups.last_mut().unwrap();
                        if *sep != 0 && *sep != c {
                            return Err(err(
                                "invalid-content-model",
                                cur.here(),
                                "a group can't mix `|` and `,`",
                            ));
                        }
                        *sep = c;
                        cur.i += 1;
                        break;
                    }
                    Some(b')') => {
                        cur.i += 1;
                        groups.pop();
                        if matches!(cur.peek(), Some(b'?' | b'*' | b'+')) {
                            cur.i += 1;
                        }
                        if groups.is_empty() {
                            return Ok(());
                        }
                    }
                    _ => return Err(self.expected(cur, "`|`, `,` or `)`")),
                }
            }
        }
    }

    fn notation_decl(&mut self, cur: &mut Cur) -> R<()> {
        let start = cur.i;
        cur.i += 10;
        self.need_space(cur, "the notation's name")?;
        let at = cur.i;
        let Some(name) = cur.name() else {
            return Err(self.expected(cur, "the notation's name"));
        };
        self.no_colon(name, cur.span(at, cur.i), "a notation's name")?;
        self.need_space(cur, "`SYSTEM` or `PUBLIC`")?;
        let (public_id, system_id) = self.external_id(cur, true)?;
        cur.space();
        if !cur.eat_byte(b'>') {
            return Err(self.expected(cur, "`>` to end the notation declaration"));
        }
        if !self.doc.notations.iter().any(|n| n.name == name) {
            self.doc.notations.push(Notation {
                name: name.to_string(),
                public_id,
                system_id,
                span: cur.span(start, cur.i),
            });
        }
        Ok(())
    }

    /// `<![INCLUDE[ ... ]]>` or `<![IGNORE[ ... ]]>`, at `<![`.
    fn conditional(&mut self, cur: &mut Cur) -> R<()> {
        cur.i += 3;
        cur.space();
        let include = if cur.eat("INCLUDE") {
            true
        } else if cur.eat("IGNORE") {
            false
        } else {
            return Err(self.expected(cur, "`INCLUDE` or `IGNORE`"));
        };
        cur.space();
        if !cur.eat_byte(b'[') {
            return Err(self.expected(cur, "`[`"));
        }
        if include {
            return self.decls(cur, End::Section);
        }
        // Ignored sections nest, and nothing else in them counts.
        let mut depth = 1;
        while depth > 0 {
            if cur.eof() {
                return Err(err(
                    "unexpected-eof",
                    cur.here(),
                    "the conditional section has no `]]>`",
                ));
            }
            if cur.eat("<![") {
                depth += 1;
            } else if cur.eat("]]>") {
                depth -= 1;
            } else {
                let c = cur.s[cur.i..].chars().next().unwrap();
                if !crate::chars::is_char(c) {
                    return Err(self.not_char(cur, cur.i));
                }
                cur.i += c.len_utf8();
            }
        }
        Ok(())
    }
}
