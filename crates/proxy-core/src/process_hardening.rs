//! Process hardening, applied before `main` via `#[ctor]`.
//!
//! Ported from OpenAI's `codex-rs/process-hardening` (Apache-2.0,
//! openai/codex#4403), with the fork's two audited deltas: removal of
//! upstream's unused `disable_process_dumping()` and the addition of macOS
//! malloc stack-logging cleanup (upstream issue #11555). See
//! `docs/hardening.md` for the lineage audit.
//!
//! This is defense in depth for the single-copy key invariant: disable core
//! dumps, deny debugger attach, and strip dangerous loader environment
//! variables so key material cannot be harvested from the process image
//! even if a second copy existed.

/// Strip the `DYLD_*`, `LD_*`, and macOS malloc stack-logging environment
/// variables from the process. On Linux, only `LD_*` variables are stripped.
// Safety: `remove_var` mutates process-global state in pre-main
// single-threaded startup.
#[allow(unsafe_code)]
fn clean_env() {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;

        let env_vars_to_remove: Vec<std::ffi::OsString> = std::env::vars_os()
            .map(|(k, _)| k)
            .filter(|k| {
                let bytes = k.as_bytes();
                bytes.starts_with(b"DYLD_")
                    || bytes.starts_with(b"LD_")
                    || bytes == b"MallocStackLogging"
                    || bytes == b"MallocStackLoggingDirectory"
                    || bytes == b"MallocLogFile"
            })
            .collect();
        for k in env_vars_to_remove {
            // Safety (edition 2024): `remove_var` mutates process-global
            // state. We are in pre-main single-threaded startup.
            unsafe {
                std::env::remove_var(k);
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        use std::os::unix::ffi::OsStrExt;

        let env_vars_to_remove: Vec<std::ffi::OsString> = std::env::vars_os()
            .map(|(k, _)| k)
            .filter(|k| k.as_bytes().starts_with(b"LD_"))
            .collect();
        for k in env_vars_to_remove {
            // Safety (edition 2024): `remove_var` mutates process-global
            // state. We are in pre-main single-threaded startup.
            unsafe {
                std::env::remove_var(k);
            }
        }
    }
}

// Safety: prctl(2) with PR_SET_DUMPABLE and constant arguments.
#[allow(unsafe_code)]
fn disable_dumpability() {
    // PR_SET_DUMPABLE is Linux-specific; macOS and other Unixes use
    // PT_DENY_ATTACH which is set in disable_core_dumps.
    #[cfg(target_os = "linux")]
    {
        const PR_SET_DUMPABLE: libc::c_int = 4;
        // Safety: `prctl` is a libc function that is safe to call with
        // PR_SET_DUMPABLE and argument 0.
        unsafe {
            libc::prctl(PR_SET_DUMPABLE, 0, 0, 0, 0);
        }
    }
}

/// Disable the ability to dump the process.
// Safety: getrlimit(2)/setrlimit(2) on a local rlimit, and ptrace(2) with
// PT_DENY_ATTACH and constant arguments on macOS.
#[allow(unsafe_code)]
fn disable_core_dumps() {
    #[cfg(unix)]
    {
        #[cfg(target_os = "macos")]
        const MACOS_PT_DENY_ATTACH: libc::c_int = 31;

        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };

        // Safety: `getrlimit` is a libc function that is safe to call with
        // a valid pointer to a `rlimit` struct.
        let ret = unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut lim) };

        if ret == 0 {
            lim.rlim_cur = 0;
            lim.rlim_max = 0;

            // Safety: `setrlimit` is a libc function that is safe to call
            // with a valid pointer to a `rlimit` struct.
            unsafe { libc::setrlimit(libc::RLIMIT_CORE, &lim) };
        }

        #[cfg(target_os = "macos")]
        {
            // Safety: `ptrace` is a libc function that is safe to call
            // with PT_DENY_ATTACH and arguments 0, 0.
            unsafe {
                libc::ptrace(MACOS_PT_DENY_ATTACH, 0, std::ptr::null_mut(), 0);
            }
        }
    }
}

/// Perform a best-effort hardening of the process. This is done before
/// the `main` function is called to ensure that the process is hardened
/// as early as possible.
///
/// This function is intended to be called before any code that could
/// be exploited is run.
pub fn pre_main_hardening() {
    disable_dumpability();
    disable_core_dumps();
    clean_env();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_main_hardening_is_idempotent() {
        pre_main_hardening();
        pre_main_hardening();
    }
}
