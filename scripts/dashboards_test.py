# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
"""Tests for dashboards.py: escaping, claims, PR buckets and the roadmap.
No network."""

import os
import re
import sys
import unittest
from datetime import datetime, timezone

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dashboards as d  # noqa: E402


def pr(**kw):
    base = {
        "isDraft": False, "mergeable": "MERGEABLE", "createdAt": "2026-11-01T00:00:00Z",
        "author": {"__typename": "User", "login": "dev"},
        "commits": {"nodes": [{"commit": {"committedDate": "2026-11-01T00:00:00Z",
                                           "statusCheckRollup": {"state": "SUCCESS"}}}]},
        "reviews": {"nodes": []}, "comments": {"nodes": []},
    }
    base.update(kw)
    return base


def comment(login, when, role="NONE"):
    return {"author": {"login": login}, "authorAssociation": role, "createdAt": when}


class Escaping(unittest.TestCase):
    def test_handles_never_bare(self):
        out = d.esc("thanks @someone and @org/team, mail a@b.com, `@quoted`")
        self.assertIsNone(re.search(r"(?<!`)@(someone|org/team)\b", out), out)
        self.assertIn("`@someone`", out)
        self.assertIn("`@org/team`", out)
        self.assertIn("a@b.com", out)
        self.assertNotIn("``@quoted``", out)
        self.assertEqual(d.handle("dev"), "`@dev`")

    def test_table_and_markup(self):
        self.assertEqual(d.esc("a | b\n<!-- c --> #12"), "a \\| b &lt;!-- c --> `#12`")


class Buckets(unittest.TestCase):
    def test_new_pr_waits_on_us(self):
        self.assertEqual(d.bucket(pr()), ("maintainer", "2026-11-01T00:00:00Z"))

    def test_maintainer_reply_hands_turn_to_author(self):
        p = pr(comments={"nodes": [comment("jamesconwaydev", "2026-11-02T00:00:00Z", "OWNER")]})
        self.assertEqual(d.bucket(p), ("author", "2026-11-02T00:00:00Z"))

    def test_author_reply_hands_it_back(self):
        p = pr(comments={"nodes": [comment("jamesconwaydev", "2026-11-02T00:00:00Z", "OWNER"),
                                   comment("dev", "2026-11-03T00:00:00Z")]})
        self.assertEqual(d.bucket(p), ("maintainer", "2026-11-03T00:00:00Z"))

    def test_outsider_comment_does_not_count(self):
        p = pr(comments={"nodes": [comment("passerby", "2026-11-02T00:00:00Z")]})
        self.assertEqual(d.bucket(p)[0], "maintainer")

    def test_order_draft_rebase_ci(self):
        red = {"nodes": [{"commit": {"committedDate": "2026-11-01T00:00:00Z", "statusCheckRollup": {"state": "FAILURE"}}}]}
        self.assertEqual(d.bucket(pr(isDraft=True, mergeable="CONFLICTING", commits=red))[0], "draft")
        self.assertEqual(d.bucket(pr(mergeable="CONFLICTING", commits=red))[0], "rebase")
        self.assertEqual(d.bucket(pr(commits=red))[0], "ci")

    def test_outside_excludes_core_and_bots(self):
        self.assertTrue(d.outside(pr()))
        self.assertFalse(d.outside(pr(author={"__typename": "Bot", "login": "dependabot"})))
        self.assertFalse(d.outside(pr(author={"__typename": "User", "login": "jamesconwaydev"})))
        self.assertFalse(d.outside(pr(author={"__typename": "User", "login": "foothillsdigital"},
                                      authorAssociation="COLLABORATOR")))


class Claims(unittest.TestCase):
    def issue(self, assignees=(), comments=(), prs=0, role="NONE"):
        return {"assignees": {"nodes": [{"login": a} for a in assignees]},
                "comments": {"nodes": [{"author": {"login": w}, "authorAssociation": role, "body": b}
                                       for w, b in comments]},
                "closedByPullRequestsReferences": {"totalCount": prs}}

    def test_claims(self):
        self.assertIsNone(d.claim(self.issue(comments=[("dev", "Nice idea")])))
        self.assertEqual(d.claim(self.issue(comments=[("dev", "I'd like to work on this")])), "taken by `@dev`")
        self.assertIsNone(d.claim(self.issue(comments=[("jamesconwaydev", "can I take this? kidding")])))
        self.assertEqual(d.claim(self.issue(assignees=["a"])), "taken by `@a`")
        self.assertEqual(d.claim(self.issue(prs=1)), "taken (PR open)")

    def test_questions_are_not_claims(self):
        for body in ("I would like to know why this fails", "the claim in the README is wrong",
                     "I'd love to see this land"):
            self.assertIsNone(d.claim(self.issue(comments=[("dev", body)])), body)
        self.assertEqual(d.claim(self.issue(comments=[("dev", "I would love to help with this")])),
                         "taken by `@dev`")

    def test_a_maintainer_saying_so_is_not_a_community_claim(self):
        self.assertIsNone(d.claim(self.issue(comments=[("other-maintainer", "I'll take this")], role="MEMBER")))


