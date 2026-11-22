# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
"""Refreshes the three community dashboards pinned in parsec-html/parsec.

Help wanted, community pull requests and roadmap status, each written into
one issue. Each issue is found by a hidden marker in its body, so a run
edits the same issue every time and only creates one when it is missing.

Needs the gh CLI, authenticated (GH_TOKEN in Actions), with issues: write
on the repository.

    python3 scripts/dashboards.py --dry-run [--out DIR]
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.parse
from datetime import datetime, timezone

REPO = "parsec-html/parsec"
CORE_TEAM = {"jamesconwaydev"}
MAINTAINER_ROLES = {"OWNER", "MEMBER", "COLLABORATOR"}
# First label that matches decides the section, so an issue shows up once.
KINDS = [
    ("good first issue", "🌱 Good first issues"),
    ("help wanted", "🤝 Help wanted"),
]
CONTRIBUTING = f"https://github.com/{REPO}/blob/main/CONTRIBUTING.md"
MARKER = re.compile(r"<!-- parsec-dashboard:([\w-]+) -->")
NEXT = 5  # open issues listed under each milestone
BODY_LIMIT = 65000  # GitHub rejects issue bodies over 65536 characters.
CLAIM = re.compile(
    r"\b(i'?ll take|i will take|"
    r"(i'?d|i would) (like|love) to (work|take|try|help|pick|fix)|"
    r"can i (take|work|pick|have)|may i (take|work)|assign (it |this )?(to )?me|"
    r"working on (this|it)|i'?m on (it|this)|dibs)\b",
    re.I,
)

calls = 0


def gh(*args, payload=None):
    global calls
    for attempt in range(3):
        calls += 1
        out = subprocess.run(
            ["gh", "api", *args],
            input=None if payload is None else json.dumps(payload),
            capture_output=True, text=True, check=False,
        )
        # Big GraphQL searches sometimes get a 502; a second try usually works.
        if out.returncode == 0 or "HTTP 5" not in out.stderr or attempt == 2:
            break
        time.sleep(10)
    if out.returncode != 0:
        sys.exit(f"gh api {' '.join(args[:3])} failed: {out.stderr.strip()}")
    return json.loads(out.stdout) if out.stdout.strip() else None


def graphql(query, **variables):
    data = gh("graphql", "--input", "-", payload={"query": query, "variables": variables})
    if data.get("errors"):
        sys.exit(f"GraphQL error: {data['errors']}")
    return data["data"]


def search(query, fields):
    q = """query($q: String!, $after: String) {
      search(query: $q, type: ISSUE, first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes { %s }
      }
    }""" % fields
    nodes, after = [], None
    while True:
        page = graphql(q, q=query, after=after)["search"]
        nodes += [n for n in page["nodes"] if n]
        if not page["pageInfo"]["hasNextPage"]:
            return nodes
        after = page["pageInfo"]["endCursor"]


# ---- Formatting -----------------------------------------------------------

def esc(text):
    """Make text safe for a table cell that must never notify anyone."""
    text = " ".join(text.split()).replace("|", "\\|").replace("<", "&lt;")
    text = re.sub(r"(?<![\w`])@([A-Za-z0-9][\w-]*(?:/[\w.-]+)?)", r"`@\1`", text)
    # A bare #12 in a title would cross-link that issue from every snapshot.
    return re.sub(r"(?<![\w`&])#(\d+)", r"`#\1`", text)


def handle(login):
    return f"`@{login}`"


def ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def age(since, now):
    hours = int((now - since).total_seconds() // 3600)
    if hours < 1:
        return "<1h"
    if hours < 48:
        return f"{hours}h"
    return f"{hours // 24}d"


def table(header, rows, empty="Nothing here right now."):
    if not rows:
        return empty + "\n"
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    lines += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(lines) + "\n"


def capped(rows, cap, more_url):
    if cap is None or len(rows) <= cap:
        return rows, ""
    return rows[:cap], f"\n…and {len(rows) - cap} more. [See them all]({more_url}).\n"


def fit(render):
    """Render, shrinking the per-section row cap until the body fits."""
    body, cap = render(None), 200
    while len(body) > BODY_LIMIT and cap > 5:
        body, cap = render(cap), cap // 2
    return body


def header(title, marker, now, intro):
    return (
        f"<!-- parsec-dashboard:{marker} -->\n# {title}\n\n"
        f"_Snapshot taken {now:%Y-%m-%d %H:%M} UTC. This issue updates itself every hour, "
        "so please don't edit it by hand._\n\n" + intro.strip() + "\n"
    )


def issues_url(q):
    return f"https://github.com/{REPO}/issues?q=" + urllib.parse.quote_plus(q)


# ---- Help wanted ----------------------------------------------------------

HELP_FIELDS = """... on Issue {
  number title url createdAt
  labels(first: 20) { nodes { name } }
  assignees(first: 3) { nodes { login } }
  comments(last: 30) { nodes { author { login } authorAssociation body } }
  closedByPullRequestsReferences(first: 1, includeClosedPrs: false) { totalCount }
}"""


def maintainer(login, association):
    """Whether someone speaks for the project: the core team, or anyone
    GitHub says owns, belongs to or collaborates on the repository."""
    return login in CORE_TEAM or association in MAINTAINER_ROLES


def claim(issue):
    """Who has this issue, as display text, or None if it is free."""
    assignees = [a["login"] for a in issue["assignees"]["nodes"]]
    if assignees:
        return "taken by " + ", ".join(map(handle, assignees))
    if issue["closedByPullRequestsReferences"]["totalCount"]:
        return "taken (PR open)"
    for c in reversed(issue["comments"]["nodes"]):
        who = (c["author"] or {}).get("login", "")
        if maintainer(who, c.get("authorAssociation")) or who.endswith("[bot]"):
            continue
        if who and CLAIM.search(c["body"]):
            return "taken by " + handle(who)
    return None


def kind(issue):
    names = {label["name"] for label in issue["labels"]["nodes"]}
    return next((label for label, _ in KINDS if label in names), None)


def help_board(issues, now):
    intro = f"""
