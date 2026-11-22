// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! What a parse gives: the tree, names and namespaces, entities and the
//! DTD, spans, errors, bytes, and the serializer.

use parsec_xml::{NodeKind, Options, Span, XML_NS, XMLNS_NS, parse, parse_bytes, parse_with};

fn code(src: &str) -> &'static str {
    match parse(src) {
        Ok(_) => panic!("{src:?} parsed"),
        Err(e) => e.code(),
    }
}

#[test]
fn a_tree_in_document_order() {
    let doc =
        parse("<?xml version='1.0'?><!--c--><a x='1'><b>t</b><?p d?><![CDATA[<x>]]></a>").unwrap();
    let kinds: Vec<_> = doc.descendants(doc.root()).map(|n| doc.kind(n)).collect();
    assert_eq!(kinds.len(), 7);
    assert_eq!(kinds[1], NodeKind::Comment("c"));
    assert!(matches!(kinds[2], NodeKind::Element(n) if n.qname == "a"));
    assert!(matches!(kinds[3], NodeKind::Element(n) if n.qname == "b"));
    assert_eq!(kinds[4], NodeKind::Text("t"));
    assert_eq!(
        kinds[5],
        NodeKind::ProcessingInstruction {
            target: "p",
            data: "d"
        }
    );
    assert_eq!(kinds[6], NodeKind::CData("<x>"));
    let a = doc.root_element();
    assert_eq!(doc.attribute(a, "", "x"), Some("1"));
    assert_eq!(doc.text(a), "t<x>");
    assert_eq!(doc.xml_decl().unwrap().version, "1.0");
    let b = doc.first_child(a).unwrap();
    assert_eq!(doc.parent(b), Some(a));
    assert_eq!(
        doc.next_sibling(b).map(|n| doc.prev_sibling(n)),
        Some(Some(b))
    );
    assert_eq!(doc.descendants(b).count(), 2);
    assert_eq!(doc.ancestors(b).collect::<Vec<_>>(), [a, doc.root()]);
}

#[test]
fn names_resolve_to_namespaces() {
    let src = r#"<r xmlns="urn:d" xmlns:p="urn:p" a="1" p:b="2"><p:c xmlns:p="urn:q"/><d xmlns=""/><p:e/></r>"#;
    let doc = parse(src).unwrap();
    let r = doc.root_element();
    let n = doc.name(r).unwrap();
    assert_eq!((n.prefix, n.local, n.namespace), ("", "r", "urn:d"));
    let attrs: Vec<_> = doc.attributes(r).collect();
    assert_eq!(attrs.len(), 4);
    assert!(attrs[0].is_namespace_declaration() && attrs[0].name.namespace == XMLNS_NS);
    // Unprefixed attributes are in no namespace, whatever the default.
    assert_eq!(attrs[2].name.namespace, "");
    assert_eq!(
        (attrs[3].name.local, attrs[3].name.namespace),
        ("b", "urn:p")
    );
    let names: Vec<_> = doc
        .children(r)
        .map(|c| doc.name(c).unwrap().namespace)
        .collect();
    assert_eq!(names, ["urn:q", "", "urn:p"]);
    assert_eq!(doc.lookup_namespace(r, "p"), Some("urn:p"));
    let c = doc.first_child(r).unwrap();
    assert_eq!(doc.lookup_namespace(c, "p"), Some("urn:q"));
    assert_eq!(doc.lookup_namespace(c, ""), Some("urn:d"));
    assert_eq!(
        doc.lookup_namespace(doc.last_child(r).unwrap(), "xml"),
        Some(XML_NS)
    );
    let d = doc.next_sibling(c).unwrap();
    assert_eq!(doc.lookup_namespace(d, ""), None);
    assert_eq!(doc.attribute_named(r, "p:b"), Some("2"));
    assert_eq!(doc.attribute(r, "urn:p", "b"), Some("2"));
}

