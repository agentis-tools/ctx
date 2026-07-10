//! `ctx self-update` and `ctx --version [--check]` -- CLI wrappers around
//! [`ctx::update`].
//!
//! Exit codes: 0 = updated / already up to date / version printed,
//! 2 = operational error (network failure, checksum mismatch, install
//! location not writable, unsupported platform).

use ctx::error::Result;
use ctx::exit::Outcome;
use ctx::update;

/// Run `ctx self-update [--version X.Y.Z]`.
pub fn run_self_update(version: Option<&str>, json: bool) -> Result<Outcome> {
    let report = update::self_update(version)?;
    if json {
        ctx::json::emit(
            "self_update",
            serde_json::json!({
                "old_version": report.old_version.to_string(),
                "new_version": report.new_version.to_string(),
                "outcome": if report.updated { "updated" } else { "up_to_date" },
            }),
        )?;
    } else if report.updated {
        println!("ctx {} → {}", report.old_version, report.new_version);
    } else {
        println!("ctx {} is already up to date", report.old_version);
    }
    Ok(Outcome::Clean)
}

/// Run `ctx --version [--check]`.
///
/// Without `--check` this prints exactly what clap's auto version flag used
/// to print (`ctx <version>`). With `--check` it queries the latest GitHub
/// release and reports the comparison on stdout; it exits 0 whether or not
/// an update exists (network failures exit 2). The explicit check is always
/// allowed: it ignores the passive check's suppression rules and 24h cache.
pub fn run_version(check: bool, json: bool) -> Result<Outcome> {
    if !check {
        // Byte-identical to clap's disabled auto flag.
        println!("ctx {}", env!("CARGO_PKG_VERSION"));
        return Ok(Outcome::Clean);
    }

    let (current, latest) = update::explicit_check()?;
    if json {
        ctx::json::emit(
            "version.check",
            serde_json::json!({
                "current_version": current.to_string(),
                "latest_version": latest.to_string(),
                "update_available": latest > current,
            }),
        )?;
    } else if latest > current {
        println!("{}", update::update_notice(&latest, &current));
    } else {
        println!("ctx {current} is up to date");
    }
    Ok(Outcome::Clean)
}
