# Security

## Reporting a problem

Please don't open a public issue. Report it privately instead: on the
repository's "Security" tab, choose "Report a vulnerability", or go
straight to
[the form](https://github.com/parsec-html/parsec/security/advisories/new).
Tell me what you found, how to reproduce it, and what an attacker could do
with it. The input that triggers it is the most useful thing you can send.

You'll hear back within a week. Once there's a fix, it ships with an
advisory crediting you, unless you'd rather not be named.

## What counts

Parsec reads HTML that nobody vouches for. Anything that lets that HTML,
or a selector, do more than produce a tree is in scope. For example:

- input that makes the parser read or write memory it shouldn't, through
  `unsafe` code or otherwise;
- input that crashes it: a panic, a stack overflow, an abort;
- input that hangs it, or makes it take time or memory far out of
  proportion to its size, which is a denial of service for anyone parsing
  pages from the web;
- a selector or DOM edit that does any of the above.

Output that differs from what a browser builds, with nothing worse, is a
bug rather than a security problem. Please open an ordinary issue for it.

## Supported versions

There's no release yet, so for now fixes go to `main`. Once there is one,
fixes go into the newest minor release. Until Parsec reaches 1.0, older
releases don't get backports.

## Verifying a release

A release is a signed tag, and on its GitHub release page the crate
(`parsec-html-VERSION.crate`), its `SHA256SUMS` and the provenance bundle
(`parsec-vVERSION.intoto.jsonl`). Each can be checked on its own.

**The tag** is signed with a maintainer's SSH key, listed in
[`.github/allowed_signers`](.github/allowed_signers). Take that file from
`main` rather than from the tag, as the release workflow does:

```sh
git fetch origin tag v0.1.0
git show origin/main:.github/allowed_signers > allowed_signers
git -c gpg.ssh.allowedSignersFile=allowed_signers tag -v v0.1.0
```

**The crate** was built by the release workflow from that tag, and GitHub
signed a record of it:

```sh
gh attestation verify parsec-html-0.1.0.crate --repo parsec-html/parsec \
  --signer-workflow parsec-html/parsec/.github/workflows/release.yml
```

**The C libraries** (`parsec-capi-VERSION-TARGET.tar.gz` or `.zip`, one
per platform, listed in `parsec-capi-SHA256SUMS`) are built and attested
the same way, by their own workflow:

```sh
gh attestation verify parsec-capi-0.1.0-x86_64-unknown-linux-gnu.tar.gz \
  --repo parsec-html/parsec \
  --signer-workflow parsec-html/parsec/.github/workflows/release-capi.yml
```

They're attached only once the tag's signature has checked out. Unlike
the crate, they aren't yet checked to rebuild byte for byte.

**A rebuild** gives the same file, byte for byte. At the tag, run

```sh
git checkout v0.1.0
scripts/package-twice.sh
```

It packages the crate twice from fresh checkouts, fails if they differ, and
prints the SHA-256, which must match the release's `SHA256SUMS`. Any recent
stable Rust gives the same package: cargo 1.99 and 1.101 do. CI runs the
same script on every change, so a change that would break this fails there
first.
