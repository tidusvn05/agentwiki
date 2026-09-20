//! Shared diagnostic-report primitives used by `doctor` and `drift`.

/// Severity of one check line; ordered so `max` gives the overall result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum Status {
    /// Neutral information.
    #[default]
    Info,
    /// Check passed.
    Ok,
    /// Suspicious but not blocking.
    Warn,
    /// Blocking problem.
    Fail,
}

impl Status {
    pub(crate) fn tag(self) -> &'static str {
        match self {
            Status::Ok => "  ok  ",
            Status::Info => " info ",
            Status::Warn => " warn ",
            Status::Fail => " FAIL ",
        }
    }
}

/// Accumulates check lines and the worst status.
#[derive(Default)]
pub(crate) struct Report {
    pub(crate) lines: Vec<(Status, String)>,
    pub(crate) worst: Status,
    pub(crate) fixes: Vec<String>,
}

impl Report {
    pub(crate) fn add(&mut self, status: Status, msg: impl Into<String>) {
        self.worst = self.worst.max(status);
        self.lines.push((status, msg.into()));
    }

    pub(crate) fn fixed(&mut self, msg: impl Into<String>) {
        self.fixes.push(msg.into());
    }
}

/// One named section of check lines.
pub(crate) fn print_section(name: &str, r: &Report) {
    println!("{name}:");
    for (s, m) in &r.lines {
        println!("  {} {m}", s.tag());
    }
    println!();
}
