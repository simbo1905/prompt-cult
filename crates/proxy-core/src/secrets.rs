//! Memory-hardened API key handling.
//!
//! Ported from OpenAI's `codex-rs/responses-api-proxy/src/read_api_key.rs`
//! (Apache-2.0, openai/codex#4778) via the Prompt Cult fork's
//! `mistral-proxy` — the fork's copy was diff-verified identical in core
//! logic to upstream as of 2026-09-05 (see `docs/hardening.md`). The fork
//! adds the environment-first key path; the stdin fallback and all
//! hardening invariants are upstream's, with their original comments
//! restored verbatim.
//!
//! Generalization beyond upstream: the caller names the environment
//! variable and the binary, so any service's proxy can share this code.
//!
//! The aim is that there is exactly ONE copy of the API key in memory, in
//! the `Authorization` header value, protected by `mlock(2)`. The key is
//! never materialized in a second `String`, `Vec`, or buffer that outlives
//! the read, never logged, and never written to disk.

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use zeroize::Zeroize;

/// Use a generous buffer size to avoid truncation and to allow for longer API
/// keys in the future.
const BUFFER_SIZE: usize = 1024;
const AUTH_HEADER_PREFIX: &[u8] = b"Bearer ";

/// Reads the auth token for `env_var` from the environment if present and
/// non-empty, falling back to stdin, and returns a static `Authorization`
/// header value with the auth token used with `Bearer`. The header value is
/// returned as a `&'static str` whose bytes are locked in memory to avoid
/// accidental exposure.
pub fn read_auth_header(env_var: &str, bin_name: &str) -> Result<&'static str> {
    // The environment is checked first: a non-empty value short-circuits the
    // stdin path entirely.
    if let Ok(env_val) = std::env::var(env_var)
        && !env_val.trim().is_empty()
    {
        return auth_header_from_key(env_val.trim());
    }

    #[cfg(unix)]
    let read = read_from_unix_stdin;
    // Use of `std::io::stdin()` has the problem mentioned in the docstring on
    // the UNIX version of `read_from_unix_stdin()`, so this should ultimately
    // be replaced with the low-level Windows equivalent. Because we do not
    // have an equivalent of mlock() on Windows right now, it is not pressing
    // until we address that issue.
    #[cfg(windows)]
    let read = |buffer: &mut [u8]| std::io::Read::read(&mut std::io::stdin(), buffer);

    read_auth_header_with(env_var, bin_name, read)
}

/// Produces a static `Authorization` header value from an already-available
/// key, applying the same validation and `mlock(2)` protection as the stdin
/// path. The key MUST contain only ASCII letters, numbers, `-` or `_`.
pub fn auth_header_from_key(key: &str) -> Result<&'static str> {
    let mut header = Vec::with_capacity(AUTH_HEADER_PREFIX.len() + key.len());
    header.extend_from_slice(AUTH_HEADER_PREFIX);
    header.extend_from_slice(key.as_bytes());
    if let Err(err) = validate_auth_header_bytes(&header[AUTH_HEADER_PREFIX.len()..]) {
        header.zeroize();
        return Err(err);
    }
    let header_str = match std::str::from_utf8(&header) {
        Ok(value) => value,
        Err(err) => {
            // In theory, validate_auth_header_bytes() should have caught
            // any invalid UTF-8 sequences, but just in case...
            header.zeroize();
            return Err(err).context("constructing Authorization header as UTF-8");
        }
    };
    let leaked: &'static mut str = header_str.to_string().leak();
    mlock_str(leaked);
    Ok(leaked)
}

/// We perform a low-level read with `read(2)` because `stdio::io::stdin()` has
/// an internal BufReader:
///
/// https://github.com/rust-lang/rust/blob/bcbbdcb8522fd3cb4a8dde62313b251ab107694d/library/std/src/io/stdio.rs#L250-L252
///
/// that can end up retaining a copy of stdin data in memory with no way to zero
/// it out, whereas we aim to guarantee there is exactly one copy of the API key
/// in memory, protected by mlock(2).
// Safety: only read(2) into the caller's buffer; no invariants to uphold
// beyond those documented in the module header.
#[cfg(unix)]
#[allow(unsafe_code)]
fn read_from_unix_stdin(buffer: &mut [u8]) -> std::io::Result<usize> {
    use libc::c_void;
    use libc::read;

    // Perform a single read(2) call into the provided buffer slice.
    // Looping and newline/EOF handling are managed by the caller.
    loop {
        let result = unsafe {
            read(
                libc::STDIN_FILENO,
                buffer.as_mut_ptr().cast::<c_void>(),
                buffer.len(),
            )
        };

        if result == 0 {
            return Ok(0);
        }

        if result > 0 {
            return Ok(result as usize);
        }

        // read(2) returning a negative value signals an error, which is
        // reflected in errno.
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(err);
    }
}

