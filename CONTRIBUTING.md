# Contributing to Parsec

Thanks for wanting to help. A page Parsec parses differently from a
browser, a selector it gets wrong, something Lexbor does that Parsec
can't, a fix, a faster loop: all of it is welcome, and most of it starts
with an issue.

## Before you write code

Open an issue first for anything bigger than a small fix, so we can agree
on the shape before you spend an evening on it. For questions, use
[Discussions](https://github.com/parsec-html/parsec/discussions) rather
than an issue.

If an issue is labelled `good first issue`, it's small, self-contained and
I'll help you through it.

## Where a change goes

Parsec is a set of crates, one per language, over shared plumbing. The
wiki's [Architecture](https://github.com/parsec-html/parsec/wiki/Architecture#the-crates-and-how-they-fit)
page has a diagram of them and the rule they follow: each language keeps
its own tree and its own error handling, and only `parsec-base` is
shared. A change that makes one crate depend on another needs a reason
in its pull request.

Everything new, `parsec-js`, `parsec-xml` and `parsec-base` included, is
written here. The HTML parser is the exception: it has two homes. Its source is Netweir's
[`crates/netweir-html`](https://github.com/netweir/netweir/tree/main/crates/netweir-html),
and `scripts/sync-from-netweir.sh` copies it into `crates/parsec`,
renaming as it goes. The Netweir commit it came from is in
`NETWEIR_COMMIT`.

So if your change touches any of these, make it in Netweir first:

- `crates/parsec/src`
- `crates/parsec/tests/*.rs`
- `crates/parsec/examples`
- `crates/parsec/scripts`
- `crates/parsec/fuzz/fuzz_targets`
- `crates/parsec-xpath/src` and `crates/parsec-xpath/tests`, which come from
  Netweir's [`crates/netweir-xpath`](https://github.com/netweir/netweir/tree/main/crates/netweir-xpath)

Once it's merged there, a pull request here runs the sync and moves
`NETWEIR_COMMIT` forward:

```
scripts/sync-from-netweir.sh ../netweir
```

CI checks that those paths match Netweir at the pinned commit, so an edit
made here alone fails it. Everything else in the repository, including
the manifests, the test data and every crate beside `crates/parsec`, is
Parsec's own, and you change it here directly.

## Getting it running

You need Rust 1.88 or newer. That's all.

```
git clone --recurse-submodules https://github.com/parsec-html/parsec
cd parsec
cargo test
```

Cloned without `--recurse-submodules`? The tokenizer tests read
[html5lib-tests](https://github.com/html5lib/html5lib-tests), which is a
submodule, so fetch it before you test:

```
git submodule update --init
```

The tree-construction tests aren't a submodule. They moved from
html5lib-tests to web-platform-tests, and the files are pinned in
`crates/parsec/tests/tree-construction`, with the WPT commit in `COMMIT`
beside them. `crates/parsec/scripts/fetch-tree-tests.sh` moves them to a
newer commit.

## Checking your change

These are what CI runs, so running them first saves a round trip:

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

The fuzz target builds on nightly with
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), from
`crates/parsec`:

```
cargo +nightly fuzz run parse
```

The JavaScript lexer has one too, from `crates/parsec-js`:

```
cargo +nightly fuzz run lex
```

Each conformance suite writes its counts when `CONFORMANCE_DIR` is set,
and `conformance/baseline.json` holds the last release's. CI's
`conformance` job compares the two on every pull request, and a release
can't ship with any suite passing fewer cases or failing more. To see the
comparison before you push:

```
CONFORMANCE_DIR=target/conformance cargo test --workspace
scripts/conformance-gate.sh conformance/baseline.json target/conformance
```

Don't edit the baseline to make your pull request pass: it changes only
in a release pull request. If your change trades one case for a better
one, say so in the pull request; the drop shows there in red for
everyone to weigh.

A bug fix comes with the test that would have caught it. A new feature
comes with tests, and a page in the
[wiki](https://github.com/parsec-html/parsec/wiki) if you'd use it from
your own code.

## Commits and pull requests

Commit subjects take a
[conventional](https://www.conventionalcommits.org/) prefix and describe
the problem rather than the action, in 72 characters or fewer:

```
fix(tree): a table inside a template lost its foster parent
feat(css): selectors could match :has() only one level down
```

The body, wrapped at 72 columns, says what was wrong and what changed.

Keep a pull request to one change. The template will ask what it fixes and
how you tested it.

## How review works

Every pull request is read in full before it merges, including the
maintainer's own; once Parsec has two maintainers, it needs an approving
review from the one who didn't write it ([GOVERNANCE.md](GOVERNANCE.md)).
A reviewer checks, in this order:

1. **It's right.** It does what the HTML Standard (or the spec in question)
   says, and a test shows it: one that fails without the change.
2. **It's safe.** No new `unsafe`, no recursion whose depth a page
   controls, and nothing whose time grows faster than the input. Anything
   that could crash or stall on hostile input is a security question (see
   [SECURITY.md](SECURITY.md)).
3. **It's not slower.** Parser changes come with before-and-after numbers
   on the benchmark corpus; a slowdown has to be worth what it buys, and
   the pull request says so.
4. **It's documented.** The wiki page for anything users can see is
   updated in the same change, and the README if a claim there changes.
5. **It's tidy.** CI is green: formatting, clippy, cargo-deny, the drift
   check and the tests on Linux, macOS and Windows.

A pull request merges once all of that holds and every review thread is
resolved. If a contributor and reviewer disagree, the reasoning goes in the
thread and the maintainer decides, as GOVERNANCE.md describes.

## The contributor licence agreement

Parsec is Apache-2.0. Its parser is shared with Netweir, which is AGPL-3.0
and also sold under a commercial licence, so code that lands here may go
there too. That only works if I can license each contribution under other
terms as well, so before your first pull request is merged you'll be asked
to agree to the [CLA](CLA.md). You keep the copyright in your work.

A check on your pull request will ask you to post one comment:

> I have read the CLA and I agree to it.

You only do this once.

## Where the docs live

In the [wiki](https://github.com/parsec-html/parsec/wiki). There's no
separate website. Anyone with a GitHub account can suggest a fix there;
for anything bigger, open an issue first.

## Being decent

Everyone taking part agrees to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Please don't open a public issue for a security problem. See
[SECURITY.md](SECURITY.md).
