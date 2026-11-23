#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Fetches the pages in bench/realworld.tsv into bench/corpus/realworld/,
# named by their SHA-256, for `-- realworld`. A page whose body has changed
# since the list was made isn't kept, so the benchmark leaves it out and
# says how many it left out. The pages are their sites', so the repository
# holds only their URLs, hashes and dates.
#
#   bench/fetch-realworld.sh                    # fetch what's missing, check it
#   bench/fetch-realworld.sh --urls urls.txt    # fetch every URL in Netweir's
#                                               # bench/realworld/urls.txt and
#                                               # print a new manifest
#
# It fetches as Netweir's bench/realworld/run.py does: robots.txt obeyed
# for every redirect, one request at a time to a host with a second
# between them (or the Crawl-delay, up to 10 s), sixteen hosts at once, a
# User-Agent naming the project, and a 20 s timeout. It needs Python 3.11
# or later and nothing else.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$here/corpus/realworld"
exec python3 -I - "$here" "$@" <<'PY'
import datetime, gzip, hashlib, socket, sys, threading, time, zlib
import urllib.error, urllib.parse, urllib.request, urllib.robotparser
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

UA = "parsec-realworld/1.0 (+https://github.com/parsec-html/parsec)"
TIMEOUT, MAX_BODY, GAP, MAX_DELAY = 20, 32 * 1024 * 1024, 1.0, 10.0
HTML_TYPES = ("text/html", "application/xhtml+xml")
here = Path(sys.argv[1])
args = sys.argv[2:]
urls_file = Path(args[args.index("--urls") + 1]) if "--urls" in args else None
out = here / "corpus" / "realworld"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


OPENER = urllib.request.build_opener(NoRedirect)


def to_uri(url):
    parts = urllib.parse.urlsplit(url)
    host = parts.hostname.encode("idna").decode() if parts.hostname else ""
    netloc = host + (f":{parts.port}" if parts.port else "")
    safe = "/:@!$&'()*+,;=%~-._?"
    path, query = (urllib.parse.quote(x, safe=safe) for x in (parts.path, parts.query))
    return urllib.parse.urlunsplit((parts.scheme, netloc, path, query, ""))


def get(url, accept):
    headers = {"User-Agent": UA, "Accept": accept, "Accept-Encoding": "gzip, deflate"}
    try:
        response = OPENER.open(urllib.request.Request(to_uri(url), headers=headers), timeout=TIMEOUT)
    except urllib.error.HTTPError as e:
        response = e
    with response:
        body = response.read(MAX_BODY + 1)
        headers = {k.lower(): v for k, v in response.headers.items()}
        status = response.status if hasattr(response, "status") else response.code
    if len(body) > MAX_BODY:
        raise ValueError(f"body over {MAX_BODY} bytes")
    coding = headers.get("content-encoding", "").lower()
    if coding == "gzip":
        body = gzip.decompress(body)
    elif coding == "deflate":
        try:
            body = zlib.decompress(body)
        except zlib.error:
            body = zlib.decompress(body, -zlib.MAX_WBITS)
    return status, headers, body


class Hosts:
    """robots.txt for each host, and one request at a time to each."""

    def __init__(self):
        self.lock = threading.Lock()
        self.locks = defaultdict(threading.Lock)
        self.robots, self.last = {}, {}

    def request(self, url, accept):
        host = urllib.parse.urlsplit(url).netloc.lower()
        with self.lock:
            host_lock = self.locks[host]
        with host_lock:
            if host not in self.robots:
                self.robots[host] = self.read_robots(url, host)
            rules = self.robots[host]
            if isinstance(rules, Exception):
                raise rules  # robots.txt unreachable: nothing on the host is fetched
            if not rules.can_fetch(UA, url):
                return None
            delay = min(MAX_DELAY, float(rules.crawl_delay(UA) or GAP))
            wait = self.last.get(host, 0) + delay - time.monotonic()
            if wait > 0:
                time.sleep(wait)
            try:
                return get(url, accept)
            finally:
                self.last[host] = time.monotonic()

    def read_robots(self, url, host):
        """As RFC 9309 has it: a 4xx allows everything, and no answer or a
        5xx allows nothing."""
        parts = urllib.parse.urlsplit(url)
        rules = urllib.robotparser.RobotFileParser()
        location = f"{parts.scheme}://{parts.netloc}/robots.txt"
        try:
            for _ in range(5):
                status, headers, body = get(location, "text/plain")
                if status in (301, 302, 303, 307, 308) and "location" in headers:
                    location = urllib.parse.urljoin(location, headers["location"])
                    continue
                break
        except Exception as e:
            return e
        self.last[host] = time.monotonic()
        if status >= 500 or status == 429:
            rules.disallow_all = True
        elif status >= 300:
            rules.allow_all = True
        else:
            rules.parse(body.decode("utf-8", "replace").splitlines())
        return rules


def fetch(url, hosts):
    """The body at the end of url's redirects and its Content-Type, or why not."""
    location = url
    for _ in range(6):
        got = hosts.request(location, "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5")
        if got is None:
            return "robots.txt disallows it"
        status, headers, body = got
        if status in (301, 302, 303, 307, 308) and "location" in headers:
            location = urllib.parse.urljoin(location, headers["location"])
            continue
        content_type = headers.get("content-type") or "-"
        if status != 200:
            return f"status {status}"
        if content_type.split(";")[0].strip().lower() not in HTML_TYPES and content_type != "-":
            return f"not HTML: {content_type}"
        return body, content_type.replace("\t", " ")
    return "more than 5 redirects"


if urls_file:
    wanted = [(l.split("\t")[0].strip(), None) for l in urls_file.read_text().splitlines()
              if l.strip() and not l.startswith("#")]
else:
    wanted = []
    for line in (here / "realworld.tsv").read_text().splitlines():
        if line and not line.startswith("#"):
            url, sha, *_ = line.split("\t")
            if not (out / f"{sha}.html").exists():
                wanted.append((url, sha))
by_host = defaultdict(list)
for url, sha in wanted:
    by_host[urllib.parse.urlsplit(url).netloc.lower()].append((url, sha))
hosts = Hosts()
lock = threading.Lock()
counts = defaultdict(int)
lines = []


def one_host(pages):
    for url, sha in pages:
        try:
            try:
                got = fetch(url, hosts)
            except urllib.error.URLError as e:
                # Tranco lists domains, and some answer only at www.
                parts = urllib.parse.urlsplit(url)
                if not isinstance(e.reason, socket.gaierror) or parts.netloc.startswith("www."):
                    raise
                got = fetch(f"{parts.scheme}://www.{parts.netloc}{parts.path}", hosts)
        except Exception as e:
            got = f"{type(e).__name__}: {e}"[:200]
        with lock:
            if isinstance(got, str):
                counts["not fetched"] += 1
                print(f"{url}: {got}", file=sys.stderr)
                continue
            body, content_type = got
            found = hashlib.sha256(body).hexdigest()
            if sha is None or found == sha:
                (out / f"{found}.html").write_bytes(body)
                counts["kept"] += 1
                now = datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds")
                lines.append(f"{url}\t{found}\t{now}\t{content_type}")
            else:
                counts["changed"] += 1
                print(f"{url}: changed since the list was made", file=sys.stderr)


with ThreadPoolExecutor(16) as pool:
    list(pool.map(one_host, by_host.values()))
if urls_file:
    print("\n".join(sorted(lines)))
summary = ", ".join(f"{k} {v}" for k, v in sorted(counts.items())) or "nothing to fetch"
print(f"{len(wanted)} pages to fetch: {summary}", file=sys.stderr)
PY
