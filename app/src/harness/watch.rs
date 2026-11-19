//! Local background lookups for the session header and the @ file list: git
//! branch + dirty count and the project's file list. Each request runs one
//! short-lived thread that only starts local processes (no network) and
//! reports through a channel. Nothing is logged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How often the git state of the active project is re-read.
const GIT_INTERVAL: Duration = Duration::from_secs(6);
/// How long a file list stays valid before the next @ re-reads it.
const FILES_INTERVAL: Duration = Duration::from_secs(30);
/// Upper bound for the file list (also bounds the fallback walk).
const MAX_FILES: usize = 20_000;
/// Upper bound for the directories the fallback walk may read: even a tree
/// of self-referencing entries must not keep the thread busy forever.
const MAX_WALK_DIRS: usize = 10_000;

/// Git state of a folder. `None` branch: not a git repository.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitInfo {
    pub branch: Option<String>,
    pub changed: usize,
}

/// Parse the output of `git rev-parse --abbrev-ref HEAD` and
/// `git status --porcelain`.
pub fn parse_git(branch: &str, status: &str) -> GitInfo {
    let branch = branch.trim();
    GitInfo {
        branch: (!branch.is_empty()).then(|| branch.to_string()),
        changed: status.lines().filter(|l| !l.trim().is_empty()).count(),
    }
}

/// One `git worktree list --porcelain` entry. `main` marks the repository's
/// own worktree (the first entry), which is never listed for deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
    pub main: bool,
}

/// Parse `git worktree list --porcelain` output.
pub fn parse_worktrees(out: &str) -> Vec<WorktreeInfo> {
    let mut list = Vec::new();
    let mut cur: Option<WorktreeInfo> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            if let Some(w) = cur.take() {
                list.push(w);
            }
            cur = Some(WorktreeInfo {
                path: PathBuf::from(p.trim()),
                branch: String::new(),
                main: list.is_empty(),
            });
        } else if let Some(b) = line.strip_prefix("branch ") {
            if let Some(w) = &mut cur {
                w.branch = b.trim().trim_start_matches("refs/heads/").to_string();
            }
        }
    }
    if let Some(w) = cur.take() {
        list.push(w);
    }
    list
}

enum Found {
    Git(PathBuf, GitInfo),
    Files(PathBuf, Vec<String>),
    Worktrees(PathBuf, Vec<WorktreeInfo>),
}

/// Runs the lookups and keeps their latest results.
pub struct Watcher {
    tx: mpsc::Sender<Found>,
    rx: mpsc::Receiver<Found>,
    git: HashMap<PathBuf, GitInfo>,
    git_asked: HashMap<PathBuf, Instant>,
    files: HashMap<PathBuf, Vec<String>>,
    files_asked: HashMap<PathBuf, Instant>,
    worktrees: HashMap<PathBuf, Vec<WorktreeInfo>>,
    worktrees_asked: HashMap<PathBuf, Instant>,
}

impl Default for Watcher {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            git: HashMap::new(),
            git_asked: HashMap::new(),
            files: HashMap::new(),
            files_asked: HashMap::new(),
            worktrees: HashMap::new(),
            worktrees_asked: HashMap::new(),
        }
    }
}

impl Watcher {
    /// Take finished lookups; call once per frame.
    pub fn poll(&mut self) {
        while let Ok(found) = self.rx.try_recv() {
            match found {
                Found::Git(dir, info) => {
                    self.git.insert(dir, info);
                }
                Found::Files(dir, files) => {
                    self.files.insert(dir, files);
                }
                Found::Worktrees(dir, wts) => {
                    self.worktrees.insert(dir, wts);
                }
            }
        }
    }

    /// The last known git state of `dir`; re-reads it when it is stale.
    pub fn git(&mut self, dir: &Path) -> Option<&GitInfo> {
        let stale = self
            .git_asked
            .get(dir)
            .is_none_or(|t| t.elapsed() >= GIT_INTERVAL);
        if stale {
            self.git_asked.insert(dir.to_path_buf(), Instant::now());
            let (tx, dir_owned) = (self.tx.clone(), dir.to_path_buf());
            std::thread::spawn(move || {
                let branch = git_output(&dir_owned, &["rev-parse", "--abbrev-ref", "HEAD"]);
                let status = git_output(&dir_owned, &["status", "--porcelain"]);
                let info = match (branch, status) {
                    (Some(b), Some(s)) => parse_git(&b, &s),
                    _ => GitInfo::default(),
                };
                let _ = tx.send(Found::Git(dir_owned, info));
            });
        }
        self.git.get(dir)
    }

