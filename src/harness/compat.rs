//! Version compatibility check for `ctx harness compat`.
//!
//! Generated hook scripts begin with `ctx harness compat --require <V>`
//! where `<V>` is the ctx version that generated them. The guard means
//! "this binary must be at least as new as the templates", so a *bare*
//! version is compared with `>=` semantics -- deliberately **not** semver
//! caret semantics, which would fail `--require 0.1` against a 0.2.x binary
//! even though the newer binary understands the older templates.
//!
//! Explicit requirement expressions (`^0.2`, `>=0.2, <0.4`, `0.2.*`) are
//! parsed as [`semver::VersionReq`] and honored as written.

use crate::error::{CtxError, Result};
use semver::{Version, VersionReq};

/// True if `require` contains an explicit requirement operator, meaning it
/// should be parsed as a full semver `VersionReq` rather than a bare
/// "at least this version" floor.
fn has_operator(require: &str) -> bool {
    require
        .chars()
        .any(|c| matches!(c, '^' | '~' | '>' | '<' | '=' | '*' | ',') || c.is_whitespace())
}

/// Zero-pad a bare version to three components (`"0.1"` -> `"0.1.0"`).
fn pad_bare_version(require: &str) -> String {
    // Only pad the numeric core; keep any pre-release/build suffix intact.
    let (core, suffix) = match require.find(['-', '+']) {
        Some(idx) => require.split_at(idx),
        None => (require, ""),
    };
    let dots = core.matches('.').count();
    match dots {
        0 => format!("{core}.0.0{suffix}"),
        1 => format!("{core}.0{suffix}"),
        _ => require.to_string(),
    }
}

/// Does `current` satisfy the requirement string `require`?
///
/// Bare versions are floors (`current >= require`); expressions with
/// operators are semver requirements. Unparseable input is an error
/// (exit code 2 at the CLI, never 3).
pub fn satisfies(current: &Version, require: &str) -> Result<bool> {
    let require = require.trim();
    if require.is_empty() {
        return Err(CtxError::Other(
            "--require needs a version (e.g. \"0.2\") or requirement (e.g. \">=0.2\")".to_string(),
        ));
    }

    if has_operator(require) {
        let req = VersionReq::parse(require).map_err(|e| {
            CtxError::Other(format!("invalid version requirement '{require}': {e}"))
        })?;
        return Ok(req.matches(current));
    }

    let padded = pad_bare_version(require);
    let required = Version::parse(&padded)
        .map_err(|e| CtxError::Other(format!("invalid version '{require}': {e}")))?;
    Ok(current >= &required)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn test_bare_versions_are_floors() {
        let current = v("0.2.1");
        // "0.1" would fail under caret semantics; the floor accepts it.
        assert!(satisfies(&current, "0.1").unwrap());
        assert!(satisfies(&current, "0.2").unwrap());
        assert!(satisfies(&current, "0.2.1").unwrap());
        assert!(!satisfies(&current, "0.2.2").unwrap());
        assert!(!satisfies(&current, "999.0").unwrap());
        assert!(!satisfies(&current, "999").unwrap());
    }

    #[test]
    fn test_operator_expressions_use_semver_req() {
        let current = v("0.2.1");
        assert!(satisfies(&current, "^0.2").unwrap());
        assert!(!satisfies(&current, "^0.1").unwrap()); // caret: 0.1.x only
        assert!(satisfies(&current, ">=0.1").unwrap());
        assert!(!satisfies(&current, ">=999").unwrap());
        assert!(satisfies(&current, ">=0.2, <0.3").unwrap());
        assert!(satisfies(&current, "0.2.*").unwrap());
    }

    #[test]
    fn test_garbage_is_an_error() {
        let current = v("0.2.1");
        assert!(satisfies(&current, "garbage").is_err());
        assert!(satisfies(&current, "").is_err());
        assert!(satisfies(&current, ">>nope").is_err());
    }

    #[test]
    fn test_bare_prerelease_versions_parse() {
        let current = v("0.3.0");
        assert!(satisfies(&current, "0.3.0-alpha").unwrap());
        assert!(!satisfies(&v("0.3.0-alpha"), "0.3.0").unwrap());
    }
}
