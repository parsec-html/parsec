// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! CSS Syntax Level 3: the tokenizer, the parser and its entry points, the
//! An+B and `<urange>` microsyntaxes, and serialization back to CSS, as
//! the W3C's current editor's draft defines them, nesting included.
//!
//! Every rule, declaration, block, function and token carries its
//! [`Span`] in the input, and [`LineIndex`] turns one into a line and
//! column. Names and values borrow the input unless an escape changes
//! them. `tests/` runs the css-parsing-tests suite, and the CSS tokenizer
//! tests, against it.
//!
//! ```
//! use parsec_css::{Item, Rule};
//!
//! let css = "a { color: red; &:hover { color: blue } }";
//! let sheet = parsec_css::parse_stylesheet(css);
//! let Rule::Qualified(rule) = &sheet.rules[0] else { unreachable!() };
//! assert_eq!(parsec_css::to_css(&rule.prelude), "a ");
//!
//! let contents = rule.block.clone().contents(css);
//! let Item::Declaration(color) = &contents.items[0] else { unreachable!() };
//! assert_eq!((&*color.name, color.span.start), ("color", 4));
//! assert!(matches!(contents.items[1], Item::Rule(_)));
//! ```

mod bytes;
mod microsyntax;
mod parser;
mod position;
mod serialize;
mod tokenizer;

pub use bytes::{decode_stylesheet, fallback_encoding};
pub use encoding_rs::Encoding;
pub use microsyntax::{UnicodeRange, anb, parse_anb, parse_unicode_ranges, serialize_anb};
pub use parser::{
    AtRule, BlockContents, Bracket, ComponentValue, Declaration, ErrorKind, Function, Item,
    ParseError, QualifiedRule, Rule, SimpleBlock, Stylesheet, parse_block_contents,
    parse_comma_separated, parse_component_value, parse_component_values, parse_declaration,
    parse_declaration_list, parse_rule, parse_rule_list, parse_stylesheet,
    parse_unicode_range_value,
};
pub use position::LineIndex;
pub use serialize::{to_css, write_identifier, write_string, write_values};
pub use tokenizer::{Number, Span, Token, Tokenizer};
