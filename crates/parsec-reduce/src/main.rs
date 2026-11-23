// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Shrinks a page that breaks Parsec to the smallest input that still
//! does, for a regression fixture in `crates/parsec-html/tests/regressions`:
//!
//! ```text
//! cargo run --release -p parsec-reduce -- page.html --check panic
//! ```
//!
//! `--check` says what failing means:
//!
//! - `panic`: parsing panics or aborts, with the same panic location as
//!   the page itself, so the reduction can't slide onto a different bug.
//! - `hang:MS`: parsing takes longer than MS milliseconds.
//! - `differs-from-html5ever`: Parsec's tree and html5ever's differ, as
//!   html5lib-tests tree dumps, both with scripting off.
//! - `cmd:COMMAND`: COMMAND, run by `sh -c` with the candidate file's path
//!   as `$1`, exits non-zero: the way a failing test or script reports
//!   it.
//!
//! Each candidate runs in a child process, so a panic, an abort or a
//! hang can't take the reducer down with it. The result goes to `-o
//! FILE`, or beside the input as `NAME.reduced.html`.
//!
//! The page is read as UTF-8, with anything invalid replaced.
//! shortcut: a bug in decoding bytes can't be reduced this way; reduce
//! the bytes with `cmd:` and your own decoding check if one comes up.

mod reduce;
mod tree;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

/// How long a candidate may run under a check other than `hang`, before
/// it counts as not failing the same way.
const LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq)]
enum Check {
    Panic,
    Hang(Duration),
    Differs,
    Cmd(String),
}

impl Check {
    fn parse(s: &str) -> Result<Check, String> {
        Ok(match s {
            "panic" => Check::Panic,
            "differs-from-html5ever" => Check::Differs,
            _ if s.starts_with("hang:") => {
                let ms: u64 = s[5..]
                    .parse()
                    .map_err(|_| format!("hang:MS wants milliseconds, not {:?}", &s[5..]))?;
                Check::Hang(Duration::from_millis(ms))
            }
            _ if s.starts_with("cmd:") && s.len() > 4 => Check::Cmd(s[4..].to_string()),
            _ => {
                return Err(format!(
                    "no check {s:?}: use panic, hang:MS, differs-from-html5ever or cmd:COMMAND"
                ));
            }
        })
    }
}

/// How one run of a candidate ended.
#[derive(Debug, PartialEq)]
enum Outcome {
    Exited { ok: bool, stderr: String },
    TimedOut,
}

/// Runs `command` with `input` on its stdin, killing it after `limit`.
fn run(mut command: Command, input: Option<&str>, limit: Duration) -> Outcome {
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("can't run {command:?}: {e}"));
    if let Some(input) = input {
        // A child that exits before reading it all closes the pipe; that
        // isn't an error here.
        let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    }
    let mut stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Outcome::Exited {
                ok: status.success(),
                stderr: reader.join().unwrap(),
            };
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Outcome::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Where a panic or an abort happened, from a child's stderr: the line
/// with `panicked at`, or the stack overflow message.
fn signature(stderr: &str) -> Option<String> {
    stderr.lines().find_map(|l| {
        if let Some(at) = l.find("panicked at ") {
            Some(l[at..].trim_end_matches(':').to_string())
        } else if l.contains("has overflowed its stack") {
            Some("stack overflow".to_string())
        } else {
            None
        }
    })
}

/// Decides whether a candidate still fails, remembering each answer.
struct Tester {
    check: Check,
    exe: PathBuf,
    scratch: PathBuf,
    /// The page's own panic location, which candidates must share.
    signature: Option<String>,
    seen: HashMap<String, bool>,
    runs: usize,
}

impl Tester {
    fn outcome(&mut self, candidate: &str) -> Outcome {
        self.runs += 1;
        match &self.check {
            Check::Cmd(cmd) => {
                std::fs::write(&self.scratch, candidate).unwrap();
                let mut c = Command::new("sh");
                c.arg("-c").arg(cmd).arg("sh").arg(&self.scratch);
                run(c, None, Duration::MAX)
            }
            check => {
                let mut c = Command::new(&self.exe);
                let (kind, limit) = match check {
                    Check::Hang(limit) => ("parse", *limit),
                    Check::Differs => ("differs", LIMIT),
                    _ => ("parse", LIMIT),
                };
                c.arg("--child").arg(kind);
                run(c, Some(candidate), limit)
            }
        }
    }

    fn fails(&mut self, candidate: &str) -> bool {
        if let Some(&known) = self.seen.get(candidate) {
            return known;
        }
        let outcome = self.outcome(candidate);
        let fails = match (&self.check, outcome) {
            (Check::Hang(_), o) => o == Outcome::TimedOut,
            (Check::Panic, Outcome::Exited { ok: false, stderr }) => {
                self.signature.is_none() || signature(&stderr) == self.signature
            }
            (Check::Panic, _) => false,
            // A panic there is another bug: the child exits 1 only on a
            // difference.
            (Check::Differs, Outcome::Exited { ok: false, stderr }) => signature(&stderr).is_none(),
            (Check::Differs, _) => false,
            (Check::Cmd(_), o) => matches!(o, Outcome::Exited { ok: false, .. }),
        };
        self.seen.insert(candidate.to_string(), fails);
        fails
    }
}