#[test]
fn namespace_errors() {
    assert_eq!(code("<p:a/>"), "undeclared-prefix");
    assert_eq!(code("<a p:b='1'/>"), "undeclared-prefix");
    assert_eq!(
        code("<a xmlns:p='u' xmlns:q='u' p:x='1' q:x='2'/>"),
        "duplicate-attribute"
    );
    assert_eq!(code("<a xmlns:p=''/>"), "invalid-namespace-declaration");
    assert_eq!(
        code("<a xmlns:xml='urn:x'/>"),
        "invalid-namespace-declaration"
    );
    assert_eq!(
        code("<a xmlns:x='http://www.w3.org/XML/1998/namespace'/>"),
        "invalid-namespace-declaration"
    );
    assert_eq!(
        code("<a xmlns:xmlns='urn:x'/>"),
        "invalid-namespace-declaration"
    );
    assert_eq!(
        code("<a xmlns='http://www.w3.org/2000/xmlns/'/>"),
        "invalid-namespace-declaration"
    );
    assert_eq!(
        code("<xmlns:a xmlns:xmlns='u'/>"),
        "invalid-namespace-declaration"
    );
    assert_eq!(code("<a:b:c xmlns:a='u'/>"), "invalid-qname");
    assert_eq!(code("<:a/>"), "invalid-qname");
    assert_eq!(code("<a: xmlns:a='u'/>"), "invalid-qname");
    assert_eq!(code("<a><?p:q x?></a>"), "colon-in-name");
    assert_eq!(
        code("<!DOCTYPE a [<!ENTITY a:b 'x'>]><a/>"),
        "colon-in-name"
    );
    // The xml prefix needs no declaration, and may be declared as itself.
    assert!(parse("<a xml:lang='en' xmlns:xml='http://www.w3.org/XML/1998/namespace'/>").is_ok());
    // A prefix's scope ends with its element.
    assert_eq!(code("<a><b xmlns:p='u'/><p:c/></a>"), "undeclared-prefix");
    // With namespaces off, any name goes and none has a namespace.
    let off = Options {
        namespaces: false,
        ..Options::default()
    };
    let doc = parse_with("<a:b:c p:x='1' xmlns:p=''/>", &off).unwrap();
    let n = doc.name(doc.root_element()).unwrap();
    assert_eq!(
        (n.qname, n.prefix, n.local, n.namespace),
        ("a:b:c", "", "a:b:c", "")
    );
}

#[test]
fn lang_and_space_are_inherited() {
    let doc = parse("<a xml:lang='en' xml:space='preserve'><b xml:lang='fr'><c/></b><d xml:space='default'/></a>").unwrap();
    let a = doc.root_element();
    let b = doc.first_child(a).unwrap();
    let c = doc.first_child(b).unwrap();
    let d = doc.next_sibling(b).unwrap();
    assert_eq!(
        (doc.xml_lang(a), doc.xml_lang(c), doc.xml_lang(d)),
        (Some("en"), Some("fr"), Some("en"))
    );
    assert!(doc.preserves_space(c) && !doc.preserves_space(d));
}

#[test]
fn spans_and_positions() {
    let src = "<a>\n <b x = 'é'>t</b>&amp;</a>";
    let doc = parse(src).unwrap();
    let a = doc.root_element();
    assert_eq!(doc.span(a), Span::new(0, src.len() as u32));
    let b = doc.children(a).find(|&n| doc.is_element(n)).unwrap();
    assert_eq!(doc.span(b).text(src), "<b x = 'é'>t</b>");
    let x = doc.attributes(b).next().unwrap();
    assert_eq!(x.span.text(src), "x = 'é'");
    assert!(x.specified);
    // Text either side of a reference is one node, spanning both.
    let last = doc.last_child(a).unwrap();
    assert_eq!(doc.kind(last), NodeKind::Text("&"));
    let e = parse("<a>\n\t<é></b></a>").unwrap_err();
    assert_eq!((e.position.line, e.position.column), (2, 5));
    assert_eq!(e.span().text("<a>\n\t<é></b></a>"), "</b>");
    assert_eq!(e.diagnostic.labels[0].span.text("<a>\n\t<é></b></a>"), "<é");
}

#[test]
fn line_ends_are_normalized() {
    let doc = parse("<a b='x\r\ny\rz\tw'>1\r\n2\r3<![CDATA[\r\n]]><!--\r--></a>").unwrap();
    let a = doc.root_element();
    assert_eq!(doc.attribute(a, "", "b"), Some("x y z w"));
    let kinds: Vec<_> = doc.children(a).map(|n| doc.kind(n)).collect();
    assert_eq!(
        kinds,
        [
            NodeKind::Text("1\n2\n3"),
            NodeKind::CData("\n"),
            NodeKind::Comment("\n")
        ]
    );
    // A character reference is kept as it is.
    let doc = parse("<a b='&#13;&#10;&#9;'>&#13;</a>").unwrap();
    let a = doc.root_element();
    assert_eq!(doc.attribute(a, "", "b"), Some("\r\n\t"));
    assert_eq!(doc.text(a), "\r");
}