fn read_auth_header_with<F>(env_var: &str, bin_name: &str, mut read: F) -> Result<&'static str>
where
    F: FnMut(&mut [u8]) -> std::io::Result<usize>,
{
    let mut buf = AUTH_HEADER_PREFIX.to_vec();
    buf.resize(BUFFER_SIZE, 0);
    let prefix_len = AUTH_HEADER_PREFIX.len();
    let mut total = prefix_len;

    // Read at most one line: stop at the first newline so any bytes after it
    // are never even buffered, and at EOF so a short key is accepted.
    let mut saw_newline = false;
    let mut saw_eof = false;

    while total < BUFFER_SIZE {
        let slice = &mut buf[total..];
        let read = match read(slice) {
            Ok(n) => n,
            Err(err) => {
                buf.zeroize();
                return Err(err).context("reading API key from stdin");
            }
        };
        if read == 0 {
            saw_eof = true;
            break;
        }
        let newly_written = &slice[..read];
        if let Some(pos) = newly_written.iter().position(|&b| b == b'\n') {
            total += pos + 1;
            saw_newline = true;
            break;
        }
        total += read;
    }

    if total == BUFFER_SIZE && !saw_newline && !saw_eof {
        // We filled the entire buffer without seeing a newline. Assume the
        // input is too long, and give up rather than risk reading a partial
        // value.
        buf.zeroize();
        return Err(anyhow!("API key provided on stdin is too long"));
    }

    // If the input ends with trailing newline characters, strip them
    // (\n, \r\n, and lone \r).
    while total > prefix_len && (buf[total - 1] == b'\n' || buf[total - 1] == b'\r') {
        total -= 1;
    }

    if total == AUTH_HEADER_PREFIX.len() {
        buf.zeroize();
        return Err(anyhow!(
            "API key must be provided via stdin (e.g. printenv {env_var} | {bin_name})"
        ));
    }

    if let Err(err) = validate_auth_header_bytes(&buf[AUTH_HEADER_PREFIX.len()..total]) {
        buf.zeroize();
        return Err(err);
    }

    let header_str = match std::str::from_utf8(&buf[..total]) {
        Ok(value) => value,
        Err(err) => {
            // In theory, validate_auth_header_bytes() should have caught
            // any invalid UTF-8 sequences, but just in case...
            buf.zeroize();
            return Err(err).context("reading Authorization header from stdin as UTF-8");
        }
    };

    let header_value = String::from(header_str);
    buf.zeroize();

    let leaked: &'static mut str = header_value.leak();
    mlock_str(leaked);

    Ok(leaked)
}

// Safety: mlock(2)/sysconf(2) with page-aligned arithmetic verified above.
#[cfg(unix)]
#[allow(unsafe_code)]
fn mlock_str(value: &str) {
    use libc::_SC_PAGESIZE;
    use libc::c_void;
    use libc::mlock;
    use libc::sysconf;

    if value.is_empty() {
        return;
    }

    let page_size = unsafe { sysconf(_SC_PAGESIZE) };
    if page_size <= 0 {
        return;
    }
    let page_size = page_size as usize;
    if page_size == 0 {
        return;
    }

    let addr = value.as_ptr() as usize;
    let len = value.len();
    let start = addr & !(page_size - 1);
    let addr_end = match addr.checked_add(len) {
        Some(v) => match v.checked_add(page_size - 1) {
            Some(total) => total,
            None => return,
        },
        None => return,
    };
    let end = addr_end & !(page_size - 1);
    let size = end.saturating_sub(start);
    if size == 0 {
        return;
    }

    let _ = unsafe { mlock(start as *const c_void, size) };
}

#[cfg(not(unix))]
fn mlock_str(_value: &str) {}

