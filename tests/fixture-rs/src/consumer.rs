use crate::core::{engine::Engine, util::Util};
use crate::facade::Helper;

#[allow(dead_code)]
pub fn make() -> (Engine, Util, Helper) {
    (Engine { u: Util }, Util, Helper)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deep::inner::probe;

    #[test]
    fn probe_works() {
        let _ = probe();
    }
}