#[test]
fn internal_entities_expand() {
    let src = r#"<!DOCTYPE a [
        <!ENTITY e "<b>&f;</b>">
        <!ENTITY f "x &#38;amp; y">
        <!ENTITY g "1&#x20;&#9;2">
        <!ENTITY e "ignored">
    ]><a v="&g;|&f;">&e;&e;</a>"#;
    let doc = parse(src).unwrap();
    let a = doc.root_element();
    // In an attribute, white space is a space; a character reference
    // replaced at declaration is a character the value keeps.
    assert_eq!(doc.attribute(a, "", "v"), Some("1  2|x & y"));
    let bs: Vec<_> = doc.children(a).collect();
    assert_eq!(bs.len(), 2);
    assert_eq!(doc.text(bs[0]), "x & y");
    // Nodes from an entity have the span of the reference.
    assert_eq!(doc.span(bs[1]).text(src), "&e;");
    assert_eq!(doc.entities().len(), 3);
    assert_eq!(doc.entities()[0].value.as_deref(), Some("<b>&f;</b>"));
    assert!(doc.warnings().is_empty());
    // A reference to a character that's markup is only text.
    let doc = parse("<!DOCTYPE a [<!ENTITY lt2 '&#38;#60;'>]><a>&lt2;</a>").unwrap();
    assert_eq!(doc.text(doc.root_element()), "<");
}

#[test]
fn parameter_entities_in_the_internal_subset() {
    let src = r#"<!DOCTYPE a [
        <!ENTITY % decls "<!ENTITY e 'from a PE'><!ATTLIST a d CDATA 'yes'>">
        %decls;
    ]><a>&e;</a>"#;
    let doc = parse(src).unwrap();
    let a = doc.root_element();
    assert_eq!(doc.text(a), "from a PE");
    assert_eq!(doc.attribute(a, "", "d"), Some("yes"));
    // Not inside a declaration, in the internal subset.
    assert_eq!(
        code("<!DOCTYPE a [<!ENTITY % t 'CDATA'><!ATTLIST a b %t; #IMPLIED>]><a/>"),
        "pe-in-markup-declaration"
    );
    assert_eq!(
        code("<!DOCTYPE a [<!ENTITY % p 'x'><!ENTITY e '%p;'>]><a/>"),
        "pe-in-markup-declaration"
    );
    // A standalone document can't use a general entity declared in one.
    assert_eq!(
        code(
            "<?xml version='1.0' standalone='yes'?><!DOCTYPE a [<!ENTITY % d '<!ENTITY e \"x\">'>%d;]><a>&e;</a>"
        ),
        "undeclared-entity"
    );
}

#[test]
fn what_isnt_read_is_reported() {
    let src = r#"<!DOCTYPE a SYSTEM "a.dtd" [
        <!ENTITY ext SYSTEM "ext.xml">
        <!ENTITY % pe SYSTEM "more.dtd">
        <!ENTITY before "1">
        %pe;
        <!ENTITY after "2">
    ]><a>&ext;&before;&undeclared;</a>"#;
    let doc = parse(src).unwrap();
    let codes: Vec<_> = doc.warnings().iter().map(|w| w.code).collect();
    assert_eq!(
        codes,
        [
            "external-dtd-not-loaded",
            "entity-not-loaded",
            "declarations-skipped",
            "entity-not-loaded",
            "entity-not-loaded"
        ]
    );
    assert!(doc.warnings().iter().all(|w| !w.is_error()));
    let kinds: Vec<_> = doc
        .children(doc.root_element())
        .map(|n| doc.kind(n))
        .collect();
    assert_eq!(
        kinds,
        [
            NodeKind::EntityRef("ext"),
            NodeKind::Text("1"),
            NodeKind::EntityRef("undeclared")
        ]
    );
    assert_eq!(doc.doctype().unwrap().system_id.as_deref(), Some("a.dtd"));
    // `after` came after a parameter entity that wasn't read.
    assert!(doc.entities().iter().all(|e| e.name != "after"));
    // Without unread parts, an undeclared entity is an error.
    assert_eq!(code("<a>&nope;</a>"), "undeclared-entity");
    assert_eq!(
        code("<!DOCTYPE a [<!ENTITY x 'y'>]><a>&nope;</a>"),
        "undeclared-entity"
    );
    assert_eq!(
        code("<?xml version='1.0' standalone='yes'?><!DOCTYPE a SYSTEM 'a.dtd'><a>&nope;</a>"),
        "undeclared-entity"
    );
    // An external entity can't be in an attribute value at all.
    assert_eq!(
        code("<!DOCTYPE a [<!ENTITY ext SYSTEM 'x'>]><a b='&ext;'/>"),
        "external-entity-in-attribute"
    );
    assert_eq!(
        code("<!DOCTYPE a [<!NOTATION n SYSTEM 'n'><!ENTITY u SYSTEM 'x' NDATA n>]><a>&u;</a>"),
        "unparsed-entity-reference"
    );
}