    /// The project's worktrees (main one included); re-read in the
    /// background when stale. Empty until the first lookup has finished.
    pub fn worktrees(&mut self, dir: &Path) -> &[WorktreeInfo] {
        let stale = self
            .worktrees_asked
            .get(dir)
            .is_none_or(|t| t.elapsed() >= GIT_INTERVAL);
        if stale {
            self.worktrees_asked
                .insert(dir.to_path_buf(), Instant::now());
            let (tx, dir_owned) = (self.tx.clone(), dir.to_path_buf());
            std::thread::spawn(move || {
                let wts = git_output(&dir_owned, &["worktree", "list", "--porcelain"])
                    .map(|o| parse_worktrees(&o))
                    .unwrap_or_default();
                let _ = tx.send(Found::Worktrees(dir_owned, wts));
            });
        }
        self.worktrees.get(dir).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Forget the throttle so the next `git` call re-reads (after a turn).
    pub fn invalidate_git(&mut self, dir: &Path) {
        self.git_asked.remove(dir);
        self.worktrees_asked.remove(dir);
    }

    /// The project's relative file paths; refreshed in the background when
    /// stale. Empty until the first lookup has finished.
    pub fn files(&mut self, dir: &Path) -> &[String] {
        let stale = self
            .files_asked
            .get(dir)
            .is_none_or(|t| t.elapsed() >= FILES_INTERVAL);
        if stale {
            self.files_asked.insert(dir.to_path_buf(), Instant::now());
            let (tx, dir_owned) = (self.tx.clone(), dir.to_path_buf());
            std::thread::spawn(move || {
                let files = list_files(&dir_owned);
                let _ = tx.send(Found::Files(dir_owned, files));
            });
        }
        self.files.get(dir).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Run git in `dir` with a deadline of its own, for user-triggered work
/// that can take long (a worktree checkout of a big repository): stdout on
/// success, else the first line of git's stderr (or what went wrong).
/// Meant for a background thread; never call it on the UI thread.
pub(crate) fn run_git(dir: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("git")
        // The repository's own config must not run code on our calls.
        // Overridden here, on the command line (which beats .git/config):
        // `core.fsmonitor` — executable at scan time (verified against
        // git 2.56: `worktree add` calls it even under
        // `--no-optional-locks`) — and the hook directory, so a
        // `worktree add` cannot run the repo's post-checkout hook
        // (verified: `/dev/null` suppresses it). Not covered by this:
        // smudge/clean filters on checkout, which a working tree needs
        // to work — those stay the repo's own, and their .gitattributes
        // are the same privilege as the files they live in.
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    // both pipes are drained while waiting, so a chatty git never blocks
    fn drain<R: std::io::Read + Send + 'static>(
        pipe: Option<R>,
    ) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut out = String::new();
            if let Some(p) = pipe {
                let _ = std::io::Read::read_to_string(&mut std::io::BufReader::new(p), &mut out);
            }
            out
        })
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = stdout.join().unwrap_or_default();
                let err = stderr.join().unwrap_or_default();
                if status.success() {
                    return Ok(out);
                }
                return Err(err
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("git failed")
                    .to_string());
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git: timeout after {} s", timeout.as_secs()));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    }
}

/// A git call must never hang the watcher thread: after `GIT_TIMEOUT` the
/// child is killed and the answer counts as "none".
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// `git_output` with an explicit timeout, so the hang guard (N12) can be
/// tested with a small budget.
///
/// Why no `join`: the reader thread ends once the pipe reaches EOF, which
/// normally happens the moment the child exits. But a *grandchild* that
/// git spawned (a shell alias, a hook) can inherit the pipe and keep its
/// write end open forever — `reader.join()` without a deadline then hangs
/// the watcher thread forever. Instead of joining, the reader reports
/// through a channel and the caller waits at most until the overall
/// deadline passes; the whole call is thereby bounded by `timeout`. In the
/// normal case (no grandchild) EOF arrives right after the child exits, the
/// message lands in the channel long before the deadline, and the thread
/// ends on its own — no leak.
pub(crate) fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    git_output_bounded(dir, args, GIT_TIMEOUT)
}

