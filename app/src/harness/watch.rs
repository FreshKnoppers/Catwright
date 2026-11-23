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

/// Upper bound for the changed files kept per folder (the count stays
/// exact; only the list for the header's menu is cut).
const MAX_CHANGED_LISTED: usize = 500;

/// Git state of a folder. `None` branch: not a git repository.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitInfo {
    pub branch: Option<String>,
    pub changed: usize,
    /// The changed entries, at most `MAX_CHANGED_LISTED`, in git's order.
    pub files: Vec<ChangedFile>,
    /// The repository root: `files` are relative to it, not to the folder.
    pub root: Option<PathBuf>,
    /// A repository exists, but Catwright runs no git in it: the config
    /// probe failed closed (repo-scope `include.*`/`includeIf.*`/`hook.*`,
    /// a config file that is not a regular file, or no answer in time).
    pub refused: bool,
}

/// One `git status` entry: its two-letter status code (e.g. ` M`, `??`)
/// and its path relative to the repository root. An untracked folder ends
/// in `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub code: String,
    pub path: String,
}

/// Parse the output of `git rev-parse --abbrev-ref HEAD` and
/// `git status --porcelain -z` (NUL-separated, paths unquoted; a rename or
/// copy is followed by its original path, which is not an entry of its own).
pub fn parse_git(branch: &str, status: &str) -> GitInfo {
    let branch = branch.trim();
    let mut changed = 0;
    let mut files = Vec::new();
    let mut entries = status.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        let (Some(code), Some(path)) = (entry.get(..2), entry.get(3..)) else {
            continue;
        };
        if code.starts_with(['R', 'C']) {
            // the original path of a rename/copy
            entries.next();
        }
        changed += 1;
        if files.len() < MAX_CHANGED_LISTED {
            files.push(ChangedFile {
                code: code.to_string(),
                path: path.to_string(),
            });
        }
    }
    GitInfo {
        branch: (!branch.is_empty()).then(|| branch.to_string()),
        changed,
        files,
        root: None,
        refused: false,
    }
}

/// Whether `root` (git's `--show-toplevel` for `dir`) is the work tree
/// git discovered from `dir`: the nearest ancestor of `dir` (or `dir`
/// itself) that holds `.git`. A repository's `core.worktree` can point the
/// work tree anywhere (`$HOME`, `/`), and `git status` would then list and
/// scan that tree every few seconds (audit M10); such a root is refused.
fn plausible_root(dir: &Path, root: &Path) -> bool {
    let Some(discovered) = discovered_root(dir) else {
        return false;
    };
    std::fs::canonicalize(root).is_ok_and(|r| r == discovered)
}

/// The work tree git discovers from `dir`: the nearest ancestor (or `dir`
/// itself) that holds `.git`, canonical.
fn discovered_root(dir: &Path) -> Option<PathBuf> {
    let dir = std::fs::canonicalize(dir).ok()?;
    dir.ancestors()
        .find(|a| std::fs::symlink_metadata(a.join(".git")).is_ok())
        .map(Path::to_path_buf)
}