#[test]
fn attribute_defaults_and_types() {
    let src = r#"<!DOCTYPE a [
        <!ATTLIST a
            id ID #IMPLIED
            kind (x|y) "x"
            fixed CDATA #FIXED " keep  spaces "
            xmlns:p CDATA "urn:p">
        <!ATTLIST a kind CDATA "ignored" more NMTOKENS "  1   2 ">
    ]><a id="  i1  " p:z="1"/>"#;
    let doc = parse(src).unwrap();
    let a = doc.root_element();
    let attrs: Vec<_> = doc
        .attributes(a)
        .map(|a| (a.name.qname, a.value, a.specified))
        .collect();
    assert_eq!(
        attrs,
        [
            ("id", "i1", true),
            ("p:z", "1", true),
            ("kind", "x", false),
            ("fixed", " keep  spaces ", false),
            ("xmlns:p", "urn:p", false),
            ("more", "1 2", false),
        ]
    );
    // The defaulted declaration binds the prefix.
    assert_eq!(doc.attribute(a, "urn:p", "z"), Some("1"));
    assert_eq!(
        code("<!DOCTYPE a [<!ATTLIST a b CDATA '&nope;'>]><a/>"),
        "undeclared-entity"
    );
    assert_eq!(
        code("<!DOCTYPE a [<!ATTLIST a b CDATA '<'>]><a/>"),
        "lt-in-attribute"
    );
}