fn git_output_bounded(dir: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new("git")
        // see run_git: repo config must not execute on the scans —
        // `core.fsmonitor` runs on `ls-files`/`status` despite
        // `--no-optional-locks` (verified against git 2.56). Not covered
        // here: `status` may run the repo's clean filter on worktree
        // files whose index entry looks racy-clean — that filter is part
        // of how the worktree reports itself, so the lookup lets it.
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-C")
        .arg(dir)
        // never touch the repository's index: the agent's own git commands
        // must not collide with ours over index.lock
        .arg("--no-optional-locks")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    // stdout is drained while waiting: a big `git status` must not block
    // on a full pipe until the timeout (same pipe shape as script.rs)
    let stdout = child.stdout.take()?;
    let (reader_tx, reader_rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::BufReader::new(stdout), &mut out);
        let _ = reader_tx.send(out);
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                // The child exited and the pipe should hit EOF any moment,
                // but a grandchild holding the write end keeps it open (see
                // above): the receive is bounded by the remaining budget,
                // so even that case returns instead of hanging.
                let rest = deadline.saturating_duration_since(Instant::now());
                return reader_rx.recv_timeout(rest).ok();
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Tracked and untracked-but-not-ignored files of a git repository, else a
/// bounded walk that skips hidden and build folders.
fn list_files(dir: &Path) -> Vec<String> {
    if let Some(out) = git_output(dir, &["ls-files", "-co", "--exclude-standard"]) {
        let mut files: Vec<String> = out
            .lines()
            .filter(|l| !l.is_empty())
            .take(MAX_FILES)
            .map(str::to_string)
            .collect();
        files.sort();
        return files;
    }
    walk_files(dir, MAX_WALK_DIRS)
}

/// The fallback walk behind `list_files`, with an explicit directory budget
/// so the bounds are testable small.
///
/// Hardening (N12/M3): a directory entry is checked with
/// `symlink_metadata`, so a symlink to a directory is NOT followed (it would
/// turn loops into exponential walking); such symlinks stay listable as
/// entries, like file symlinks. And the number of directories read is
/// capped: whatever the tree looks like, the walk ends after `max_dirs`.
fn walk_files(dir: &Path, max_dirs: usize) -> Vec<String> {
    let mut files = Vec::new();
    let mut visited = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if visited >= max_dirs {
            // budget spent: return what was found so far
            break;
        }
        visited += 1;
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            let path = e.path();
            // symlink_metadata does not follow the symlink: only real
            // directories are traversed
            let is_dir = std::fs::symlink_metadata(&path)
                .map(|m| m.is_dir())
                .unwrap_or(false);
            if is_dir {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(dir) {
                files.push(rel.to_string_lossy().into_owned());
                if files.len() >= MAX_FILES {
                    files.sort();
                    return files;
                }
            }
        }
    }
    files.sort();
    files
}

/// The best matches of `query` among relative paths: file-name prefix
/// first, then file name contains, then path contains, then the query's
/// characters in order (subsequence). Case-insensitive, at most `limit`.
pub fn filter_files<'a>(files: &'a [String], query: &str, limit: usize) -> Vec<&'a str> {
    let q = query.to_lowercase();
    let mut scored: Vec<(u8, usize, &str)> = Vec::new();
    for f in files {
        let lower = f.to_lowercase();
        let name = lower.rsplit('/').next().unwrap_or(&lower);
        let rank = if q.is_empty() {
            Some(4)
        } else if name.starts_with(&q) {
            Some(0)
        } else if name.contains(&q) {
            Some(1)
        } else if lower.contains(&q) {
            Some(2)
        } else if is_subsequence(&q, &lower) {
            Some(3)
        } else {
            None
        };
        if let Some(rank) = rank {
            scored.push((rank, f.len(), f));
        }
    }
    scored.sort();
    scored.into_iter().take(limit).map(|(_, _, f)| f).collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut chars = hay.chars();
    needle.chars().all(|n| chars.any(|h| h == n))
}