/// Read the git state of `dir` (blocking: git subprocesses).
fn read_git(dir: &Path) -> GitInfo {
    // the work tree first: a repo-chosen one elsewhere is never scanned
    let Some(root) = git_output(dir, &["rev-parse", "--show-toplevel"])
        .map(|r| PathBuf::from(r.trim()))
        .filter(|r| !r.as_os_str().is_empty() && plausible_root(dir, r))
    else {
        // no work tree of its own: a plain folder, a repo-chosen work tree
        // elsewhere (M10), or a repository the probe refuses — which the
        // UI names instead of hiding
        return GitInfo {
            refused: probe_filter_drivers(dir).is_none(),
            ..GitInfo::default()
        };
    };
    let branch = git_output(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    // `--ignore-submodules=all`: `status` would otherwise recurse into
    // submodules, whose shipped `.git/modules/*/config` can define more
    // filter drivers (hardening; verified flag against git 2.56)
    let status = git_output(
        dir,
        &["status", "--porcelain", "-z", "--ignore-submodules=all"],
    );
    match (branch, status) {
        (Some(b), Some(s)) => {
            let mut info = parse_git(&b, &s);
            info.root = Some(root);
            info
        }
        // no answer: a plain folder (the probe says "no repository") or a
        // repository git refuses, which the UI names instead of hiding
        _ => GitInfo {
            refused: probe_filter_drivers(dir).is_none(),
            ..GitInfo::default()
        },
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
    /// Only `trusted` folders are scanned: until Vibe reports the folder as
    /// trusted, running git in it is not acceptable — the folder may be a
    /// malicious download that ships its own `.git/config` (filter drivers,
    /// fsmonitor; both executable by git). Callers pass
    /// `trust.get(dir).is_some_and(|t| t.trusted)`.
    pub fn git(&mut self, dir: &Path, trusted: bool) -> Option<&GitInfo> {
        if !trusted {
            return None;
        }
        let stale = self
            .git_asked
            .get(dir)
            .is_none_or(|t| t.elapsed() >= GIT_INTERVAL);
        if stale {
            self.git_asked.insert(dir.to_path_buf(), Instant::now());
            let (tx, dir_owned) = (self.tx.clone(), dir.to_path_buf());
            std::thread::spawn(move || {
                let info = read_git(&dir_owned);
                let _ = tx.send(Found::Git(dir_owned, info));
            });
        }
        self.git.get(dir)
    }

    /// The project's worktrees (main one included); re-read in the
    /// background when stale. Empty until the first lookup has finished —
    /// and stays empty in folders that are not `trusted` (same rule as the
    /// git scan: a shipped `.git` is executable by git).
    pub fn worktrees(&mut self, dir: &Path, trusted: bool) -> &[WorktreeInfo] {
        if !trusted {
            return &[];
        }
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
    /// stale. Empty until the first lookup has finished — and stays empty in
    /// folders that are not `trusted` (same rule as the git scan).
    pub fn files(&mut self, dir: &Path, trusted: bool) -> &[String] {
        if !trusted {
            return &[];
        }
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
    // The repository's own config must not run code on our calls.
    // Overridden on the command line (which beats .git/config):
    // `core.fsmonitor` — executable at scan time (verified against
    // git 2.56: `worktree add` calls it even under
    // `--no-optional-locks`) — and the hook directory, so a `worktree add`
    // cannot run the repo's post-checkout hook (verified: `/dev/null`
    // suppresses it). Filter drivers are neutralized through env config
    // (probe_filter_drivers + filter_neutralizers; a folder shipped as
    // zip/tarball can carry its own `.git/config` or an include, and
    // `worktree add` would run its smudge filter on checkout — PoC-verified
    // against git 2.56). A failed probe fails the call closed.
    // These calls write (worktree add/remove, branch -D), so they cannot run
    // against a config snapshot (see RepoSnapshot): probe and command stay
    // two reads of the config here. They run only on the user's click,
    // not every few seconds like the status scan.
    let neutral = filter_neutralizers(
        &probe_filter_drivers(dir)
            .ok_or_else(|| "could not verify the repository's filter drivers".to_string())?,
    );
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        // no fetching, ever: a shipped partial-clone config
        // (`extensions.partialClone` + a promisor remote) would make git
        // lazily fetch missing objects, and an ssh remote would run the
        // repo's `core.sshCommand`. Command-line `-c` beats .git/config,
        // and the env var disables lazy fetch itself (git >= 2.45).
        .arg("-c")
        .arg("protocol.allow=never")
        .env("GIT_NO_LAZY_FETCH", "1")
        // beats per-protocol `protocol.<name>.allow` in the repo's config
        // (verified against git 2.56: env wins even over `-c`)
        .env("GIT_ALLOW_PROTOCOL", "none")
        // belt and braces for smudge on `worktree add` checkout (the lfs
        // process filter itself is neutralized via env config, but a
        // non-process smudge could still start it)
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());
    if !neutral.is_empty() {
        cmd.env("GIT_CONFIG_COUNT", neutral.len().to_string());
        for (i, (k, v)) in neutral.iter().enumerate() {
            cmd.env(format!("GIT_CONFIG_KEY_{i}"), k);
            cmd.env(format!("GIT_CONFIG_VALUE_{i}"), v);
        }
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
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

/// `git_output` with a custom deadline, for calls that must not stall the
/// UI thread for the full watcher budget.
pub(crate) fn git_output_timeout(dir: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    git_output_bounded(dir, args, timeout)
}

/// Default stdout cap for one git call: a scan's answer is small; anything
/// bigger (a huge blob from `git show`) counts as "none" instead of
/// becoming a huge String on the caller's thread.
const MAX_GIT_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Filter drivers that can execute code on a git scan or checkout, probed
/// from every config scope a shipped `.git` can reach: `--includes` +
/// `--show-scope` report where each key came from (a zip/tarball can ship
/// an `include.path` target — PoC-verified that `git status` then runs its
/// clean filter while a `--local`-only probe sees nothing), and every scope
/// other than system/global is the repo's own word. A repo-scope
/// `include.*`/`includeIf.*` or `hook.*` fails the probe CLOSED instead
/// (an include can point at a FIFO a live attacker rewrites between probe
/// and command; git >= 2.46 runs config-defined hooks even with
/// core.hooksPath=/dev/null), and the config files themselves must be
/// regular files. `git config --list` exits 0 in any repository, so
/// `None` here means the probe itself failed — callers fail closed.
pub fn probe_filter_drivers(dir: &Path) -> Option<Vec<String>> {
    probe_filter_drivers_timeout(dir, GIT_TIMEOUT)
}

/// Same probe with a custom deadline — the diff preview calls git on the UI
/// thread, so its budget is smaller than the watcher thread's.
fn probe_filter_drivers_timeout(dir: &Path, timeout: Duration) -> Option<Vec<String>> {
    match repo_dirs(dir, timeout)? {
        RepoDirs::NoRepo => Some(Vec::new()),
        RepoDirs::Repo { gitdir, commondir } => {
            // A repository EXISTS. Its config FILES must be plain regular
            // files: a `.git/config` that is a FIFO or a symlink would let
            // a live writer serve the probe one config and the real command
            // another. `symlink_metadata` does not follow symlinks, so a
            // symlinked config fails closed here. (A plain file can still be
            // rewritten between probe and command: commands that run filters
            // go through `Scan`, which reads a snapshot instead — audit H4.)
            if !config_files_are_regular(&gitdir, &commondir) {
                return None;
            }
            scoped_drivers(dir, timeout, None)
        }
    }
}

/// Where a folder's repository lives, as far as the probe can tell.
enum RepoDirs {
    /// `rev-parse` said "not a git repository": no repo config, no repo code.
    NoRepo,
    /// The repository's git dir and common dir (the same for a plain repo,
    /// the main `.git` for a linked worktree).
    Repo { gitdir: PathBuf, commondir: PathBuf },
}

/// Find the repository of `dir`. `None` when that cannot be told for sure
/// (callers fail closed).
fn repo_dirs(dir: &Path, timeout: Duration) -> Option<RepoDirs> {
    // Tri-state gate, fail closed on anything ambiguous. `rev-parse
    // --git-dir` exits 0 in a repository and 128 outside one — but it READS
    // the repo config, so a shipped `include.path` pointing at a FIFO blocks
    // it until the timeout (PoC-verified against git 2.56). A timeout, a
    // kill, a spawn failure or any other exit code must therefore NOT be
    // read as "not a repo" (that would run the real command unneutralized
    // while an attacker feeds the FIFO a filter driver); only a clean
    // exit 128 means "no repo, no repo config, no repo code", which keeps
    // plain calls like `git init` working outside repositories.
    let gitdir_out = match git_raw(
        dir,
        &["rev-parse".to_string(), "--git-dir".to_string()],
        timeout,
        4096,
        &[],
        None,
    ) {
        RawGit::Exit(128) => return Some(RepoDirs::NoRepo),
        RawGit::Ok(out) => out,
        _ => return None,
    };
    let gitdir = {
        let g = gitdir_out.trim();
        let p = Path::new(g);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            dir.join(p)
        }
    };
    // `commondir` must be a regular file before we read it — a FIFO
    // would block this read forever (no deadline on the UI thread)
    let commondir_file = gitdir.join("commondir");
    // (the read itself is capped and non-blocking too: a FIFO swapped in
    // after the type check must not hang it either)
    let commondir = match std::fs::symlink_metadata(&commondir_file) {
        Ok(m) if m.is_file() => match crate::store::read_regular_capped(&commondir_file, 4096) {
            crate::store::CappedRead::Bytes(b) => {
                let Ok(c) = String::from_utf8(b) else {
                    return None;
                };
                let p = Path::new(c.trim());
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    gitdir.join(p)
                }
            }
            crate::store::CappedRead::Missing => gitdir.clone(),
            _ => return None, // too large, swapped, unreadable
        },
        Ok(_) => return None,     // FIFO/symlink/... — fail closed
        Err(_) => gitdir.clone(), // no commondir: plain repo
    };
    Some(RepoDirs::Repo { gitdir, commondir })
}

/// The filter drivers of the repository's config, read through `git config`
/// (against `snapshot` when given). `None` fails the caller closed: the
/// config could not be read, or a repo scope holds an include or a hook.
fn scoped_drivers(
    dir: &Path,
    timeout: Duration,
    snapshot: Option<&RepoSnapshot>,
) -> Option<Vec<String>> {
    // A repository EXISTS: its config must be readable, else fail closed
    // (`git config --list` alone would exit 0 everywhere — it also reads
    // global/system — so rev-parse is the gate).
    match git_raw(
        dir,
        &[
            "config".to_string(),
            "--includes".to_string(),
            "--show-scope".to_string(),
            "--list".to_string(),
            "-z".to_string(),
        ],
        timeout,
        MAX_GIT_OUTPUT_BYTES,
        &[],
        snapshot,
    ) {
        RawGit::Ok(out) => {
            // A repo-scope `include.*`/`includeIf.*` can point at a FIFO or
            // a file a live attacker rewrites between our probe and the real
            // command — probing first cannot fix that race, so refuse to run
            // git here at all. A repo-scope `hook.*` is worse: git >= 2.46
            // runs config-defined hooks even with core.hooksPath=/dev/null
            // (PoC-verified against git 2.56: worktree add ran a shipped
            // hook.pwn.command with event=post-checkout).
            if repo_scoped_dangerous_key(&out) {
                return None;
            }
            Some(parse_scoped_filter_list(&out))
        }
        _ => None,
    }
}

/// Cap for one copied config file of a snapshot (a real one is a few KiB).
const MAX_SNAPSHOT_CONFIG_BYTES: u64 = 1024 * 1024;
/// Cap for the entries of a git dir mirrored into a snapshot.
const MAX_SNAPSHOT_ENTRIES: usize = 1024;

/// A private, read-once copy of a repository's config for git calls that
/// read work-tree content (audit H4). Probing `.git/config` and then
/// running git on it are two reads, and the agent can rewrite the file in
/// between: the probe sees no driver, `status` then runs a fresh
/// `filter.<x>.clean`. Here `config`, `config.worktree` and `HEAD` are
/// copied once into a 0700 folder of ours, every other entry of the git
/// dir is a symlink to the real one, and git runs with `GIT_DIR` on that
/// folder and `GIT_WORK_TREE` on the folder that holds `.git`. The probe
/// and the command then read the same bytes, whatever happens to the real
/// file. Verified against git 2.56 for plain repos and linked worktrees
/// (same `status` output; a driver added to the real config after the
/// copy never runs). Only for read-only commands: a write through the
/// symlinked folder (packed-refs, a new `worktrees/` entry) would land in
/// the copy. The folder is removed on drop; `remove_dir_all` removes
/// symlinks without following them.
struct RepoSnapshot {
    base: PathBuf,
    git_dir: PathBuf,
    work_tree: PathBuf,
}

impl RepoSnapshot {
    fn new(dir: &Path, gitdir: &Path, commondir: &Path) -> Option<Self> {
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let work_tree = discovered_root(dir)?;
        let gitdir = std::fs::canonicalize(gitdir).ok()?;
        let commondir = std::fs::canonicalize(commondir).ok()?;
        let tmp = std::env::temp_dir();
        let base = (0..16).find_map(|_| {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let p = tmp.join(format!("catwright-git-{}-{n}", std::process::id()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&p)
                .ok()
                .map(|_| p)
        })?;
        let snap = RepoSnapshot {
            git_dir: base.join("git"),
            base,
            work_tree,
        };
        // on any failure below, `snap` drops and removes what was made
        let mirror = |src: &Path, dst: &Path| -> Option<()> {
            std::fs::DirBuilder::new().mode(0o700).create(dst).ok()?;
            let entries: Vec<_> = std::fs::read_dir(src)
                .ok()?
                .take(MAX_SNAPSHOT_ENTRIES + 1)
                .collect::<Result<_, _>>()
                .ok()?;
            if entries.len() > MAX_SNAPSHOT_ENTRIES {
                return None;
            }
            for e in entries {
                let name = e.file_name();
                if matches!(
                    name.to_str(),
                    Some("config" | "config.worktree" | "commondir" | "HEAD")
                ) {
                    continue;
                }
                std::os::unix::fs::symlink(src.join(&name), dst.join(&name)).ok()?;
            }
            Some(())
        };
        // copied once: missing is fine for the config files, HEAD must be
        let copy = |src: &Path, dst: &Path, required: bool| -> Option<()> {
            match crate::store::read_regular_capped(src, MAX_SNAPSHOT_CONFIG_BYTES) {
                crate::store::CappedRead::Bytes(b) => write_private(dst, &b),
                crate::store::CappedRead::Missing if !required => Some(()),
                _ => None,
            }
        };
        mirror(&gitdir, &snap.git_dir)?;
        copy(&gitdir.join("HEAD"), &snap.git_dir.join("HEAD"), true)?;
        copy(
            &gitdir.join("config.worktree"),
            &snap.git_dir.join("config.worktree"),
            false,
        )?;
        if gitdir == commondir {
            copy(
                &commondir.join("config"),
                &snap.git_dir.join("config"),
                false,
            )?;
        } else {
            // a linked worktree: git takes the common dir from the git dir's
            // own `commondir` file (refs ignore GIT_COMMON_DIR, verified),
            // so ours names the mirrored common dir
            let common = snap.base.join("common");
            mirror(&commondir, &common)?;
            copy(&commondir.join("config"), &common.join("config"), false)?;
            let mut line = common.into_os_string().into_encoded_bytes();
            line.push(b'\n');
            write_private(&snap.git_dir.join("commondir"), &line)?;
        }
        Some(snap)
    }
}

impl Drop for RepoSnapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// Create `path` (must not exist) as a 0600 file holding `bytes`.
fn write_private(path: &Path, bytes: &[u8]) -> Option<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .ok()?;
    f.write_all(bytes).ok()
}

/// A git call that reads work-tree content (`status`): the repository's
/// filter drivers are probed from a config snapshot and the command runs
/// against the same snapshot, so a config rewritten in between is never
/// read (audit H4; see `RepoSnapshot`).
struct Scan {
    snapshot: RepoSnapshot,
    neutral: Vec<(String, String)>,
}

impl Scan {
    /// Snapshot the config and probe it. `None` outside a repository and
    /// whenever the probe fails (callers fail closed).
    fn prepare(dir: &Path, timeout: Duration) -> Option<Self> {
        let RepoDirs::Repo { gitdir, commondir } = repo_dirs(dir, timeout)? else {
            return None;
        };
        let snapshot = RepoSnapshot::new(dir, &gitdir, &commondir)?;
        let drivers = scoped_drivers(dir, timeout, Some(&snapshot))?;
        Some(Scan {
            neutral: filter_neutralizers(&drivers),
            snapshot,
        })
    }

    fn run(&self, dir: &Path, args: &[&str], timeout: Duration) -> Option<String> {
        let all: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match git_raw(
            dir,
            &all,
            timeout,
            MAX_GIT_OUTPUT_BYTES,
            &self.neutral,
            Some(&self.snapshot),
        ) {
            RawGit::Ok(s) => Some(s),
            _ => None,
        }
    }
}

/// Commands that read work-tree content and so may run filter drivers:
/// they go through `Scan` (config snapshot), never the live config.
fn reads_work_tree_content(args: &[&str]) -> bool {
    matches!(
        args.first(),
        Some(&("status" | "diff" | "diff-files" | "diff-index"))
    )
}

/// The repo's config files must be plain regular files (missing is fine —
/// no repo config then). `symlink_metadata` does not follow symlinks, so a
/// symlinked or special-file config fails closed.
fn config_files_are_regular(gitdir: &Path, commondir: &Path) -> bool {
    [
        gitdir.join("config"),
        commondir.join("config"),
        gitdir.join("config.worktree"),
    ]
    .iter()
    .all(|p| match std::fs::symlink_metadata(p) {
        Ok(m) => m.is_file(),
        Err(_) => true,
    })
}

/// True when the scoped config list contains an `include.*`/`includeIf.*`
/// or `hook.*` key from a repo scope (local/worktree/... — not system/global,
/// those are the user's own). Such keys redirect config reads or execute on
/// hook events, which makes probe-then-run racy or plain dangerous, so
/// callers fail closed.
fn repo_scoped_dangerous_key(out: &str) -> bool {
    let parts: Vec<&str> = out.split('\0').collect();
    let mut i = 0;
    while i + 1 < parts.len() {
        let scope = parts[i];
        let key_value = parts[i + 1];
        i += 2;
        if scope == "system" || scope == "global" {
            continue;
        }
        if let Some((key, _)) = key_value.split_once('\n') {
            let k = key.to_ascii_lowercase();
            if k.starts_with("include.") || k.starts_with("includeif.") || k.starts_with("hook.") {
                return true;
            }
        }
    }
    false
}

/// Parse `git config --show-scope --list -z` output (verified against
/// git 2.56): the byte stream is alternating `scope\0` and `key\nvalue\0`
/// fields. Returns the distinct `filter.<name>.*` driver names from ALL
/// scopes (the subsection may contain dots, so the subkey is the part after
/// the *last* dot of the key).
pub fn parse_scoped_filter_list(out: &str) -> Vec<String> {
    // Drivers are collected from ALL scopes, including system/global: a
    // user-global driver (typically git-lfs in ~/.gitconfig) becomes
    // repo-controlled the moment a shipped `.gitattributes` says
    // `* filter=lfs` and the shipped `.git/config` defines
    // `lfs.extension.*`/`lfs.customtransfer.*` — git-lfs reads the repo's
    // local config and does its own HTTP/ssh, so the network guards do
    // not apply. Neutralizing the driver (clean=cat, smudge=cat, empty
    // process) means git never starts the process at all. The cost is
    // cosmetic: lfs-tracked files may show as modified in our scans.
    let mut names: Vec<String> = Vec::new();
    let parts: Vec<&str> = out.split('\0').collect();
    let mut i = 0;
    while i + 1 < parts.len() {
        let key_value = parts[i + 1];
        i += 2;
        let Some((key, _value)) = key_value.split_once('\n') else {
            continue;
        };
        let Some(rest) = key.strip_prefix("filter.") else {
            continue;
        };
        let Some((name, sub)) = rest.rsplit_once('.') else {
            continue;
        };
        if matches!(sub, "clean" | "smudge" | "process" | "required")
            && !names.iter().any(|n| n == name)
        {
            names.push(name.to_string());
        }
    }
    names
}

/// Neutralizers for the probed drivers as (key, value) config pairs, passed
/// to git through `GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_n` / `GIT_CONFIG_VALUE_n`
/// env (verified against git 2.56 to beat `.git/config`, and to survive
/// driver names containing `=` or spaces, which a `-c key=value` argument
/// would misparse): `cat` replaces the configured clean/smudge command, an
/// empty `process` disables the long-running filter process, and
/// `required=false` keeps git WORKING for existing setups such as git-lfs:
/// with the real driver gone, a leftover `filter.<d>.required=true` (set by
/// `git lfs install`) makes `status` and `worktree add` fail hard with exit
/// 128 — verified against git 2.56.1 — instead of falling back to no filter.
pub fn filter_neutralizers(drivers: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for d in drivers {
        out.push((format!("filter.{d}.clean"), "cat".to_string()));
        out.push((format!("filter.{d}.smudge"), "cat".to_string()));
        out.push((format!("filter.{d}.process"), String::new()));
        out.push((format!("filter.{d}.required"), "false".to_string()));
    }
    out
}

/// `git_output` with a probe first: every call neutralizes the repo's own
/// filter drivers, because `status` executes `filter.<name>.clean` on
/// racy-clean entries and `worktree add` runs smudge filters on checkout
/// (PoC-verified against git 2.56). A failed probe fails the call closed:
/// better no answer than an unneutralized one.
fn git_output_bounded(dir: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    if reads_work_tree_content(args) {
        return Scan::prepare(dir, timeout)?.run(dir, args, timeout);
    }
    let neutral = filter_neutralizers(&probe_filter_drivers(dir)?);
    let all: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    git_raw_bounded(dir, &all, timeout, MAX_GIT_OUTPUT_BYTES, &neutral)
}

/// Deadline for git calls that run on the UI thread (the diff preview):
/// probe + read together must stay well under a frame budget.
const PREVIEW_GIT_TIMEOUT: Duration = Duration::from_secs(2);

/// The committed content of `rel` at HEAD via PLUMBING (`cat-file blob`).
/// Never use `git show HEAD:./rel` for this: when `rel` is a gitlink the
/// expression resolves to a *commit*, and `git show <commit>` applies the
/// repo's `diff.<drv>.textconv` from a shipped `.gitattributes` — PoC-
/// verified against git 2.56 that this executes the configured command
/// (RCE). `cat-file blob` applies no textconv and no filters and fails on
/// non-blobs (gitlinks), which callers treat as "no old version".
pub fn head_blob_sized(dir: &Path, rel: &str, max_bytes: usize) -> HeadBlob {
    let Some(drivers) = probe_filter_drivers_timeout(dir, PREVIEW_GIT_TIMEOUT) else {
        return HeadBlob::Refused;
    };
    let neutral = filter_neutralizers(&drivers);
    match git_raw_bounded(
        dir,
        &[
            "cat-file".to_string(),
            "blob".to_string(),
            format!("HEAD:./{rel}"),
        ],
        PREVIEW_GIT_TIMEOUT,
        max_bytes,
        &neutral,
    ) {
        Some(blob) => HeadBlob::Blob(blob),
        None => HeadBlob::Missing,
    }
}

/// The committed version of a file, as `head_blob_sized` found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadBlob {
    /// The content at HEAD.
    Blob(String),
    /// No committed version: an untracked file, no commit yet, no
    /// repository, a gitlink, or over the size cap.
    Missing,
    /// Git does not run in this repository: the config probe failed closed
    /// (see `probe_filter_drivers`), so there is no trustworthy answer.
    Refused,
}

/// Outcome of one bounded git call, so callers can tell "the command ran
/// and refused" apart from "we never got a trustworthy answer".
#[derive(Debug)]
enum RawGit {
    /// exited 0 with output within the byte cap
    Ok(String),
    /// exited cleanly with this code (128 = "not a git repository" for
    /// `rev-parse --git-dir`)
    Exit(i32),
    /// spawn/io error, over the byte cap, or killed at the deadline — no
    /// trustworthy answer, callers must fail closed
    Failed,
}

fn git_raw_bounded(
    dir: &Path,
    args: &[String],
    timeout: Duration,
    max_bytes: usize,
    neutral: &[(String, String)],
) -> Option<String> {
    match git_raw(dir, args, timeout, max_bytes, neutral, None) {
        RawGit::Ok(s) => Some(s),
        _ => None,
    }
}

fn git_raw(
    dir: &Path,
    args: &[String],
    timeout: Duration,
    max_bytes: usize,
    neutral: &[(String, String)],
    snapshot: Option<&RepoSnapshot>,
) -> RawGit {
    let mut cmd = Command::new("git");
    // a config snapshot (see RepoSnapshot): git reads the copied config and
    // the work tree we found, never the live `.git/config`
    if let Some(snap) = snapshot {
        cmd.env("GIT_DIR", &snap.git_dir)
            .env("GIT_WORK_TREE", &snap.work_tree)
            .env_remove("GIT_COMMON_DIR");
    }
    cmd.arg("-c")
        .arg("core.fsmonitor=false")
        // hooksPath like run_git: no current caller starts hooks, but
        // `git_output` is pub(crate) — belt and braces for future callers
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        // no fetching, ever: a shipped partial-clone config
        // (`extensions.partialClone` + a promisor remote) would make git
        // lazily fetch missing objects, and an ssh remote would run the
        // repo's `core.sshCommand`. Command-line `-c` beats .git/config,
        // and the env var disables lazy fetch itself (git >= 2.45).
        .arg("-c")
        .arg("protocol.allow=never")
        .env("GIT_NO_LAZY_FETCH", "1")
        // beats per-protocol `protocol.<name>.allow` in the repo's config
        // (verified against git 2.56: env wins even over `-c`)
        .env("GIT_ALLOW_PROTOCOL", "none")
        // belt and braces for smudge filters (the drivers themselves are
        // neutralized via env config, so e.g. git-lfs never starts)
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .arg("-C")
        .arg(dir)
        // never touch the repository's index: the agent's own git commands
        // must not collide with ours over index.lock
        .arg("--no-optional-locks")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    // the repo's filter drivers are neutralized through env config (see
    // filter_neutralizers) — set before spawn
    if !neutral.is_empty() {
        cmd.env("GIT_CONFIG_COUNT", neutral.len().to_string());
        for (i, (k, v)) in neutral.iter().enumerate() {
            cmd.env(format!("GIT_CONFIG_KEY_{i}"), k);
            cmd.env(format!("GIT_CONFIG_VALUE_{i}"), v);
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return RawGit::Failed,
    };
    // stdout is drained while waiting: a big `git status` must not block
    // on a full pipe until the timeout (same pipe shape as script.rs). The
    // read is capped at `max_bytes + 1` BYTES (not chars: read_to_string
    // would fail on non-UTF-8 output and then the cap check never fires);
    // one byte over the cap proves truncation, and then the child (blocked
    // on a full pipe) is killed at once instead of running until the
    // deadline.
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return RawGit::Failed,
    };
    let (reader_tx, reader_rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut reader = std::io::BufReader::new(stdout);
        let mut limited = std::io::Read::take(&mut reader, max_bytes as u64 + 1);
        let _ = std::io::Read::read_to_end(&mut limited, &mut out);
        let _ = reader_tx.send(out);
    });
    let deadline = Instant::now() + timeout;
    let mut received: Option<Vec<u8>> = None;
    loop {
        if received.is_none() {
            // The child exited and the pipe should hit EOF any moment, but
            // a grandchild holding the write end keeps it open (see the
            // grandchild test): the receive is bounded by the remaining
            // budget, so even that case returns instead of hanging.
            let rest = deadline.saturating_duration_since(Instant::now());
            if rest.is_zero() {
                break;
            }
            match reader_rx.recv_timeout(rest) {
                Ok(s) => received = Some(s),
                Err(_) => break,
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = received.take().unwrap_or_default();
                if out.len() > max_bytes {
                    return RawGit::Failed;
                }
                return match status.code() {
                    Some(0) => RawGit::Ok(String::from_utf8_lossy(&out).into_owned()),
                    Some(code) => RawGit::Exit(code),
                    None => RawGit::Failed, // killed by a signal
                };
            }
            Ok(None) => {
                if received.as_ref().is_some_and(|s| s.len() > max_bytes) {
                    break; // reader hit the cap, child still writing: kill now
                }
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    RawGit::Failed
}

/// Tracked and untracked-but-not-ignored files of a git repository, else a
/// bounded walk that skips hidden and build folders.
fn list_files(dir: &Path) -> Vec<String> {
    // git only for the work tree git discovered here (see plausible_root)
    let own_tree = git_output(dir, &["rev-parse", "--show-toplevel"])
        .is_some_and(|r| plausible_root(dir, Path::new(r.trim())));
    if let Some(out) = own_tree
        .then(|| git_output(dir, &["ls-files", "-co", "--exclude-standard"]))
        .flatten()
    {
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

    /// What `probe_filter_drivers` must report for a repository whose own
    /// config defines `repo_own`: the probe neutralizes drivers from ALL
    /// scopes, so the system/global drivers of the machine running the
    /// tests come first (the GitHub runner and many desktops ship git-lfs
    /// in /etc/gitconfig or ~/.gitconfig), then the repo's own ones, each
    /// name once — the parser's order. Read outside any repository, with
    /// the same `--includes` as the probe, so only system/global count.
    fn expected_drivers(repo_own: &[&str]) -> Vec<String> {
        let outside = tmp_base("no-repo");
        let out = std::process::Command::new("git")
            .current_dir(&outside)
            // discovery must not walk up into a repository around TMPDIR
            .env("GIT_CEILING_DIRECTORIES", outside.parent().unwrap())
            .args(["config", "--includes", "--show-scope", "--list", "-z"])
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&outside);
        let mut names = parse_scoped_filter_list(&String::from_utf8_lossy(&out.stdout));
        for own in repo_own {
            if !names.iter().any(|n| n == own) {
                names.push(own.to_string());
            }
        }
        names
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
        let g = parse_git("main\n", " M a.rs\0?? b.rs\0");
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

    /// The header's menu lists the changed paths: unquoted (spaces stay),
    /// a rename counts once under its new path, folders keep their `/`.
    #[test]
    fn git_status_lists_the_changed_paths() {
        let g = parse_git(
            "main",
            " M src/a b.rs\0R  new.rs\0old.rs\0?? notes/\0A  added.rs\0",
        );
        assert_eq!(g.changed, 4);
        let listed: Vec<(&str, &str)> = g
            .files
            .iter()
            .map(|f| (f.code.as_str(), f.path.as_str()))
            .collect();
        assert_eq!(
            listed,
            vec![
                (" M", "src/a b.rs"),
                ("R ", "new.rs"),
                ("??", "notes/"),
                ("A ", "added.rs"),
            ]
        );
    }

    #[test]
    fn git_runs_against_this_repository() {
        // the crate lives in a git repository during development; outside
        // of one the lookup must simply report "no branch"
        let info = read_git(Path::new(env!("CARGO_MANIFEST_DIR")));
        if let Some(branch) = info.branch {
            assert!(!branch.contains('\n'));
            // the changed paths are relative to the root, which holds the crate
            let root = info.root.expect("a repository has a root");
            assert!(Path::new(env!("CARGO_MANIFEST_DIR")).starts_with(&root));
        }
    }

    #[test]
    fn parse_scoped_filter_list_reads_names_with_dots_from_all_scopes() {
        // `git config --show-scope --list -z` (git 2.56) emits alternating
        // `scope\0` and `key\nvalue\0` fields. ALL scopes count — a
        // user-global driver (git-lfs) is repo-controlled via a shipped
        // `.gitattributes` + repo-local `lfs.*` keys.
        let out = "global\0pull.rebase\nfalse\0\
                   local\0filter.pwn.clean\nsh -c evil.sh --flag\0\
                   local\0filter.pwn.smudge\ncat\0\
                   local\0filter.a.b.clean\ncat\0\
                   local\0core.fsmonitor\ntrue\0\
                   system\0filter.sys.clean\nevil\0\
                   worktree\0filter.wt.process\nevil\0\
                   global\0filter.lfs.process\ngit-lfs filter-process\0";
        assert_eq!(
            parse_scoped_filter_list(out),
            vec!["pwn", "a.b", "sys", "wt", "lfs"],
            "drivers from every scope are neutralized"
        );
        assert!(parse_scoped_filter_list("").is_empty());
    }

    #[test]
    fn filter_neutralizers_are_env_config_pairs_covering_all_four_keys() {
        let args = filter_neutralizers(&["pwn".to_string(), "x=y".to_string()]);
        assert_eq!(
            args,
            vec![
                ("filter.pwn.clean".to_string(), "cat".to_string()),
                ("filter.pwn.smudge".to_string(), "cat".to_string()),
                ("filter.pwn.process".to_string(), String::new()),
                ("filter.pwn.required".to_string(), "false".to_string()),
                // a driver name with '=' must survive as an env KEY
                ("filter.x=y.clean".to_string(), "cat".to_string()),
                ("filter.x=y.smudge".to_string(), "cat".to_string()),
                ("filter.x=y.process".to_string(), String::new()),
                ("filter.x=y.required".to_string(), "false".to_string()),
            ]
        );
    }

    /// PoC regression: a repo that ships its own `.git/config` with a clean
    /// filter must not execute it when we scan — `git status` would run it
    /// on racy-clean entries, and `--no-optional-locks` keeps the entries
    /// racy forever. The filter writes a marker file; the scan must not.
    #[test]
    fn status_scan_does_not_run_a_shipped_clean_filter() {
        let base = tmp_base("filter-poc");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        git(&["init", "-q", "."]);
        let marker = base.join("marker");
        git(&[
            "config",
            "--local",
            "filter.pwn.clean",
            &format!("sh -c 'echo PWNED > {}'", marker.display()),
        ]);
        git(&["config", "--local", "filter.pwn.required", "false"]);
        std::fs::write(repo.join(".gitattributes"), "* filter=pwn\n").unwrap();
        std::fs::write(repo.join("file.txt"), "hello\n").unwrap();
        git(&["add", ".gitattributes", "file.txt"]);
        // `git add` itself runs the clean filter — drop its marker, the
        // assertion below is about OUR scan only
        let _ = std::fs::remove_file(&marker);
        // entries added just now are racy-clean (entry mtime == index mtime),
        // so `status` re-checks their content through the clean filter; with
        // `--no-optional-locks` the refreshed stat is never written back and
        // they stay racy forever

        // the probe sees the driver …
        let drivers = probe_filter_drivers(&repo).expect("probe succeeds in a repo");
        assert_eq!(drivers, expected_drivers(&["pwn"]));
        // … and our scan runs `status` with the filter neutralized
        let out = git_output(&repo, &["status", "--porcelain", "-z"]);
        assert!(out.is_some(), "status still answers");
        assert!(
            !marker.exists(),
            "the shipped clean filter must not have run (marker exists)"
        );
        // sanity: without the neutralizer the filter WOULD run (the PoC)
        let _ = std::fs::remove_file(&marker);
        git(&["status", "--porcelain"]);
        assert!(
            marker.exists(),
            "PoC premise: plain git status runs the filter"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Audit H4: probe and command were two reads of `.git/config`. The
    /// agent rewrites the file in between — the probe saw no driver, then
    /// `status` ran the new clean filter. The scan now reads a snapshot:
    /// the race is played deterministically by defining the driver right
    /// after `Scan::prepare`, and it never runs. The old flow (probe, then
    /// git on the live config) runs it — the premise.
    #[test]
    fn a_config_rewritten_after_the_probe_is_never_read() {
        for linked in [false, true] {
            let base = tmp_base(if linked { "h4-wt" } else { "h4" });
            let repo = base.join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let git = |dir: &Path, args: &[&str]| {
                let out = std::process::Command::new("git")
                    .arg("-C")
                    .arg(dir)
                    .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                    .args(args)
                    .output()
                    .unwrap();
                assert!(out.status.success(), "git {args:?}");
            };
            git(&repo, &["init", "-q", "."]);
            // the attributes ask for a driver nobody defines (yet)
            std::fs::write(repo.join(".gitattributes"), "* filter=pwn\n").unwrap();
            std::fs::write(repo.join("file.txt"), "hello\n").unwrap();
            git(&repo, &["add", "."]);
            git(&repo, &["commit", "-q", "-m", "c"]);
            let work = if linked {
                let wt = base.join("wt");
                git(
                    &repo,
                    &["worktree", "add", "-q", "-b", "wt", wt.to_str().unwrap()],
                );
                wt
            } else {
                repo.clone()
            };
            // racy-clean entries: `status` re-hashes them through the filter
            std::fs::write(work.join("file.txt"), "hello\n").unwrap();
            git(&work, &["add", "file.txt"]);
            let marker = base.join("marker");

            let scan = Scan::prepare(&work, GIT_TIMEOUT).expect("scan in a repo");
            assert!(scan.neutral.is_empty(), "the probe saw no driver");
            // … the attacker now defines it in the live config
            git(
                &repo,
                &[
                    "config",
                    "filter.pwn.clean",
                    &format!("sh -c 'echo PWNED > {}'", marker.display()),
                ],
            );
            let out = scan.run(&work, &["status", "--porcelain", "-z"], GIT_TIMEOUT);
            assert!(out.is_some(), "status still answers");
            assert!(
                !marker.exists(),
                "the late driver must not run (linked: {linked})"
            );
            let snapshot_dir = scan.snapshot.base.clone();
            drop(scan);
            assert!(
                !snapshot_dir.exists(),
                "the snapshot is removed after the call"
            );

            // premise: the old flow (probe result, then git on the live
            // config) runs the driver the probe never saw
            let status = ["status".to_string(), "--porcelain".to_string()];
            let _ = git_raw(&work, &status, GIT_TIMEOUT, MAX_GIT_OUTPUT_BYTES, &[], None);
            assert!(marker.exists(), "PoC premise (linked: {linked})");
            let _ = std::fs::remove_dir_all(&base);
        }
    }

    /// The snapshot is invisible in the answer: `status` and the branch
    /// read through it equal plain git, in a plain repo, a subfolder and a
    /// linked worktree, and nothing is written into the snapshot.
    #[test]
    fn a_status_scan_answers_like_plain_git() {
        let base = tmp_base("h4-same");
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        let git = |dir: &Path, args: &[&str]| -> String {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        git(&repo, &["init", "-q", "."]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        std::fs::write(repo.join("sub/b.txt"), "b\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "c"]);
        let wt = base.join("wt");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
        );
        for (dir, edit) in [
            (repo.clone(), repo.join("a.txt")),
            (repo.join("sub"), repo.join("sub/b.txt")),
            (wt.clone(), wt.join("a.txt")),
        ] {
            std::fs::write(&edit, "changed\n").unwrap();
            std::fs::write(dir.join("new.txt"), "n\n").unwrap();
            let args = ["status", "--porcelain", "-z", "--ignore-submodules=all"];
            let mut plain = vec!["--no-optional-locks"];
            plain.extend(args);
            let scan = Scan::prepare(&dir, GIT_TIMEOUT).expect("scan");
            let expected_status = git(&dir, &plain);
            assert_eq!(
                scan.run(&dir, &args, GIT_TIMEOUT),
                Some(expected_status.clone()),
                "{dir:?}"
            );
            assert_eq!(
                scan.run(&dir, &["rev-parse", "--abbrev-ref", "HEAD"], GIT_TIMEOUT),
                Some(git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"])),
                "{dir:?}"
            );
            // only our copies are real files in the snapshot
            let mut written: Vec<String> = Vec::new();
            for sub in ["git", "common"] {
                if let Ok(rd) = std::fs::read_dir(scan.snapshot.base.join(sub)) {
                    for e in rd.flatten() {
                        if !e.file_type().unwrap().is_symlink() {
                            written.push(format!("{sub}/{}", e.file_name().to_string_lossy()));
                        }
                    }
                }
            }
            written.sort();
            let expected: &[&str] = if dir == wt {
                &["common/config", "git/HEAD", "git/commondir"]
            } else {
                &["git/HEAD", "git/config"]
            };
            assert_eq!(written, expected, "{dir:?}");
            // read_git goes through the scan as well
            let entries = expected_status
                .split('\0')
                .filter(|e| !e.is_empty())
                .count();
            assert_eq!(read_git(&dir).files.len(), entries, "{dir:?}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// PoC variant: the driver comes from an `include.path` target shipped
    /// inside the tree — a `--local`-only probe would miss it, and plain
    /// `git status` executes it (verified against git 2.56). Because an
    /// include can also point at a FIFO that a live attacker rewrites
    /// between probe and command, a repo-scope include now fails the
    /// probe CLOSED: no git scan runs there at all.
    #[test]
    fn repo_scope_include_fails_the_probe_closed() {
        let base = tmp_base("filter-include");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success());
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        git(&["init", "-q", "."]);
        let marker = base.join("marker");
        std::fs::write(
            repo.join("evil.conf"),
            format!(
                "[filter \"inc\"]\nclean = sh -c 'echo INC > {}'\nrequired = false\n",
                marker.display()
            ),
        )
        .unwrap();
        git(&["config", "--local", "include.path", "../evil.conf"]);
        std::fs::write(repo.join(".gitattributes"), "* filter=inc\n").unwrap();
        std::fs::write(repo.join("f.txt"), "hi\n").unwrap();
        git(&["add", ".gitattributes", "f.txt"]);
        let _ = std::fs::remove_file(&marker); // git add ran the filter already

        // sanity: plain git status WOULD run the included filter (the PoC)
        let plain = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        let _ = plain.status;
        assert!(
            marker.exists(),
            "PoC premise: plain git status runs the included filter"
        );
        let _ = std::fs::remove_file(&marker);

        // our probe refuses the repo outright: include => fail closed
        assert_eq!(
            probe_filter_drivers(&repo),
            None,
            "a repo-scope include must fail the probe closed"
        );
        assert!(
            git_output(&repo, &["status", "--porcelain", "-z"]).is_none(),
            "no git scan may run in a repo with a repo-scope include"
        );
        assert!(!marker.exists(), "the included filter must never run");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// PoC variant: a driver NAME containing `=` breaks `-c key=value`
    /// overrides (git splits at the first `=`), so neutralizers go through
    /// GIT_CONFIG_* env (verified against git 2.56).
    #[test]
    fn status_scan_neutralizes_a_driver_name_with_equals() {
        let base = tmp_base("filter-eq");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success());
        };
        git(&["init", "-q", "."]);
        let marker = base.join("marker");
        git(&[
            "config",
            "--local",
            "filter.x=y.clean",
            &format!("sh -c 'echo EQ > {}'", marker.display()),
        ]);
        git(&["config", "--local", "filter.x=y.required", "false"]);
        std::fs::write(repo.join(".gitattributes"), "* filter=x=y\n").unwrap();
        std::fs::write(repo.join("f.txt"), "hi\n").unwrap();
        git(&["add", ".gitattributes", "f.txt"]);
        let _ = std::fs::remove_file(&marker);

        let drivers = probe_filter_drivers(&repo).expect("probe succeeds");
        assert_eq!(drivers, expected_drivers(&["x=y"]));
        assert!(git_output(&repo, &["status", "--porcelain", "-z"]).is_some());
        assert!(!marker.exists(), "the '='-named driver must not have run");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Outside a repository there is no repo config to neutralize, so
    /// plain calls (`git init` & co.) must keep working; inside a
    /// repository whose config cannot be read, the probe fails closed.
    #[test]
    fn probe_semantics_outside_and_inside_a_repo() {
        let base = tmp_base("probe-semantics");
        let not_a_repo = base.join("plain");
        std::fs::create_dir_all(&not_a_repo).unwrap();
        assert_eq!(probe_filter_drivers(&not_a_repo), Some(vec![]));
        assert!(git_output(&not_a_repo, &["init", "-q"]).is_some());
        // now it is a repo: the probe reads its config (no repo filters
        // here, only what system/global config defines)
        assert_eq!(
            probe_filter_drivers(&not_a_repo),
            Some(expected_drivers(&[]))
        );
        assert!(git_output(&not_a_repo, &["status", "--porcelain"]).is_some());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// `head_blob_sized` must read the HEAD blob through plumbing
    /// (`cat-file blob`), never `git show`: on a gitlink `HEAD:./sub`
    /// resolves to a commit and `git show` applies the repo's textconv —
    /// PoC-verified against git 2.56 that this executes the configured
    /// command. `cat-file blob` applies no textconv and fails on gitlinks.
    #[test]
    fn head_blob_uses_plumbing_never_textconv() {
        let base = tmp_base("textconv-poc");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let marker = base.join("pwned");
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .arg("-c")
                .arg("user.email=t@t")
                .arg("-c")
                .arg("user.name=t")
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
            out
        };
        git(&["init", "-q", "."]);
        // a textconv driver that would prove execution
        git(&[
            "config",
            "diff.x.textconv",
            &format!("touch {}", marker.to_string_lossy()),
        ]);
        std::fs::write(repo.join(".gitattributes"), "* diff=x\n").unwrap();
        std::fs::write(repo.join("f.txt"), "hello\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "one"]);
        // a gitlink whose commit object ships in the same object store
        let sha = String::from_utf8(git(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string();
        git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{sha},sub"),
        ]);
        git(&["commit", "-qm", "gitlink"]);

        // PoC premise: `git show HEAD:./sub` DOES run the textconv (RCE)
        let _ = std::fs::remove_file(&marker);
        let show = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["--no-pager", "show", "HEAD:./sub"])
            .output()
            .unwrap();
        let _ = show.status;
        assert!(
            marker.exists(),
            "PoC premise: plain git show on the gitlink runs the textconv"
        );

        // our path must not: blob content for a normal file, None for the
        // gitlink, and no textconv execution either way
        let _ = std::fs::remove_file(&marker);
        assert_eq!(
            head_blob_sized(&repo, "f.txt", 1024),
            HeadBlob::Blob("hello\n".to_string()),
            "blob at HEAD"
        );
        assert!(!marker.exists(), "cat-file must not run textconv");
        assert_eq!(
            head_blob_sized(&repo, "sub", 1024),
            HeadBlob::Missing,
            "gitlink is not a blob"
        );
        assert!(
            !marker.exists(),
            "cat-file must not run textconv on gitlinks"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The probe gate is tri-state: a clean exit 128 means "not a repo"
    /// (plain calls keep working), a timeout on a blocked `rev-parse`
    /// (FIFO include, PoC-verified to block git 2.56) must FAIL CLOSED,
    /// not be misread as "not a repo" — else the real command would run
    /// unneutralized while an attacker feeds the FIFO a filter driver.
    #[test]
    fn git_raw_tri_state_not_repo_vs_blocked_vs_ok() {
        let base = tmp_base("tri-state");
        let plain = base.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        // not a repo: rev-parse exits 128
        match git_raw(
            &plain,
            &["rev-parse".to_string(), "--git-dir".to_string()],
            Duration::from_millis(2000),
            4096,
            &[],
            None,
        ) {
            RawGit::Exit(128) => {}
            other => panic!("expected Exit(128) outside a repo, got {other:?}"),
        }
        // repo with a FIFO include and a writer that never sends: the
        // probe's rev-parse blocks until the deadline -> Failed
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("-c")
            .arg("user.email=t@t")
            .arg("-c")
            .arg("user.name=t")
            .args(["init", "-q", "."])
            .output()
            .unwrap();
        assert!(out.status.success());
        let fifo = repo.join(".git").join("fifo");
        #[cfg(unix)]
        {
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap();
            std::fs::write(
                repo.join(".git").join("config"),
                format!(
                    "[include]\n\tpath = ./fifo\n{}",
                    std::fs::read_to_string(repo.join(".git").join("config")).unwrap_or_default()
                ),
            )
            .unwrap();
            // hold the write end open without sending anything
            let fifo2 = fifo.clone();
            let holder = std::thread::spawn(move || {
                let _f = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&fifo2)
                    .map(|mut f| {
                        use std::io::Write;
                        let _ = f.write_all(b"");
                        std::thread::sleep(Duration::from_secs(10));
                    });
            });
            let r = git_raw(
                &repo,
                &["rev-parse".to_string(), "--git-dir".to_string()],
                Duration::from_millis(300),
                4096,
                &[],
                None,
            );
            assert!(
                matches!(r, RawGit::Failed),
                "blocked rev-parse must be Failed (fail closed), got {r:?}"
            );
            // and so the probe fails closed instead of "no repo"
            let probe = probe_filter_drivers(&repo);
            assert_eq!(probe, None, "probe must fail closed on a blocked config");
            drop(holder);
        }
        // a healthy repo: Ok
        let ok = base.join("ok");
        std::fs::create_dir_all(&ok).unwrap();
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&ok)
            .args(["init", "-q", "."])
            .output()
            .unwrap();
        assert!(out.status.success());
        match git_raw(
            &ok,
            &["rev-parse".to_string(), "--git-dir".to_string()],
            Duration::from_millis(2000),
            4096,
            &[],
            None,
        ) {
            RawGit::Ok(_) => {}
            other => panic!("expected Ok in a repo, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// `include.*`/`includeIf.*`/`hook.*` from a repo scope fails the probe
    /// closed; from system/global (the user's own) it must not.
    #[test]
    fn repo_scoped_dangerous_key_detection() {
        // scope\0 key\nvalue\0 pairs, as `git config --show-scope --list -z` emits
        let with_include = "local\0include.path\n../evil.conf\0worktree\0filter.x.clean\ncat\0";
        assert!(repo_scoped_dangerous_key(with_include));
        let with_include_if = "local\0includeIf.gitdir:/x/.path\ny\0";
        assert!(repo_scoped_dangerous_key(with_include_if));
        let capitalized = "local\0Include.Path\n../evil.conf\0";
        assert!(repo_scoped_dangerous_key(capitalized));
        // config-defined hooks run even with core.hooksPath=/dev/null (PoC)
        let with_hook =
            "local\0hook.pwn.command\ntouch /tmp/x\0local\0hook.pwn.event\npost-checkout\0";
        assert!(repo_scoped_dangerous_key(with_hook));
        let user_scopes_only =
            "global\0include.path\n~/ok.conf\0system\0hook.pwn.command\nx\0local\0filter.x.clean\ncat\0";
        assert!(!repo_scoped_dangerous_key(user_scopes_only));
        assert!(!repo_scoped_dangerous_key(""));
    }

    /// A `.git/config` that is a symlink must fail the probe closed: a live
    /// writer could serve the probe one config and the real command another.
    #[test]
    fn symlinked_git_config_fails_the_probe_closed() {
        let base = tmp_base("symlink-config");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success());
        };
        git(&["init", "-q", "."]);
        assert!(
            probe_filter_drivers(&repo).is_some(),
            "regular config is fine"
        );
        // replace .git/config with a symlink to a regular file
        let real = base.join("real-config");
        std::fs::copy(repo.join(".git").join("config"), &real).unwrap();
        std::fs::remove_file(repo.join(".git").join("config")).unwrap();
        std::os::unix::fs::symlink(&real, repo.join(".git").join("config")).unwrap();
        assert_eq!(
            probe_filter_drivers(&repo),
            None,
            "a symlinked .git/config must fail closed"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A `commondir` file that is not a regular file (FIFO) must fail the
    /// probe closed immediately — reading it would block forever (the read
    /// has no deadline and can happen on the UI thread).
    #[test]
    fn fifo_commondir_fails_the_probe_closed_without_blocking() {
        let base = tmp_base("fifo-commondir");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["init", "-q", "."])
            .output()
            .unwrap();
        assert!(out.status.success());
        std::process::Command::new("mkfifo")
            .arg(repo.join(".git").join("commondir"))
            .status()
            .unwrap();
        // git's own rev-parse reads the FIFO commondir and blocks until
        // the probe deadline (5 s) — bounded, then fail closed; our own
        // read never happens (regular-file check first)
        let start = std::time::Instant::now();
        assert_eq!(probe_filter_drivers(&repo), None);
        assert!(
            start.elapsed() < GIT_TIMEOUT + Duration::from_secs(3),
            "the probe must return (fail closed) instead of hanging forever"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// PoC: git >= 2.46 runs config-defined hooks (`hook.<name>.command` +
    /// `hook.<name>.event`) even with `core.hooksPath=/dev/null` — verified
    /// against git 2.56 that `worktree add` executes a shipped
    /// post-checkout hook. A repo-scope `hook.*` must therefore fail the
    /// probe closed, so no git command runs there.
    #[test]
    fn repo_scope_config_hook_fails_the_probe_closed() {
        let base = tmp_base("hook-poc");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .arg("-c")
                .arg("user.email=t@t")
                .arg("-c")
                .arg("user.name=t")
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q", "."]);
        let marker = base.join("hook-ran");
        git(&[
            "config",
            "hook.pwn.command",
            &format!("touch {}", marker.display()),
        ]);
        git(&["config", "hook.pwn.event", "post-checkout"]);
        std::fs::write(repo.join("f"), "x").unwrap();
        git(&["add", "f"]);
        git(&["commit", "-qm", "one"]);

        // premise: plain worktree add DOES run the config hook
        let _ = std::fs::remove_file(&marker);
        let wt = base.join("wt");
        let plain = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("-c")
            .arg("core.hooksPath=/dev/null")
            .args(["worktree", "add", &wt.to_string_lossy(), "HEAD"])
            .output()
            .unwrap();
        let _ = plain.status;
        // Config-defined hooks only exist in newer git: 2.56 runs them
        // (PoC-verified), while older releases such as 2.43 (Ubuntu 24.04)
        // and 2.47 do not run them at all, so there is no attack to show.
        // The premise is required from 2.56 on; the fail-closed checks below
        // run on every version.
        let version = std::process::Command::new("git")
            .arg("--version")
            .output()
            .ok()
            .and_then(|o| {
                let out = String::from_utf8_lossy(&o.stdout).into_owned();
                crate::harness::compat::Version::parse(
                    out.trim().trim_start_matches("git version "),
                )
            });
        let ran = marker.exists();
        if version.is_some_and(|v| v >= crate::harness::compat::Version(2, 56, 0)) {
            assert!(
                ran,
                "PoC premise: worktree add runs the config-defined hook even with hooksPath=/dev/null"
            );
        } else if !ran {
            eprintln!("git {version:?} runs no config-defined hooks: PoC premise not checked");
        }
        let _ = std::fs::remove_file(&marker);
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "remove", "--force", &wt.to_string_lossy()])
            .status();

        // our probe refuses the repo: no git command of ours may run there
        assert_eq!(
            probe_filter_drivers(&repo),
            None,
            "repo-scope hook.* must fail closed"
        );
        assert!(git_output(&repo, &["status", "--porcelain"]).is_none());
        assert!(
            !marker.exists(),
            "the config hook must never run via our calls"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The watcher must not run any git scan in a folder that is not
    /// trusted, and must start scanning once it is.
    #[test]
    fn git_scan_requires_trust() {
        let base = tmp_base("trust-gate");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["init", "-q", "."])
            .output()
            .unwrap();
        assert!(out.status.success());
        let mut w = Watcher::default();
        assert!(w.git(&repo, false).is_none(), "untrusted: no scan result");
        w.poll();
        std::thread::sleep(Duration::from_millis(100));
        w.poll();
        assert!(
            w.git(&repo, false).is_none(),
            "untrusted: the scan must not even have been started"
        );
        assert!(w.git(&repo, true).is_none(), "first call starts the scan");
        let mut tries = 0;
        while w.git(&repo, true).is_none() && tries < 100 {
            w.poll();
            std::thread::sleep(Duration::from_millis(20));
            tries += 1;
        }
        assert!(w.git(&repo, true).is_some(), "trusted: the scan ran");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A repository whose config probe fails closed is reported as
    /// `refused` (the UI names it instead of hiding git silently), and its
    /// diff preview gets `HeadBlob::Refused`, not "no old version". A plain
    /// folder and a healthy repository are not refused.
    #[test]
    fn refused_repository_is_told_apart_from_no_repository() {
        let base = tmp_base("refused");
        let plain = base.join("plain");
        let repo = base.join("repo");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .arg("-c")
                .arg("user.email=t@t")
                .arg("-c")
                .arg("user.name=t")
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
        };
        git(&["init", "-q", "."]);
        std::fs::write(repo.join("f.txt"), "hello\n").unwrap();
        git(&["add", "f.txt"]);
        git(&["commit", "-qm", "one"]);

        // healthy repository: branch known, not refused
        let ok = read_git(&repo);
        assert!(ok.branch.is_some() && !ok.refused, "{ok:?}");
        assert_eq!(
            head_blob_sized(&repo, "f.txt", 1024),
            HeadBlob::Blob("hello\n".to_string())
        );
        // plain folder: no repository, nothing refused
        let none = read_git(&plain);
        assert!(none.branch.is_none() && !none.refused, "{none:?}");

        // a repo-scope include makes the probe fail closed
        git(&["config", "include.path", "extra.cfg"]);
        let refused = read_git(&repo);
        assert!(refused.branch.is_none(), "no git ran: {refused:?}");
        assert!(refused.refused, "the refusal is reported: {refused:?}");
        assert_eq!(head_blob_sized(&repo, "f.txt", 1024), HeadBlob::Refused);
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod worktree_root_tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "catwright-root-{tag}-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Audit M10: `core.worktree` pointing outside the repository (here a
    /// stand-in for `$HOME` with a secret in it) must not make the app list
    /// or scan that tree; the repo shows no git state instead.
    #[test]
    fn a_repo_chosen_work_tree_elsewhere_is_refused() {
        let home = dir("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_rsa"), "secret").unwrap();
        let repo = dir("repo");
        git_output(&repo, &["init", "-q"]).unwrap();
        git_output(
            &repo,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "c",
            ],
        )
        .unwrap();
        git_output(&repo, &["config", "core.worktree", home.to_str().unwrap()]).unwrap();
        // without the check, git really lists the foreign tree (plain git)
        let plain = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["status", "--porcelain", "-uall"])
            .output()
            .unwrap();
        let listed = String::from_utf8_lossy(&plain.stdout);
        assert!(listed.contains(".ssh/id_rsa"), "{listed}");
        // our status scan sets the work tree itself (the folder holding
        // `.git`), so `core.worktree` cannot move it either
        let scanned = git_output(&repo, &["status", "--porcelain", "-uall"]).unwrap_or_default();
        assert!(!scanned.contains("id_rsa"), "{scanned}");
        let info = read_git(&repo);
        assert!(info.root.is_none(), "{:?}", info.root);
        assert!(info.files.is_empty() && info.branch.is_none());
        std::fs::remove_dir_all(&home).ok();
        std::fs::remove_dir_all(&repo).ok();
    }

    /// The ordinary cases still work: the repo root itself and a folder
    /// inside it.
    #[test]
    fn the_discovered_work_tree_is_accepted() {
        let repo = dir("plain");
        git_output(&repo, &["init", "-q"]).unwrap();
        // HEAD needs a commit, or there is no branch to show
        git_output(
            &repo,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "c",
            ],
        )
        .unwrap();
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::write(repo.join("sub/new.txt"), "x").unwrap();
        let root = std::fs::canonicalize(&repo).unwrap();
        let info = read_git(&repo);
        assert_eq!(info.root.as_deref(), Some(root.as_path()));
        assert_eq!(info.changed, 1);
        let info = read_git(&repo.join("sub"));
        assert_eq!(info.root.as_deref(), Some(root.as_path()));
        assert!(!plausible_root(&repo.join("sub"), &repo.join("sub")));
        std::fs::remove_dir_all(&repo).ok();
    }
}