/// The child's side: read a candidate from stdin and parse it.
fn child(kind: &str) -> ExitCode {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    match kind {
        "parse" => {
            let _ = parsec::parse_with(&input, parsec::Options::default());
            ExitCode::SUCCESS
        }
        "differs" => {
            let (a, b) = (tree::parsec(&input), tree::html5ever(&input));
            ExitCode::from(u8::from(a != b))
        }
        _ => ExitCode::from(2),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: parsec-reduce <input.html> --check <panic|hang:MS|differs-from-html5ever|cmd:COMMAND> [-o OUT]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--child") {
        return child(args.get(1).map_or("", String::as_str));
    }
    let (mut input, mut check, mut out) = (None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--check" => check = it.next(),
            "-o" => out = it.next().map(PathBuf::from),
            "-h" | "--help" => return usage(),
            _ if input.is_none() && !a.starts_with('-') => input = Some(PathBuf::from(a)),
            _ => return usage(),
        }
    }
    let (Some(input), Some(check)) = (input, check) else {
        return usage();
    };
    let check = match Check::parse(check) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let page = match std::fs::read(&input) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            eprintln!("can't read {}: {e}", input.display());
            return ExitCode::from(2);
        }
    };
    let out = out.unwrap_or_else(|| reduced_path(&input));
    let scratch = std::env::temp_dir().join(format!("parsec-reduce-{}.html", std::process::id()));
    let mut tester = Tester {
        check,
        exe: std::env::current_exe().unwrap(),
        scratch: scratch.clone(),
        signature: None,
        seen: HashMap::new(),
        runs: 0,
    };

    // The page itself has to fail, and a panic's location is what the
    // candidates are held to.
    match tester.outcome(&page) {
        Outcome::Exited { ok: false, stderr } if tester.check == Check::Panic => {
            tester.signature = signature(&stderr);
            eprintln!(
                "{}: fails with {}",
                input.display(),
                tester.signature.as_deref().unwrap_or("a non-zero exit")
            );
        }
        _ if tester.check != Check::Panic && tester.fails(&page) => {
            eprintln!("{}: fails", input.display());
        }
        _ => {
            eprintln!(
                "{}: doesn't fail that way, so there's nothing to reduce",
                input.display()
            );
            let _ = std::fs::remove_file(&scratch);
            return ExitCode::FAILURE;
        }
    }
    tester.seen.insert(page.clone(), true);

    let start = Instant::now();
    let mut last = (String::new(), page.len());
    let reduced = reduce::reduce(&page, &mut |c| tester.fails(c), &mut |level, len| {
        if level != last.0 || len != last.1 {
            eprintln!("  {level:<10} {len:>9} bytes");
            last = (level.to_string(), len);
        }
    });
    let _ = std::fs::remove_file(&scratch);
    if let Err(e) = std::fs::write(&out, &reduced) {
        eprintln!("can't write {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!(
        "{} bytes to {} in {} runs and {:.1} s: {}",
        page.len(),
        reduced.len(),
        tester.runs,
        start.elapsed().as_secs_f64(),
        out.display()
    );
    eprintln!(
        "For a fixture, start it with: <!-- site: URL; date: YYYY-MM-DD; failed: WHAT; issue: #N -->"
    );
    ExitCode::SUCCESS
}

/// `page.html` to `page.reduced.html`, beside it.
fn reduced_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map_or("page".into(), |s| s.to_string_lossy());
    input.with_file_name(format!("{stem}.reduced.html"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_parse() {
        assert_eq!(Check::parse("panic"), Ok(Check::Panic));
        assert_eq!(
            Check::parse("hang:250"),
            Ok(Check::Hang(Duration::from_millis(250)))
        );
        assert_eq!(Check::parse("differs-from-html5ever"), Ok(Check::Differs));
        assert_eq!(
            Check::parse("cmd:grep -q x \"$1\""),
            Ok(Check::Cmd("grep -q x \"$1\"".into()))
        );
        for bad in ["hang:", "hang:soon", "cmd:", "crash", ""] {
            assert!(Check::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn signatures() {
        let s = "\nthread 'main' panicked at crates/parsec/src/tree.rs:12:5:\nboom\n";
        assert_eq!(
            signature(s).as_deref(),
            Some("panicked at crates/parsec/src/tree.rs:12:5")
        );
        assert_eq!(
            signature("thread 'main' has overflowed its stack\n").as_deref(),
            Some("stack overflow")
        );
        assert_eq!(signature("all fine"), None);
    }

    #[test]
    fn reduced_beside_the_input() {
        assert_eq!(
            reduced_path(Path::new("/tmp/x/page.html")),
            Path::new("/tmp/x/page.reduced.html")
        );
    }
}
