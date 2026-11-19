# Governance

## Today

James Conway ([@jamesconwaydev](https://github.com/jamesconwaydev)) started
Parsec and holds its copyright. Parsec has two maintainers, listed below.
Decisions happen in public, in issues, Discussions and pull requests; the
maintainers reach them together, and James makes the final call when they
can't agree.

## Licensing

Parsec is licensed under the Apache License, version 2.0. Its parser is
shared with Netweir, which is AGPL-3.0 and also offered under a commercial
licence, so a contribution here can go there too. That's why contributors
agree to the [CLA](CLA.md) once, before their first pull request is
merged. You keep the copyright in your work; the CLA gives the maintainer
the licence needed to use it in both projects.

## How changes get in

Every change, including the maintainer's own, goes through a pull request
that passes CI on Linux, macOS and Windows. Nothing is pushed straight to
`main`. The parser core is made in Netweir first and synced here, as
[CONTRIBUTING.md](CONTRIBUTING.md) explains, along with the checks and
what a pull request needs.

## Maintainers

Anyone who contributes steadily and reviews other people's work can be
invited to become a maintainer. Once there are at least two maintainers,
pull requests need an approving review from a maintainer other than the
author.

The list of maintainers lives in this file:

- James Conway ([@jamesconwaydev](https://github.com/jamesconwaydev))
- foothillsdigital ([@foothillsdigital](https://github.com/foothillsdigital)),
  since November 2026

Both are admins of the repository, so either one can change its settings,
rulesets and access, and keep Parsec going if the other can't. Their
public signing keys are in
[`.github/allowed_signers`](.github/allowed_signers), and the release
workflow refuses a tag that isn't signed by one of them. Only repository
admins can create `v*` tags.
