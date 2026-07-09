//! Exit-code convention shared by all ctx commands.
//!
//! ctx uses a three-way exit-code convention (similar to `grep` or linters):
//!
//! | Code | Meaning                                                    |
//! |------|------------------------------------------------------------|
//! | 0    | Success, no findings                                       |
//! | 1    | Command ran successfully but produced findings             |
//! | 2    | Operational error (bad arguments, missing index, IO, ...)  |
//!
//! Commands return `Ok(Outcome::Clean)` or `Ok(Outcome::Findings)`;
//! any `Err(_)` maps to exit code 2 in `main`.

/// The successful outcome of a command, mapped to a process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Command succeeded with nothing to report (exit code 0).
    Clean,
    /// Command succeeded but produced findings (exit code 1).
    ///
    /// Used by quality commands (e.g. a check that flags issues) so scripts
    /// and CI can distinguish "found problems" from "failed to run".
    Findings,
}

impl Outcome {
    /// The process exit code for this outcome.
    pub fn code(self) -> u8 {
        match self {
            Outcome::Clean => 0,
            Outcome::Findings => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outcome_codes() {
        assert_eq!(Outcome::Clean.code(), 0);
        assert_eq!(Outcome::Findings.code(), 1);
        assert_ne!(Outcome::Clean, Outcome::Findings);
    }
}