class Existing(unittest.TestCase):
    def run_with(self, issues):
        real = d.gh
        d.gh = lambda *a, **k: issues
        try:
            return {k: v["number"] for k, v in d.existing().items()}
        finally:
            d.gh = real

    def board(self, n, login, role):
        return {"number": n, "body": "<!-- parsec-dashboard:roadmap -->\nx",
                "user": {"login": login}, "author_association": role}

    def test_a_strangers_issue_with_the_marker_is_ignored(self):
        # Newest first, as GitHub lists them.
        found = self.run_with([self.board(200, "stranger", "NONE"), self.board(181, "jamesconwaydev", "OWNER")])
        self.assertEqual(found, {"roadmap": 181})

    def test_the_workflows_own_issue_counts(self):
        self.assertEqual(self.run_with([self.board(7, "github-actions[bot]", "NONE")]), {"roadmap": 7})


def issue(title, labels=("help wanted",), assignees=()):
    return {"title": title, "url": "https://github.com/parsec-html/parsec/issues/1",
            "createdAt": "2026-11-01T00:00:00Z",
            "labels": {"nodes": [{"name": label} for label in labels]},
            "assignees": {"nodes": [{"login": a} for a in assignees]}, "comments": {"nodes": []},
            "closedByPullRequestsReferences": {"totalCount": 0}}


NOW = datetime(2026, 11, 6, tzinfo=timezone.utc)


class HelpBoard(unittest.TestCase):
    def test_first_label_wins_and_free_count(self):
        body = d.help_board([
            issue("Both labels", ("help wanted", "good first issue")),
            issue("Taken one", ("help wanted",), ["dev"]),
            issue("Unlabelled", ()),
        ], NOW)
        self.assertIn("1 of 2 are free right now.", body)
        self.assertIn("Good first issues (1)", body)
        self.assertIn("Help wanted (1)", body)
        self.assertNotIn("Unlabelled", body)

    def test_body_shrinks_under_limit(self):
        body = d.help_board([issue("x" * 400)] * 400, NOW)
        self.assertLess(len(body), d.BODY_LIMIT)
        self.assertIn("more. [See them all]", body)


def milestone(number, title, state="OPEN", open_=0, closed=0, nxt=()):
    return {"number": number, "title": title, "state": state,
            "url": f"https://github.com/parsec-html/parsec/milestone/{number}",
            "open": {"totalCount": open_}, "closed": {"totalCount": closed},
            "next": {"nodes": [{"number": n, "title": t, "url": f"https://github.com/parsec-html/parsec/issues/{n}",
                                "assignees": {"nodes": [{"login": a} for a in who]}} for n, t, who in nxt]}}


class Roadmap(unittest.TestCase):
    def test_board(self):
        body = d.roadmap_board([
            milestone(1, "M0. Foundations", "CLOSED", closed=7),
            milestone(2, "M1. Proof", open_=3, closed=2,
                      nxt=[(5, "Tokens", ()), (6, "Trees", ("dev",)), (9, "🙋 Help wanted", ())]),
            milestone(3, "0.3.0"),
        ], {2: {9}}, NOW)
        self.assertIn("50%: 2 closed, 2 open", body, "the dashboard stays out of the counts")
        self.assertNotIn("issues/9", body)
        self.assertIn("[Trees](https://github.com/parsec-html/parsec/issues/6), `@dev`", body)
        self.assertIn("`░░░░░░░░░░` nothing filed yet", body)
        self.assertIn("- [M0. Foundations](https://github.com/parsec-html/parsec/milestone/1): 7 closed", body)
        self.assertLess(body.index("M1. Proof"), body.index("0.3.0"))
        self.assertLess(body.index("0.3.0"), body.index("✅ Done"))

    def test_more_link(self):
        body = d.roadmap_board([milestone(2, "M1", open_=8, nxt=[(n, "t", ()) for n in range(8)])], {}, NOW)
        self.assertIn("…and [3 more]", body)

    def test_bar(self):
        self.assertEqual(d.bar(1, 2), "▓▓▓▓▓░░░░░")
        self.assertEqual(d.bar(0, 0), "░░░░░░░░░░")


if __name__ == "__main__":
    unittest.main()
