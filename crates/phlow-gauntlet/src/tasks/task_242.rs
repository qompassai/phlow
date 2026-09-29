//! task-242: tool socket egress denied.
//!
//! Honest scope: Actual Python checks attempt AF_INET/AF_INET6 socket creation inside
//! Runtime.check. No external connection is made. EPERM/EACCES is the required denial; unavailable
//! Python or unsupported protocols are fixture failures. Broker forwarding itself has no API and is
//! not claimed.

//! A host loopback listener must succeed first; outer-sandbox socket denial is a
//! fixture limitation, never evidence of Phlow enforcement.

use super::task_233::{Fixture, ok, socket_denied};
use crate::{Ctx, TaskKind, TaskOutcome};

/// Stable task identifier.
pub const ID: &str = "task-242";
/// Desired invariant.
pub const NAME: &str = "tool socket egress denied";
/// Executes real Rust runtime seams.
pub const KIND: TaskKind = TaskKind::Rust;
/// Two validation cases, then two adversarial cases.
pub const CASES: [&str; 4] = [
    "non_network_tool_runs",
    "local_computation_runs",
    "ipv4_raw_socket_denied",
    "ipv6_raw_socket_denied",
];

/// Execute all cases, retaining assertion failures separately from fixture errors.
pub fn run(_ctx: &Ctx) -> TaskOutcome {
    super::task_233::collect_outcome(ID, CASES.iter().enumerate().map(|(i, c)| (*c, case(i))))
}

fn case(index: usize) -> Result<bool, String> {
    match index {
        0 => {
            let f = Fixture::new()?;
            let mut r = f.runtime(&["/usr/bin/python3", "-c", "pass"], 1000, true)?;
            Ok(ok(&r.check(Some("probe"))))
        }
        1 => {
            let f = Fixture::new()?;
            let mut r = f.runtime(
                &["/usr/bin/python3", "-c", "assert sum(range(10)) == 45"],
                1000,
                true,
            )?;
            Ok(ok(&r.check(Some("probe"))))
        }
        2 => socket_denied("AF_INET"),
        3 => socket_denied("AF_INET6"),
        _ => Err("case index outside fixed array".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn non_network_tool_runs() {
        let result = super::case(0);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn local_computation_runs() {
        let result = super::case(1);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn ipv4_raw_socket_denied() {
        let result = super::case(2);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }

    #[test]
    fn ipv6_raw_socket_denied() {
        let result = super::case(3);
        assert!(matches!(result, Ok(true)), "{result:?}");
    }
}