Every open Parsec issue where we'd love a hand, in one place. Find one
marked **free**, say on the issue that you're taking it, and it's yours. A
question first is fine too.

New here? The [contributing guide]({CONTRIBUTING}) covers setup, the tests
and where a change goes: most of the parser's source lives in Netweir and
syncs across, so a fix there may start in the other repository. A good
first issue is small and self-contained, and I'll help you through it.

**Taken** means someone is assigned, has said they're on it, or has an open
pull request linked. If one has gone quiet for a few weeks, ask on the issue.
"""
    groups = {label: [] for label, _ in KINDS}
    for i in sorted(issues, key=lambda i: i["createdAt"]):
        if kind(i):
            groups[kind(i)].append(i)
    listed = [i for g in groups.values() for i in g]
    free = sum(not claim(i) for i in listed)

    def render(cap):
        out = [header("🙋 Help wanted across Parsec", "help-wanted", now, intro)]
        out.append(f"\n{free} of {len(listed)} are free right now.\n")
        for label, title in KINDS:
            rows = [[
                f"[{esc(i['title'])}]({i['url']})",
                age(ts(i["createdAt"]), now),
                claim(i) or "**free**",
            ] for i in groups[label]]
            rows, more = capped(rows, cap, issues_url(f'is:issue is:open label:"{label}"'))
            out.append(f"\n## {title} ({len(groups[label])})\n\n")
            out.append(table(["Issue", "Age", "Status"], rows) + more)
        return "".join(out)

    return fit(render)


# ---- Pull requests --------------------------------------------------------

PR_FIELDS = """... on PullRequest {
  number title url isDraft createdAt additions deletions changedFiles mergeable
  author { __typename login } authorAssociation
  commits(last: 1) { nodes { commit { committedDate statusCheckRollup { state } } } }
  reviews(last: 30) { nodes { author { login } authorAssociation submittedAt } }
  comments(last: 30) { nodes { author { login } authorAssociation createdAt } }
}"""

BUCKETS = [
    ("maintainer", "Waiting on us", "We owe these a review or a reply."),
    ("author", "Waiting on the author", "We've left feedback, so the next move is yours."),
    ("ci", "Failing CI", "Checks are red. The logs usually say why; ask on the PR if they don't."),
    ("rebase", "Needs a rebase", "These conflict with main."),
    ("draft", "Drafts", "Still in progress. Mark it ready when you want a review."),
]


def outside(pr):
    a = pr["author"]
    return bool(a) and a["__typename"] != "Bot" and not maintainer(a["login"], pr.get("authorAssociation"))


def bucket(pr):
    """Return (bucket, waiting since) for one PR."""
    author = pr["author"]["login"]
    commit = pr["commits"]["nodes"][0]["commit"] if pr["commits"]["nodes"] else {}
    last_author = max([pr["createdAt"], commit.get("committedDate") or pr["createdAt"]])
    last_us = None
    for item in pr["reviews"]["nodes"] + pr["comments"]["nodes"]:
        who = (item["author"] or {}).get("login")
        when = item.get("submittedAt") or item.get("createdAt")
        if not who or not when:
            continue
        if who == author:
            last_author = max(last_author, when)
        elif maintainer(who, item["authorAssociation"]):
            last_us = max(last_us or when, when)
    if pr["isDraft"]:
        return "draft", last_author
    if pr["mergeable"] == "CONFLICTING":
        return "rebase", last_author
    if (commit.get("statusCheckRollup") or {}).get("state") in ("FAILURE", "ERROR"):
        return "ci", last_author
    if last_us and last_us > last_author:
        return "author", last_us
    return "maintainer", last_author


def pr_board(prs, now):
    intro = f"""