/// The `@`-token that ends at char index `cursor` of `text`: the text after
/// the `@`, when that `@` starts a word and no whitespace follows it.
pub fn mention_at(text: &str, cursor: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let end = cursor.min(chars.len());
    let mut i = end;
    while i > 0 {
        let c = chars[i - 1];
        if c == '@' {
            let at = i - 1;
            let starts_word = at == 0 || chars[at - 1].is_whitespace();
            return starts_word.then(|| (at, chars[i..end].iter().collect()));
        }
        if c.is_whitespace() {
            return None;
        }
        i -= 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_base(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "catwright-watch-{tag}-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A `!` alias makes git spawn a shell; the background `sleep` inherits
    /// the piped stdout and keeps its write end open, while git itself has
    /// already exited with status 0. The old `reader.join()` hung on this
    /// forever; the channel receive gives up when the budget is spent.
    #[test]
    fn git_output_returns_bounded_when_a_grandchild_holds_stdout() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let start = Instant::now();
        let out = git_output_bounded(
            dir,
            &["-c", "alias.holdstdout=!sleep 30 &", "holdstdout"],
            Duration::from_millis(300),
        );
        let elapsed = start.elapsed();
        assert_eq!(
            out, None,
            "no output can be collected while the grandchild holds the pipe"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the call is bounded instead of hanging (took {elapsed:?})"
        );
    }

    /// The child itself hanging is the already-handled timeout path: after
    /// the budget it is killed and the call returns "none", fast.
    #[test]
    fn git_output_kills_a_hanging_child_after_the_budget() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let start = Instant::now();
        let out = git_output_bounded(
            dir,
            &["-c", "alias.slow=!sleep 10", "slow"],
            Duration::from_millis(250),
        );
        assert_eq!(out, None);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "the hanging child is killed bounded (took {:?})",
            start.elapsed()
        );
    }

    /// Directory symlinks must not be followed (M3 hardening): a self-refer
    /// ring terminates, the symlinks stay listable entries, real folders
    /// are still walked.
    #[test]
    fn walk_files_terminates_on_dir_symlinks_and_keeps_them_as_entries() {
        let base = tmp_base("walk-symlinks");
        std::fs::create_dir_all(base.join("real")).unwrap();
        std::fs::write(base.join("top.txt"), "x").unwrap();
        std::fs::write(base.join("real/a.txt"), "x").unwrap();
        // self-reference and a link to the parent: both loops
        std::os::unix::fs::symlink(".", base.join("loop")).unwrap();
        std::os::unix::fs::symlink("..", base.join("up")).unwrap();
        let files = walk_files(&base, 100);
        assert!(files.contains(&"top.txt".to_string()));
        assert!(
            files.contains(&"loop".to_string()),
            "dir symlink listed, not walked"
        );
        assert!(
            files.contains(&"up".to_string()),
            "parent symlink listed, not walked"
        );
        assert!(
            files.contains(&"real/a.txt".to_string()),
            "regular folders are still walked"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The directory budget cuts artificially deep trees: the walk reads at
    /// most `max_dirs` directories and returns whatever it found by then.
    #[test]
    fn walk_files_stops_at_the_directory_budget() {
        let base = tmp_base("walk-budget");
        let mut dir = base.clone();
        for i in 0..30 {
            dir = dir.join(format!("d{i}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("f{i}.txt")), "x").unwrap();
        }
        let all = walk_files(&base, 100);
        assert_eq!(all.len(), 30, "the full chain finds every file");
        // budget 5 = root plus the first four chain folders, so exactly
        // their four files were read before the cutoff
        let capped = walk_files(&base, 5);
        assert_eq!(capped.len(), 4, "the budget stops the walk early");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn git_output_parses_branch_and_dirty_count() {
        let g = parse_git("main\n", " M a.rs\n?? b.rs\n");
        assert_eq!(g.branch.as_deref(), Some("main"));
        assert_eq!(g.changed, 2);
        assert_eq!(parse_git("", ""), GitInfo::default());
    }

    #[test]
    fn file_filter_ranks_name_matches_first() {
        let files: Vec<String> = [
            "src/app.rs",
            "docs/app_notes.md",
            "app/main.rs",
            "src/map.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let r = filter_files(&files, "app", 10);
        assert_eq!(r[0], "src/app.rs", "file-name prefix, shortest path");
        assert_eq!(r[1], "docs/app_notes.md");
        assert_eq!(r[2], "app/main.rs", "path match after name matches");
        assert_eq!(filter_files(&files, "mprs", 10), vec!["src/map.rs"]);
        assert_eq!(filter_files(&files, "", 2).len(), 2);
        assert!(filter_files(&files, "zzz", 10).is_empty());
    }

    #[test]
    fn mention_token_is_found_only_at_a_word_start() {
        assert_eq!(mention_at("@", 1), Some((0, String::new())));
        assert_eq!(mention_at("schau @src/ma", 13), Some((6, "src/ma".into())));
        assert_eq!(mention_at("mail a@b.de", 11), None, "inside a word");
        assert_eq!(
            mention_at("@foo bar", 8),
            None,
            "whitespace after the token"
        );
        assert_eq!(mention_at("@foo bar", 4), Some((0, "foo".into())));
        assert_eq!(mention_at("", 0), None);
    }

    #[test]
    fn git_runs_against_this_repository() {
        // the crate lives in a git repository during development; outside
        // of one the lookup must simply report "no branch"
        let info = {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
            match (
                git_output(dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
                git_output(dir, &["status", "--porcelain"]),
            ) {
                (Some(b), Some(s)) => parse_git(&b, &s),
                _ => GitInfo::default(),
            }
        };
        if let Some(branch) = info.branch {
            assert!(!branch.contains('\n'));
        }
    }
}