#[test]
fn not_well_formed() {
    let cases = [
        ("", "no-root"),
        ("  ", "no-root"),
        ("<a>", "unexpected-eof"),
        ("<a></b>", "mismatched-end-tag"),
        ("<a/><b/>", "content-after-root"),
        ("<a/>text", "content-after-root"),
        ("text<a/>", "expected-root"),
        ("<a b='1' b='2'/>", "duplicate-attribute"),
        ("<a b='1'c='2'/>", "unexpected-char"),
        ("<a b=1/>", "unexpected-char"),
        ("<a b='<'/>", "lt-in-attribute"),
        ("<a>&</a>", "invalid-reference"),
        ("<a>&x</a>", "invalid-reference"),
        ("<a>&#xD800;</a>", "invalid-char-ref"),
        ("<a>&#0;</a>", "invalid-char-ref"),
        ("<a>&#99999999999;</a>", "invalid-char-ref"),
        ("<a>&#;</a>", "invalid-char-ref"),
        ("<a>]]></a>", "cdata-end-in-text"),
        ("<a>\u{1}</a>", "invalid-char"),
        ("<a>\u{FFFE}</a>", "invalid-char"),
        ("<a b='\u{FFFF}'/>", "invalid-char"),
        ("<a><!-- a -- b --></a>", "double-hyphen-in-comment"),
        ("<a><!-- a ---></a>", "double-hyphen-in-comment"),
        ("<a><!-- a </a>", "unexpected-eof"),
        ("<a><?xml version='1.0'?></a>", "reserved-pi-target"),
        (" <?xml version='1.0'?><a/>", "reserved-pi-target"),
        ("<a><?XmL x?></a>", "reserved-pi-target"),
        ("<a><?pi?x?></a>", "unexpected-char"),
        ("<a><!DOCTYPE a></a>", "invalid-markup"),
        ("<a/><!DOCTYPE a>", "misplaced-doctype"),
        ("<!DOCTYPE a><!DOCTYPE a><a/>", "misplaced-doctype"),
        ("<?xml version='2.0'?><a/>", "invalid-version"),
        ("<?xml encoding='UTF-8'?><a/>", "unexpected-char"),
        (
            "<?xml version='1.0' standalone='maybe'?><a/>",
            "invalid-standalone",
        ),
        (
            "<?xml version='1.0' standalone='yes' encoding='UTF-8'?><a/>",
            "unexpected-char",
        ),
        (
            "<?xml version='1.0' encoding='8'?><a/>",
            "invalid-encoding-name",
        ),
        ("<a></ a>", "unexpected-char"),
        (
            "<!DOCTYPE a [<!ENTITY e '<b>'>]><a>&e;</b></a>",
            "unbalanced-entity",
        ),
        (
            "<!DOCTYPE a [<!ENTITY e '</a>'>]><a>&e;",
            "unbalanced-entity",
        ),
        (
            "<!DOCTYPE a [<!ENTITY e '&e;'>]><a>&e;</a>",
            "recursive-entity",
        ),
        (
            "<!DOCTYPE a [<!ENTITY e '&f;'><!ENTITY f '&e;'>]><a b='&e;'/>",
            "recursive-entity",
        ),
        ("<!DOCTYPE a [<!ENTITY e '<'>]><a>&e;</a>", "invalid-name"),
        (
            "<!DOCTYPE a [<!ENTITY e '<'>]><a b='&e;'/>",
            "lt-in-attribute",
        ),
        (
            "<!DOCTYPE a [<!ELEMENT a (b|c,d)>]><a/>",
            "invalid-content-model",
        ),
        (
            "<!DOCTYPE a [<!ELEMENT a (#PCDATA|b)>]><a/>",
            "unexpected-char",
        ),
        ("<!DOCTYPE a [<!ELEMENT a ()>]><a/>", "unexpected-char"),
        (
            "<!DOCTYPE a [<!ENTITY e PUBLIC 'a\"b' 'c'>]><a/>",
            "invalid-public-id",
        ),
        ("<!DOCTYPE a [<![INCLUDE[]]>]><a/>", "unexpected-char"),
        (
            "<!DOCTYPE a [<!ENTITY % p SYSTEM 'x' NDATA n>]><a/>",
            "unexpected-char",
        ),
        ("<!DOCTYPE a [", "unexpected-eof"),
        ("<a\u{0}/>", "unexpected-char"),
    ];
    for (src, want) in cases {
        match parse(src) {
            Ok(_) => panic!("{src:?} parsed"),
            Err(e) => assert_eq!(e.code(), want, "{src:?}: {e}"),
        }
    }
}

#[test]
fn well_formed_odds_and_ends() {
    for src in [
        "<a/>",
        "\u{FEFF}<a/>",
        "<?xml version='1.1'?><a/>",
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\" ?>\n<a/>\n<!--end-->\n<?pi?>",
        "<a b = \"'\" c='\"'></a >",
        "<a>]]</a>",
        "<a>]>]]&gt;</a>",
        "<a><![CDATA[]]]]><![CDATA[>]]></a>",
        "<!DOCTYPE a PUBLIC '-//A//B' \"x.dtd\"><a/>",
        "<!DOCTYPE a [<!ELEMENT a ((b|c)*,(d?,e+))><!ELEMENT b EMPTY><!ELEMENT c ANY><!ELEMENT d (#PCDATA)><!ELEMENT e ( #PCDATA | b | c )*>]><a/>",
        "<!DOCTYPE a [<!NOTATION n PUBLIC 'p'><!NOTATION m SYSTEM 's'><!ATTLIST a n NOTATION (n|m) #IMPLIED>]><a/>",
        "<!DOCTYPE a [<!-- c --><?pi in dtd?> %undeclared; ]><a/>",
        "<!DOCTYPE a [<!ENTITY % i '<![INCLUDE[<!ENTITY e \"in\">]]><![IGNORE[<![ x ]]> junk ]]>'>%i;]><a>&e;</a>",
        "<a>&#x10FFFF;&#9;&#xa;</a>",
        "<é·><_-.0/></é·>",
    ] {
        if let Err(e) = parse(src) {
            panic!("{src:?}: {e}");
        }
    }
}