/// The key should match /^[A-Za-z0-9\-_]+$/.
///
/// Ensure there is no funny business with NUL characters and whatnot.
///
/// Note: some vendors issue keys containing other characters (see
/// docs/appendix-proxy-bugs.md, upstream openai/codex#34138). Verify the
/// charset against each vendor before enabling a service; loosening it is a
/// deliberate, reviewed decision.
fn validate_auth_header_bytes(key_bytes: &[u8]) -> Result<()> {
    if key_bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Ok(());
    }

    Err(anyhow!(
        "API key may only contain ASCII letters, numbers, '-' or '_'"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn reads_key_with_no_newlines() {
        let mut sent = false;
        let result = read_auth_header_with("K", "bin", |buf| {
            if sent {
                return Ok(0);
            }
            let data = b"sk-abc123";
            buf[..data.len()].copy_from_slice(data);
            sent = true;
            Ok(data.len())
        });
        assert_eq!(result.expect("ok"), "Bearer sk-abc123");
    }

    #[test]
    fn reads_key_with_one_trailing_newline() {
        let mut sent = false;
        let result = read_auth_header_with("K", "bin", |buf| {
            if sent {
                return Ok(0);
            }
            let data = b"sk-abc123\n";
            buf[..data.len()].copy_from_slice(data);
            sent = true;
            Ok(data.len())
        });
        assert_eq!(result.expect("ok"), "Bearer sk-abc123");
    }

    #[test]
    fn reads_key_with_multiple_trailing_newlines() {
        let mut sent = false;
        let result = read_auth_header_with("K", "bin", |buf| {
            if sent {
                return Ok(0);
            }
            let data = b"sk-abc123\n\n\n\n";
            buf[..data.len()].copy_from_slice(data);
            sent = true;
            Ok(data.len())
        });
        assert_eq!(result.expect("ok"), "Bearer sk-abc123");
    }

    #[test]
    fn reads_key_across_multiple_reads() {
        let mut sent = false;
        let result = read_auth_header_with("K", "bin", |buf| {
            if sent {
                return Ok(0);
            }
            let data = b"sk-abc123\n";
            let mut i = 0;
            while i < data.len() {
                buf[i] = data[i];
                i += 1;
            }
            sent = true;
            Ok(data.len())
        });
        assert_eq!(result.expect("ok"), "Bearer sk-abc123");
    }

    #[test]
    fn rejects_overlong_key() {
        let mut sent = false;
        let result = read_auth_header_with("K", "bin", |buf| {
            if sent {
                return Ok(0);
            }
            buf.fill(b'A');
            sent = true;
            Ok(buf.len())
        });
        let err = result.expect_err("too long");
        assert_eq!(err.to_string(), "API key provided on stdin is too long");
    }

    #[test]
    fn rejects_key_with_invalid_charset() {
        let mut sent = false;
        let result = read_auth_header_with("K", "bin", |buf| {
            if sent {
                return Ok(0);
            }
            let data = b"sk-abc123!\n";
            buf[..data.len()].copy_from_slice(data);
            sent = true;
            Ok(data.len())
        });
        let err = result.expect_err("bad charset");
        assert_eq!(
            err.to_string(),
            "API key may only contain ASCII letters, numbers, '-' or '_'"
        );
    }

    #[test]
    fn rejects_missing_key() {
        let result = read_auth_header_with("K", "bin", |_| Ok(0));
        let err = result.expect_err("missing");
        assert_eq!(
            err.to_string(),
            "API key must be provided via stdin (e.g. printenv K | bin)"
        );
    }

    #[test]
    fn auth_header_from_key_validates_and_locks() {
        assert_eq!(
            auth_header_from_key("sk-abc123").expect("ok"),
            "Bearer sk-abc123"
        );
        let err = auth_header_from_key("has spaces and !").expect_err("bad");
        assert_eq!(
            err.to_string(),
            "API key may only contain ASCII letters, numbers, '-' or '_'"
        );
    }

    #[test]
    fn leaked_headers_are_distinct_values() {
        let a = auth_header_from_key("sk-one").expect("ok");
        let b = auth_header_from_key("sk-two").expect("ok");
        assert_eq!(a, "Bearer sk-one");
        assert_eq!(b, "Bearer sk-two");
        assert!(!std::ptr::eq(a.as_ptr(), b.as_ptr()));
    }
}
