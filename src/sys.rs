//! Platform process listing + PATH lookup — used by `doctor` and by the
//! run-lock liveness check. Kept dependency-free: unix shells out to
//! `ps`, Windows to `tasklist`.

use std::path::{Path, PathBuf};

/// Process names agentwiki may spawn (or that indicate a concurrent run).
/// Matched against the basename of argv[0], lowercased, `.exe` stripped.
pub const AGENT_PROCS: &[&str] = &["agentwiki", "devin", "claude", "codex"];

/// argv[0] basenames that execute a script given as argv[1] — a
/// node-installed `codex` shows up as `node /path/to/codex …`.
const INTERPRETERS: &[&str] = &[
    "node", "bun", "deno", "python", "python3", "sh", "bash", "zsh", "env",
];

/// One observed process.
#[derive(Debug, Clone)]
pub struct ProcInfo {
    /// Process id.
    pub pid: u32,
    /// Basename of argv[0] (image name on Windows), lowercase, no `.exe`.
    pub name: String,
    /// Basename of argv[1] when argv[0] is an interpreter, else `None`.
    pub script: Option<String>,
    /// `ps`-style elapsed time (`[[dd-]hh:]mm:ss`) when available.
    pub etime: Option<String>,
}

/// Snapshot of the process table; empty when the platform probe fails.
pub fn list_processes() -> Vec<ProcInfo> {
    raw_procs()
}

/// Is `pid` a live process right now?
pub fn pid_alive(pid: u32) -> bool {
    list_processes().iter().any(|p| p.pid == pid)
}

/// Does this process look like one of the agent CLIs / agentwiki itself?
/// Returns the canonical name when it does.
pub fn agent_name(p: &ProcInfo) -> Option<&'static str> {
    let cands = [Some(&p.name), p.script.as_ref()];
    cands
        .into_iter()
        .flatten()
        .find_map(|c| AGENT_PROCS.iter().copied().find(|n| *n == c.as_str()))
}

/// First executable named `name` on `PATH`.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let base = dir.join(name);
        #[cfg(unix)]
        let candidates = [base];
        #[cfg(windows)]
        let candidates = [
            base.with_extension("exe"),
            base.with_extension("cmd"),
            base.with_extension("bat"),
            base,
        ];
        for c in candidates {
            if is_executable(&c) {
                return Some(c);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// Basename for both `/` and `\` separators, lowercased, `.exe` stripped.
fn basename_lc(arg0: &str) -> String {
    let base = arg0.rsplit(['/', '\\']).next().unwrap_or(arg0);
    base.to_lowercase()
        .strip_suffix(".exe")
        .map(str::to_string)
        .unwrap_or_else(|| base.to_lowercase())
}

#[cfg(unix)]
fn raw_procs() -> Vec<ProcInfo> {
    // `etime` is a formatted duration supported by both Linux and macOS ps.
    let out = std::process::Command::new("ps")
        .args(["-eo", "pid=,etime=,args="])
        .output();
    match out {
        Ok(o) if o.status.success() => parse_ps(&String::from_utf8_lossy(&o.stdout), true),
        _ => std::process::Command::new("ps")
            .args(["-eo", "pid=,args="])
            .output()
            .map(|o| parse_ps(&String::from_utf8_lossy(&o.stdout), false))
            .unwrap_or_default(),
    }
}

#[cfg(windows)]
fn raw_procs() -> Vec<ProcInfo> {
    let out = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output();
    let Ok(o) = out else { return Vec::new() };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| {
            // `"devin.exe","1234","Console","1","12,345 K"`
            let fields: Vec<&str> = l.trim().trim_matches('"').split("\",\"").collect();
            let name = basename_lc(fields.first()?);
            let pid = fields.get(1)?.parse().ok()?;
            Some(ProcInfo {
                pid,
                name,
                script: None,
                etime: None,
            })
        })
        .collect()
}

#[cfg(unix)]
fn parse_ps(text: &str, with_etime: bool) -> Vec<ProcInfo> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid: u32 = it.next()?.parse().ok()?;
            let etime = if with_etime {
                it.next().map(str::to_string)
            } else {
                None
            };
            let args: Vec<&str> = it.collect();
            let name = basename_lc(args.first()?);
            let script = if INTERPRETERS.contains(&name.as_str()) {
                args.get(1).map(|s| basename_lc(s))
            } else {
                None
            };
            Some(ProcInfo {
                pid,
                name,
                script,
                etime,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn parse_ps_rows() {
        let text = "  123 02:11 /usr/local/bin/devin run --foo\n\
                    9999 1-02:03:04 /usr/bin/node /opt/x/codex\n\
                    42 00:01 vim devin.md\n";
        let procs = parse_ps(text, true);
        assert_eq!(procs.len(), 3);
        assert_eq!(procs[0].pid, 123);
        assert_eq!(procs[0].name, "devin");
        assert_eq!(procs[0].etime.as_deref(), Some("02:11"));
        assert_eq!(procs[1].name, "node");
        assert_eq!(procs[1].script.as_deref(), Some("codex"));
        // `vim devin.md` — argv0 is vim, must not match as an agent.
        assert_eq!(agent_name(&procs[2]), None);
        assert_eq!(agent_name(&procs[0]), Some("devin"));
        assert_eq!(agent_name(&procs[1]), Some("codex"));
    }

    #[test]
    fn basename_strips_exe_and_dirs() {
        assert_eq!(basename_lc("/usr/bin/Devin"), "devin");
        assert_eq!(basename_lc("C:\\tools\\claude.EXE"), "claude");
        assert_eq!(basename_lc("agentwiki.exe"), "agentwiki");
    }

    #[test]
    fn find_self_on_path() {
        // `sh`/`cmd` is guaranteed on PATH for the host platform.
        #[cfg(unix)]
        assert!(find_on_path("sh").is_some());
        #[cfg(windows)]
        assert!(find_on_path("cmd").is_some());
        assert!(find_on_path("definitely-not-a-real-cli-xyz").is_none());
    }
}
