use super::util::Util;

pub struct Engine {
    pub u: Util,
}

pub fn run() {
    // Inline path — the ONLY evidence for engine -> deep::inner.
    let _ = crate::deep::inner::probe();
}