Open pull requests from the community. That's everyone except
{handle("jamesconwaydev")} and the bots. If yours is here, thank you.

Each list is sorted by how long it has been waiting, longest first.
"""
    groups = {key: [] for key, _, _ in BUCKETS}
    for pr in prs:
        key, since = bucket(pr)
        groups[key].append((ts(since), pr))
    out = [header("🔀 Community pull requests", "pr-triage", now, intro)]
    for key, title, blurb in BUCKETS:
        rows = [[
            f"[{esc(pr['title'])}]({pr['url']})",
            handle(pr["author"]["login"]),
            f"+{pr['additions']} −{pr['deletions']}, {pr['changedFiles']} file{'s' * (pr['changedFiles'] != 1)}",
            age(since, now),
        ] for since, pr in sorted(groups[key], key=lambda x: x[0])]
        out.append(f"\n## {title} ({len(rows)})\n\n{blurb}\n\n")
        out.append(table(["Pull request", "Author", "Size", "Waiting"], rows))
    return "".join(out)


# ---- Roadmap --------------------------------------------------------------

MILESTONES = """query($owner: String!, $name: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    milestones(first: 50, after: $after, orderBy: {field: NUMBER, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number title url state
        open: issues(states: OPEN) { totalCount }
        closed: issues(states: CLOSED) { totalCount }
        next: issues(states: OPEN, first: %d, orderBy: {field: CREATED_AT, direction: ASC}) {
          nodes { number title url assignees(first: 1) { nodes { login } } }
        }
      }
    }
  }
}""" % (NEXT + 3)  # room to drop the dashboards themselves


def milestones():
    owner, name = REPO.split("/")
    nodes, after = [], None
    while True:
        page = graphql(MILESTONES, owner=owner, name=name, after=after)["repository"]["milestones"]
        nodes += page["nodes"]
        if not page["pageInfo"]["hasNextPage"]:
            return nodes
        after = page["pageInfo"]["endCursor"]


def bar(done, total, width=10):
    filled = round(width * done / total) if total else 0
    return "▓" * filled + "░" * (width - filled)


def roadmap_board(stones, dashboards, now):
    """dashboards maps a milestone number to the dashboard issue numbers in it,
    which never close and so stay out of its counts."""
    intro = f"""
