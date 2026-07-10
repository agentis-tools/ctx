//! Process execution: spawn the ctx binary, time wall clock around
//! spawn -> reap, and collect `ru_maxrss` via `libc::wait4`.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

/// One timed (or preparatory) invocation of the ctx binary.
#[derive(Debug)]
pub struct RunResult {
    pub wall_ms: f64,
    pub max_rss_kb: u64,
    /// Exit code; a signal death is reported as `128 + signo`.
    pub exit_code: i32,
    pub stderr: String,
}

impl RunResult {
    /// ctx's exit-code convention: 0 = clean, 1 = findings, 2+ = operational
    /// error. `ctx score`/`ctx check` may legitimately exit 1, so both 0 and
    /// 1 count as success for timing purposes.
    pub fn is_success(&self) -> bool {
        self.exit_code == 0 || self.exit_code == 1
    }
}

/// Run `bin args...` with `cwd` as the working directory and return timing,
/// exit code, max RSS, and captured stderr (stdout is discarded).
pub fn run_once(bin: &Path, args: &[&str], cwd: &Path) -> std::io::Result<RunResult> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    scrub_env(&mut cmd);

    let start = Instant::now();
    let mut child = cmd.spawn()?;
    let mut stderr_pipe = child.stderr.take().expect("stderr was piped");
    // Drain stderr on a thread so a chatty child can never fill the pipe
    // and deadlock against our wait4.
    let reader = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stderr_pipe.read_to_string(&mut buf);
        buf
    });

    let (exit_code, max_rss_kb) = wait4_reap(child.id() as libc::pid_t)?;
    let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
    let stderr = reader.join().unwrap_or_default();
    // `child` was already reaped by wait4; dropping the handle is harmless
    // (std's Child does not wait or kill on drop).
    Ok(RunResult {
        wall_ms,
        max_rss_kb,
        exit_code,
        stderr,
    })
}

/// Reap `pid` with `wait4(2)` to obtain its rusage. Returns
/// `(exit_code, max_rss_kb)`.
fn wait4_reap(pid: libc::pid_t) -> std::io::Result<(i32, u64)> {
    let mut status: libc::c_int = 0;
    // SAFETY: rusage is plain-old-data; an all-zero value is valid.
    let mut rusage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: pid refers to a child we spawned and have not reaped yet;
    // both out-pointers are valid for the duration of the call.
    let rc = unsafe { libc::wait4(pid, &mut status, 0, &mut rusage) };
    if rc == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let exit_code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        -1
    };
    Ok((exit_code, normalize_maxrss(rusage.ru_maxrss)))
}

/// `ru_maxrss` is kilobytes on Linux but bytes on macOS; normalize to KB.
fn normalize_maxrss(raw: libc::c_long) -> u64 {
    let raw = raw.max(0) as u64;
    if cfg!(target_os = "macos") {
        raw / 1024
    } else {
        raw
    }
}

/// Scrub the child environment:
///
/// - drop every `CTX_GATE_*` variable so gate logging / gate config from the
///   invoking shell cannot alter what the timed command does;
/// - neutralize the passive update check. Per `src/update.rs`,
///   `CTX_NO_UPDATE_CHECK` (non-empty) suppresses the check outright — no
///   network call at all — and a non-TTY stderr (our pipe) suppresses it too.
///   As a final belt-and-suspenders measure the API base URL is pointed at an
///   unroutable localhost port so even a code regression in the suppression
///   logic cannot reach the network during timed runs.
fn scrub_env(cmd: &mut Command) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("CTX_GATE_") {
            cmd.env_remove(&key);
        }
    }
    cmd.env_remove("CTX_UPDATE_FORCE_TTY");
    cmd.env("CTX_NO_UPDATE_CHECK", "1");
    cmd.env("CTX_UPDATE_BASE_URL", "http://127.0.0.1:9");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_maxrss_clamps_negative() {
        assert_eq!(normalize_maxrss(-5), 0);
    }

    #[test]
    fn run_once_times_a_real_process() {
        // `true` exits 0 instantly; enough to prove spawn/reap/rusage works.
        let result = run_once(Path::new("/usr/bin/true"), &[], Path::new("/")).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.is_success());
        assert!(result.wall_ms >= 0.0);
    }

    #[test]
    fn run_once_captures_exit_code_and_stderr() {
        let result = run_once(
            Path::new("/bin/sh"),
            &["-c", "echo oops >&2; exit 2"],
            Path::new("/"),
        )
        .unwrap();
        assert_eq!(result.exit_code, 2);
        assert!(!result.is_success());
        assert!(result.stderr.contains("oops"));
    }
}
