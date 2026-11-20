// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Parsec: an HTML parser that follows the WHATWG HTML Standard, with CSS
//! selectors and a tree you can change and write back out, and the other
//! standards a page needs beside it.
//!
//! The parser is at the top of this crate, and each standard is a module
//! behind a cargo feature:
//!
//! | Module | Feature | Default |
//! |---|---|---|
//! | `encoding` | `encoding` | on |
//! | `url` | `url` | on |
//! | `xpath` | `xpath` | on |
//! | `stylesheet` | `css` | off |
//!
//! [`css`] is the selector engine, which is always here; `stylesheet` is
//! CSS Syntax and the cascade, which a scraper rarely needs.
//!
//! ```
//! use parsec::css::SelectorList;
//!
//! let doc = parsec::parse("<ul><li class=hit>one<li>two</ul>", false);
//! let hits = SelectorList::parse("li.hit").unwrap().select(&doc, doc.root());
//! assert_eq!(doc.text(hits[0]), "one");
//! ```

#[doc(inline)]
pub use html::*;

#[cfg(feature = "encoding")]
#[doc(inline)]
pub use parsec_encoding as encoding;

#[cfg(feature = "url")]
#[doc(inline)]
pub use parsec_url as url;

#[cfg(feature = "xpath")]
#[doc(inline)]
pub use parsec_xpath as xpath;

#[cfg(feature = "css")]
#[doc(inline)]
pub use parsec_css as stylesheet;