Where each milestone stands, in the order they're planned. The
[roadmap](https://github.com/{REPO}/blob/main/ROADMAP.md) says what each one
is for. The next few open issues are listed under each; anything without a
name next to it is a place to start, so ask on the issue.
"""
    out = [header("🗺️ Roadmap status", "roadmap", now, intro)]
    shut = [m for m in stones if m["state"] == "CLOSED"]
    for m in (m for m in stones if m["state"] == "OPEN"):
        skip = dashboards.get(m["number"], set())
        open_ = m["open"]["totalCount"] - len(skip)
        done = m["closed"]["totalCount"]
        total = open_ + done
        pct = f"{100 * done // total}%" if total else "nothing filed yet"
        out.append(f"\n## [{esc(m['title'])}]({m['url']})\n\n"
                   f"`{bar(done, total)}` {pct}: {done} closed, {open_} open\n\n")
        nxt = [i for i in m["next"]["nodes"] if i["number"] not in skip][:NEXT]
        for i in nxt:
            who = [a["login"] for a in i["assignees"]["nodes"]]
            out.append(f"- [{esc(i['title'])}]({i['url']})" + (f", {handle(who[0])}" if who else "") + "\n")
        if open_ > len(nxt):
            out.append(f"- …and [{open_ - len(nxt)} more]({m['url']})\n")
    if shut:
        out.append("\n## ✅ Done\n\n")
        out += [f"- [{esc(m['title'])}]({m['url']}): {m['closed']['totalCount']} closed\n" for m in shut]
    return "".join(out)


# ---- Publishing -----------------------------------------------------------

# Who may own a dashboard issue: a maintainer, or this workflow when it
# makes one. Anyone can open an issue with a marker in it.
BOARD_OWNERS = {"github-actions[bot]"}


def existing():
    """Map each marker to the open issue that carries it, among issues a
    maintainer (or this workflow) opened."""
    found, page = {}, 1
    while True:
        issues = gh(f"repos/{REPO}/issues?state=open&per_page=100&page={page}")
        for i in issues:
            m = MARKER.match(i.get("body") or "")
            owner = maintainer(i["user"]["login"], i.get("author_association")) or i["user"]["login"] in BOARD_OWNERS
            if m and owner and "pull_request" not in i:
                found.setdefault(m.group(1), i)
        if len(issues) < 100:
            return found
        page += 1


def publish(boards, found):
    for marker, (title, body) in boards.items():
        if marker in found:
            n = found[marker]["number"]
            gh("-X", "PATCH", f"repos/{REPO}/issues/{n}", "--input", "-", payload={"body": body})
            print(f"updated #{n} ({marker})")
        else:
            made = gh("-X", "POST", f"repos/{REPO}/issues", "--input", "-", payload={"title": title, "body": body})
            print(f"created #{made['number']} ({marker})")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--dry-run", action="store_true", help="print the Markdown, change nothing")
    p.add_argument("--out", help="also write each dashboard to DIR/<marker>.md")
    args = p.parse_args()
    now = datetime.now(timezone.utc)

    found = existing()
    dashboards = {}
    for i in found.values():
        if i.get("milestone"):
            dashboards.setdefault(i["milestone"]["number"], set()).add(i["number"])

    labels = ",".join(f'"{label}"' for label, _ in KINDS)
    issues = search(f"repo:{REPO} is:issue is:open label:{labels}", HELP_FIELDS)
    core = " ".join(f"-author:{login}" for login in sorted(CORE_TEAM))
    prs = [pr for pr in search(f"repo:{REPO} is:pr is:open {core}", PR_FIELDS) if outside(pr)]

    boards = {
        "help-wanted": ("🙋 Help wanted across Parsec", help_board(issues, now)),
        "pr-triage": ("🔀 Community pull requests", pr_board(prs, now)),
        "roadmap": ("🗺️ Roadmap status", roadmap_board(milestones(), dashboards, now)),
    }
    if args.out:
        os.makedirs(args.out, exist_ok=True)
        for marker, (_, body) in boards.items():
            with open(os.path.join(args.out, f"{marker}.md"), "w", encoding="utf-8") as f:
                f.write(body)
    if args.dry_run:
        for _, body in boards.values():
            print(body, end="\n\n---\n\n")
    else:
        publish(boards, found)
    print(f"{calls} GitHub API calls", file=sys.stderr)


if __name__ == "__main__":
    main()