#[test]
fn bytes_in() {
    let opts = Options::default();
    let utf16le: Vec<u8> = "\u{FEFF}<a>é</a>"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let p = parse_bytes(&utf16le, &opts).unwrap();
    assert_eq!(
        (p.encoding.name(), p.document.text(p.document.root())),
        ("UTF-16LE", "é".into())
    );
    assert_eq!(p.text, "<a>é</a>");
    // UTF-16 without a byte order mark, known by its first bytes.
    let be: Vec<u8> = "<?xml version='1.0' encoding='UTF-16'?><a>€</a>"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    let p = parse_bytes(&be, &opts).unwrap();
    assert_eq!(p.encoding.name(), "UTF-16BE");
    // UTF-8 is borrowed.
    let p = parse_bytes(b"\xEF\xBB\xBF<a/>", &opts).unwrap();
    assert!(matches!(p.text, std::borrow::Cow::Borrowed("<a/>")));
    let p = parse_bytes(
        b"<?xml version='1.0' encoding='Shift_JIS'?><a>\x93\xfa</a>",
        &opts,
    )
    .unwrap();
    assert_eq!(p.document.text(p.document.root()), "日");
    let e = parse_bytes(b"<a>\n ok \xff</a>", &opts).unwrap_err();
    assert_eq!(
        (e.code(), e.position.line, e.position.column),
        ("invalid-byte-sequence", 2, 5)
    );
    let e = parse_bytes(b"<?xml version='1.0' encoding='x-nope'?><a/>", &opts).unwrap_err();
    assert_eq!(e.code(), "unsupported-encoding");
    let e = parse_bytes(b"<?xml version='1.0' encoding='UTF-16'?><a/>", &opts).unwrap_err();
    assert_eq!(e.code(), "encoding-mismatch");
    let e = parse_bytes(b"\0\0\0<\0\0\0?", &opts).unwrap_err();
    assert_eq!(e.code(), "unsupported-encoding");
    let e = parse_bytes(b"<a>", &opts).unwrap_err();
    assert_eq!(e.code(), "unexpected-eof");
}

#[test]
fn the_serializer_round_trips() {
    for src in [
        r#"<?xml version="1.0" standalone="yes"?>
<!DOCTYPE r [<!ENTITY e "<i>x</i>"><!ATTLIST r d CDATA "1">]>
<!--top-->
<r xmlns="urn:a" xmlns:b="urn:b" b:at="&quot;'&#9;&#10;&#13;&lt;&amp;"><b:c>t &amp; &lt; &gt; &#13;</b:c>&e;<![CDATA[ <&> ]]><?pi data?><d xmlns=""/></r>
<?after?>"#,
        "<!DOCTYPE a SYSTEM 'x.dtd'><a>&ext;</a>",
        "<a xmlns:p='u'><p:b><p:c p:x='1'/></p:b></a>",
    ] {
        let doc = parse(src).unwrap();
        let once = doc.to_xml();
        let again = parse(&once).unwrap_or_else(|e| panic!("{once}: {e}"));
        assert_eq!(again.to_xml(), once);
        assert_eq!(again.len(), doc.len(), "{once}");
        for (a, b) in doc
            .descendants(doc.root())
            .zip(again.descendants(again.root()))
        {
            assert_eq!(doc.kind(a), again.kind(b));
            let x: Vec<_> = doc.attributes(a).map(|a| (a.name, a.value)).collect();
            let y: Vec<_> = again.attributes(b).map(|a| (a.name, a.value)).collect();
            assert_eq!(x, y);
        }
    }
    // An element alone takes the declarations it's in the scope of.
    let doc = parse("<a xmlns='urn:d' xmlns:p='urn:p'><b xmlns:p='urn:q'><p:c/></b></a>").unwrap();
    let b = doc.first_child(doc.root_element()).unwrap();
    let mut out = String::new();
    doc.write_xml(b, &mut out);
    assert_eq!(out, "<b xmlns:p=\"urn:q\" xmlns=\"urn:d\"><p:c/></b>");
    let mut whole = String::new();
    doc.write_xml(doc.root(), &mut whole);
    assert_eq!(whole, doc.to_xml());
}
