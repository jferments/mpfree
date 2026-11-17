//! Downloads over HTTPS with the system's libcurl, opened at run time
//! (`libcurl.so.4`): MPFree links no TLS code of its own. The MPC's own
//! system has libcurl (Akai's app uses it), and the stick's system is a copy
//! of it. Without libcurl, or without a network, a download fails with a
//! reason and the musician copies zips to the stick instead.
//!
//! **A static program never opens libcurl itself.** The pad player for the
//! MPC is linked statically (`+crt-static`), and glibc's static `dlopen`
//! can't host libcurl reliably. Under qemu, a build linked against the
//! workstation's glibc (2.43, newer than the MPC's 2.39) crashed inside
//! `dlopen`; one linked against 2.39 got through TLS, but libcurl's resolver
//! thread crashed in `pthread_create`, since a thread made inside a library
//! loaded by a static program finds no thread setup of its own. On the MPC
//! (the 16 Nov live run) the download stopped at curl's "Insufficient
//! randomness" instead; that exact failure was not reproduced, so it being
//! the same static `dlopen` is an assumption. So a static build hands each
//! download to [`HELPER`], a small program linked dynamically against the
//! MPC's glibc (built with `cargo zigbuild` for glibc 2.34) and installed
//! beside it, which runs this same code ([`fetch_here`]) and reports on its
//! standard output ([`helper_main`]). The check of libcurl's OpenSSL
//! ([`tls`]) runs in the helper too: it starts with libcurl in every
//! download, and `mpfree-fetch --tls` reports it for the Plugins place's
//! first line ([`tls_through_helper`]).
//!
//! Only `https://` addresses on the catalog's own hosts are fetched, and
//! redirects may only go to HTTPS.

use std::ffi::{c_char, c_int, c_long, c_ulong, c_void, CStr, CString};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The download helper's file name, beside the program that uses it
/// (`/etc/mpfree-test/mpfree-fetch` on the stick).
pub const HELPER: &str = "mpfree-fetch";

/// Whether this program is linked statically, so downloads go through
/// [`HELPER`].
const STATIC: bool = cfg!(target_feature = "crt-static");

/// Hosts a download may start at: the catalog's site and GitHub, where the
/// releases and screenshots are.
pub const HOSTS: [&str; 5] = [
    "open-mpc.github.io",
    "sd88me.github.io",
    "github.com",
    "raw.githubusercontent.com",
    "objects.githubusercontent.com",
];

/// Whether `url` is one MPFree may fetch: `https://` on one of [`HOSTS`].
pub fn allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    HOSTS.contains(&host)
}

// libcurl's option numbers (curl/curl.h): LONG options from 0,
// OBJECTPOINT from 10000, FUNCTIONPOINT from 20000.
const OPT_WRITEDATA: c_int = 10001;
const OPT_URL: c_int = 10002;
const OPT_ERRORBUFFER: c_int = 10010;
const OPT_USERAGENT: c_int = 10018;
const OPT_XFERINFODATA: c_int = 10057;
const OPT_WRITEFUNCTION: c_int = 20011;
const OPT_XFERINFOFUNCTION: c_int = 20219;
const OPT_LOW_SPEED_LIMIT: c_int = 19;
const OPT_LOW_SPEED_TIME: c_int = 20;
const OPT_NOPROGRESS: c_int = 43;
const OPT_FAILONERROR: c_int = 45;
const OPT_FOLLOWLOCATION: c_int = 52;
const OPT_MAXREDIRS: c_int = 68;
const OPT_CONNECTTIMEOUT: c_int = 78;
const OPT_NOSIGNAL: c_int = 99;
const OPT_MAXFILESIZE: c_int = 114;
const OPT_PROTOCOLS: c_int = 181;
const OPT_REDIR_PROTOCOLS: c_int = 182;
#[cfg(test)]
const PROTO_HTTP: c_long = 1;
const PROTO_HTTPS: c_long = 2;
const ERROR_SIZE: usize = 256;

type SetOpt = unsafe extern "C" fn(*mut c_void, c_int, ...) -> c_int;

struct Curl {
    init: unsafe extern "C" fn() -> *mut c_void,
    setopt: SetOpt,
    perform: unsafe extern "C" fn(*mut c_void) -> c_int,
    cleanup: unsafe extern "C" fn(*mut c_void),
    strerror: unsafe extern "C" fn(c_int) -> *const c_char,
    /// What [`check_tls`] said once libcurl had started.
    tls: String,
}

fn curl() -> Result<&'static Curl, String> {
    static LIB: OnceLock<Result<Curl, String>> = OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let mut h = std::ptr::null_mut();
        for name in [c"libcurl.so.4", c"libcurl.so", c"libcurl-gnutls.so.4"] {
            h = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if !h.is_null() {
                break;
            }
        }
        if h.is_null() {
            return Err("this system has no libcurl, so MPFree can't download".into());
        }
        let sym = |n: &CStr| {
            let p = libc::dlsym(h, n.as_ptr());
            if p.is_null() {
                Err(format!("libcurl has no {}", n.to_string_lossy()))
            } else {
                Ok(p)
            }
        };
        type P = *mut c_void;
        let global_init = std::mem::transmute::<P, unsafe extern "C" fn(c_long) -> c_int>(sym(
            c"curl_global_init",
        )?);
        // CURL_GLOBAL_DEFAULT: SSL and Win32 (nothing on Linux).
        if global_init(3) != 0 {
            return Err("libcurl didn't start".into());
        }
        Ok(Curl {
            init: std::mem::transmute::<P, unsafe extern "C" fn() -> P>(sym(c"curl_easy_init")?),
            setopt: std::mem::transmute::<P, SetOpt>(sym(c"curl_easy_setopt")?),
            perform: std::mem::transmute::<P, unsafe extern "C" fn(P) -> c_int>(sym(
                c"curl_easy_perform",
            )?),
            cleanup: std::mem::transmute::<P, unsafe extern "C" fn(P)>(sym(c"curl_easy_cleanup")?),
            strerror: std::mem::transmute::<P, unsafe extern "C" fn(c_int) -> *const c_char>(sym(
                c"curl_easy_strerror",
            )?),
            tls: check_tls(),
        })
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// Whether downloads can work here at all: libcurl is there, or for a
/// static program, the helper beside it.
pub fn available() -> bool {
    if STATIC {
        helper().is_some()
    } else {
        curl().is_ok()
    }
}

/// What downloads go through, for the Plugins place's line: "libcurl", or
/// for a static program the helper.
pub fn means() -> &'static str {
    if STATIC {
        HELPER
    } else {
        "libcurl"
    }
}

/// The helper beside this program, if it is there.
fn helper() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.with_file_name(HELPER);
    p.is_file().then_some(p)
}

/// What libcurl's TLS library said as it started, for the log: for example
/// "OpenSSL 3.0.13 ready". None without libcurl. A static program never
/// opens libcurl, so it asks the helper beside it, once
/// ([`tls_through_helper`]); None without the helper.
pub fn tls() -> Option<String> {
    if STATIC {
        static ASKED: OnceLock<Option<String>> = OnceLock::new();
        return ASKED
            .get_or_init(|| helper().map(|h| tls_through_helper(&h, TLS_WAIT)))
            .clone();
    }
    curl().ok().map(|c| c.tls.clone())
}

/// How long the helper may take to start libcurl and say what its OpenSSL
/// said.
pub const TLS_WAIT: Duration = Duration::from_secs(10);

/// Asks the helper at `helper` what its libcurl's OpenSSL said as it
/// started (`mpfree-fetch --tls`, which downloads nothing): its account, as
/// [`tls`] gives it in a dynamic program, or "TLS not checked: " and why
/// (the helper's own reason, it gave no answer within `wait`, or it
/// stopped). A helper still running after `wait` is stopped.
pub fn tls_through_helper(helper: &Path, wait: Duration) -> String {
    let mut child = match Command::new(helper)
        .arg("--tls")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return format!("TLS not checked: {}: {e}", helper.display()),
    };
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) if start.elapsed() < wait => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return format!(
                    "TLS not checked: {HELPER} gave no answer in {} s",
                    wait.as_secs_f32()
                );
            }
            Err(e) => break Err(e),
        }
    };
    // One short line: it fits the pipe, so the helper never waited on it.
    let mut text = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut out, &mut text);
    }
    for line in text.lines() {
        if let Some(account) = line.strip_prefix("tls ") {
            return account.to_string();
        }
        if let Some(why) = line.strip_prefix("failed ") {
            return format!("TLS not checked: {why}");
        }
    }
    match status {
        Ok(s) => format!("TLS not checked: {HELPER} stopped ({s})"),
        Err(e) => format!("TLS not checked: {HELPER}: {e}"),
    }
}

// OpenSSL's numbers (openssl/crypto.h).
const OPENSSL_VERSION: c_int = 0;
const OPENSSL_INFO_CONFIG_DIR: c_int = 1001;
const OPENSSL_INFO_SEED_SOURCE: c_int = 1007;

/// Checks that libcurl's OpenSSL can make random numbers, which TLS needs
/// before it sends anything; without them libcurl says only "Insufficient
/// randomness". The usual cause is a system `openssl.cnf` that turns on a
/// provider (the legacy one, say) but not the default one, which then never
/// loads: OpenSSL then has no random number generator at all. This loads the
/// default provider then, as OpenSSL does itself when a config turns on
/// none. For any other cause it says what OpenSSL said, where it seeds from
/// and whether the kernel gives this process random bytes. Only reads.
fn check_tls() -> String {
    unsafe {
        // libcurl has opened it already: RTLD_NOLOAD finds that copy and
        // never opens another.
        let h = libc::dlopen(
            c"libcrypto.so.3".as_ptr(),
            libc::RTLD_NOW | libc::RTLD_LOCAL | libc::RTLD_NOLOAD,
        );
        if h.is_null() {
            return "TLS not through OpenSSL 3".into();
        }
        let sym = |n: &CStr| {
            let p = libc::dlsym(h, n.as_ptr());
            (!p.is_null()).then_some(p)
        };
        type P = *mut c_void;
        let (
            Some(version),
            Some(info),
            Some(status),
            Some(get_error),
            Some(error_string),
            Some(load),
        ) = (
            sym(c"OpenSSL_version"),
            sym(c"OPENSSL_info"),
            sym(c"RAND_status"),
            sym(c"ERR_get_error"),
            sym(c"ERR_error_string_n"),
            sym(c"OSSL_PROVIDER_load"),
        )
        else {
            return "OpenSSL 3 without the functions MPFree checks it with".into();
        };
        let version =
            std::mem::transmute::<P, unsafe extern "C" fn(c_int) -> *const c_char>(version);
        let info = std::mem::transmute::<P, unsafe extern "C" fn(c_int) -> *const c_char>(info);
        let status = std::mem::transmute::<P, unsafe extern "C" fn() -> c_int>(status);
        let get_error = std::mem::transmute::<P, unsafe extern "C" fn() -> c_ulong>(get_error);
        let error_string = std::mem::transmute::<
            P,
            unsafe extern "C" fn(c_ulong, *mut c_char, usize),
        >(error_string);
        let load = std::mem::transmute::<P, unsafe extern "C" fn(P, *const c_char) -> P>(load);
        let text = |p: *const c_char| {
            if p.is_null() {
                "?".to_string()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        // OpenSSL's errors on this thread, oldest first, at most 4.
        let errors = || {
            let mut said = Vec::new();
            loop {
                let e = get_error();
                if e == 0 {
                    break;
                }
                if said.len() < 4 {
                    let mut b = [0 as c_char; 256];
                    error_string(e, b.as_mut_ptr(), b.len());
                    said.push(text(b.as_ptr()));
                }
            }
            if said.is_empty() {
                "no error said".to_string()
            } else {
                said.join("; ")
            }
        };
        let name = text(version(OPENSSL_VERSION));
        let name = name.split(' ').take(2).collect::<Vec<_>>().join(" ");
        errors();
        if status() == 1 {
            return format!("{name} ready");
        }
        let first = errors();
        if !load(std::ptr::null_mut(), c"default".as_ptr()).is_null() && status() == 1 {
            return format!(
                "{name} ready (its random numbers didn't start: {first}; MPFree loaded its default provider)"
            );
        }
        let then = errors();
        let mut b = [0u8; 16];
        let kernel =
            match libc::getrandom(b.as_mut_ptr() as *mut c_void, b.len(), libc::GRND_NONBLOCK) {
                16 => "works".to_string(),
                n => format!("gave {n} ({})", std::io::Error::last_os_error()),
            };
        format!(
            "{name} not ready: its random numbers can't start ({first}; with its default provider: {then}); \
             it seeds from {}, its config is in {}, the kernel's getrandom {kernel}",
            text(info(OPENSSL_INFO_SEED_SOURCE)),
            text(info(OPENSSL_INFO_CONFIG_DIR)),
        )
    }
}

struct Sink<'a> {
    file: File,
    written: u64,
    max: u64,
    failed: Option<String>,
    progress: &'a mut dyn FnMut(u64, u64) -> bool,
}

extern "C" fn on_data(ptr: *const c_char, size: usize, n: usize, data: *mut c_void) -> usize {
    let s = unsafe { &mut *(data as *mut Sink) };
    let len = size.saturating_mul(n);
    if s.written + len as u64 > s.max {
        s.failed = Some(format!("it is bigger than {} MB", s.max >> 20));
        return 0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    if let Err(e) = s.file.write_all(bytes) {
        s.failed = Some(e.to_string());
        return 0;
    }
    s.written += len as u64;
    len
}

extern "C" fn on_progress(data: *mut c_void, total: i64, now: i64, _: i64, _: i64) -> c_int {
    let s = unsafe { &mut *(data as *mut Sink) };
    if (s.progress)(now.max(0) as u64, total.max(0) as u64) {
        0
    } else {
        s.failed = Some("cancelled".into());
        1
    }
}

/// Downloads `url` into the file `out` (made anew), at most `max` bytes.
/// `progress(done, total)` is called now and then (total is 0 while it is
/// unknown); returning false cancels. Returns the bytes written. On any
/// failure the file is removed.
pub fn fetch(
    url: &str,
    out: &Path,
    max: u64,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<u64, String> {
    if !allowed(url) {
        return Err(format!("{url} is not on the catalog's sites"));
    }
    if STATIC {
        let helper = helper().ok_or_else(|| {
            format!("{HELPER} is not beside MPFree's program, so MPFree can't download")
        })?;
        return through_helper(&helper, url, out, max, progress);
    }
    fetch_any(url, out, max, progress, PROTO_HTTPS)
}

/// [`fetch`] in this process, whatever the build: the helper's own path.
pub fn fetch_here(
    url: &str,
    out: &Path,
    max: u64,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<u64, String> {
    if !allowed(url) {
        return Err(format!("{url} is not on the catalog's sites"));
    }
    fetch_any(url, out, max, progress, PROTO_HTTPS)
}

/// The helper's whole program: `mpfree-fetch URL OUT MAX`. It prints
/// `progress DONE TOTAL` now and then (at most ten times a second), then
/// `done BYTES` or `failed WHY`, and exits 0 or 1. When its output is gone
/// (the program that started it went), it stops the download.
pub fn helper_main(args: &[String]) -> i32 {
    let mut stdout = std::io::stdout();
    if let [flag] = args {
        if flag == "--tls" {
            return helper_tls(&mut stdout);
        }
    }
    let (url, out, max) = match args {
        [url, out, max] => match max.parse::<u64>() {
            Ok(max) => (url, Path::new(out), max),
            Err(_) => {
                let _ = writeln!(stdout, "failed the most to download is not a number");
                return 2;
            }
        },
        _ => {
            let _ = writeln!(
                stdout,
                "failed usage: {HELPER} URL OUT MAX, or {HELPER} --tls"
            );
            return 2;
        }
    };
    if STATIC {
        let _ = writeln!(
            stdout,
            "failed this {HELPER} is linked statically, so it can't open libcurl (build it as the docs say)"
        );
        return 1;
    }
    let mut last: Option<std::time::Instant> = None;
    let mut progress = |done: u64, total: u64| {
        if last.is_some_and(|t| t.elapsed().as_millis() < 100) {
            return true;
        }
        last = Some(std::time::Instant::now());
        writeln!(stdout, "progress {done} {total}").is_ok() && stdout.flush().is_ok()
    };
    let r = fetch_here(url, out, max, &mut progress);
    let mut stdout = std::io::stdout();
    match r {
        Ok(n) => {
            let _ = writeln!(stdout, "done {n}");
            0
        }
        Err(e) => {
            let _ = writeln!(stdout, "failed {}", e.replace('\n', " "));
            1
        }
    }
}

/// `mpfree-fetch --tls`: starts libcurl and prints `tls ` and what its
/// OpenSSL said ([`tls`]), or `failed ` and why libcurl didn't start.
fn helper_tls(stdout: &mut std::io::Stdout) -> i32 {
    if STATIC {
        let _ = writeln!(
            stdout,
            "failed this {HELPER} is linked statically, so it can't open libcurl (build it as the docs say)"
        );
        return 1;
    }
    match curl() {
        Ok(c) => {
            let _ = writeln!(stdout, "tls {}", c.tls.replace('\n', " "));
            0
        }
        Err(e) => {
            let _ = writeln!(stdout, "failed {}", e.replace('\n', " "));
            1
        }
    }
}

/// [`fetch`] through the helper at `helper`: its progress lines go to
/// `progress`, and returning false stops it. On any failure the file is
/// removed, as [`fetch`] does.
pub fn through_helper(
    helper: &Path,
    url: &str,
    out: &Path,
    max: u64,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<u64, String> {
    let mut child = Command::new(helper)
        .arg(url)
        .arg(out)
        .arg(max.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", helper.display()))?;
    let mut result = None;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let (word, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            match word {
                "progress" => {
                    let mut n = rest.split(' ').map(|v| v.parse::<u64>().unwrap_or(0));
                    let (done, total) = (n.next().unwrap_or(0), n.next().unwrap_or(0));
                    if !progress(done, total) {
                        let _ = child.kill();
                        result = Some(Err("cancelled".to_string()));
                        break;
                    }
                }
                "done" => result = Some(rest.parse::<u64>().map_err(|_| line.clone())),
                "failed" => result = Some(Err(rest.to_string())),
                _ => {}
            }
        }
    }
    let status = child.wait();
    let result = match (result, status) {
        (Some(Ok(n)), Ok(s)) if s.success() => Ok(n),
        (Some(Err(e)), _) => Err(e),
        (_, Ok(s)) => Err(format!("{HELPER} stopped ({s})")),
        (_, Err(e)) => Err(format!("{HELPER}: {e}")),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result
}

fn fetch_any(
    url: &str,
    out: &Path,
    max: u64,
    progress: &mut dyn FnMut(u64, u64) -> bool,
    protocols: c_long,
) -> Result<u64, String> {
    let c = curl()?;
    let curl_url = CString::new(url).map_err(|_| "a bad address".to_string())?;
    let file = File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut sink = Sink {
        file,
        written: 0,
        max,
        failed: None,
        progress,
    };
    let mut err = [0 as c_char; ERROR_SIZE];
    let code = unsafe {
        let h = (c.init)();
        if h.is_null() {
            return Err("libcurl couldn't start a download".into());
        }
        let long = |o: c_int, v: c_long| (c.setopt)(h, o, v);
        let mut bad = 0;
        bad |= (c.setopt)(h, OPT_URL, curl_url.as_ptr());
        bad |= (c.setopt)(h, OPT_USERAGENT, c"MPFree".as_ptr());
        bad |= (c.setopt)(h, OPT_ERRORBUFFER, err.as_mut_ptr());
        bad |= (c.setopt)(
            h,
            OPT_WRITEFUNCTION,
            on_data as extern "C" fn(*const c_char, usize, usize, *mut c_void) -> usize,
        );
        bad |= (c.setopt)(h, OPT_WRITEDATA, &mut sink as *mut Sink as *mut c_void);
        bad |= (c.setopt)(
            h,
            OPT_XFERINFOFUNCTION,
            on_progress as extern "C" fn(*mut c_void, i64, i64, i64, i64) -> c_int,
        );
        bad |= (c.setopt)(h, OPT_XFERINFODATA, &mut sink as *mut Sink as *mut c_void);
        bad |= long(OPT_NOPROGRESS, 0);
        bad |= long(OPT_FAILONERROR, 1);
        bad |= long(OPT_FOLLOWLOCATION, 1);
        bad |= long(OPT_MAXREDIRS, 8);
        bad |= long(OPT_CONNECTTIMEOUT, 20);
        // Give up when under 1 KB/s for 30 s.
        bad |= long(OPT_LOW_SPEED_LIMIT, 1024);
        bad |= long(OPT_LOW_SPEED_TIME, 30);
        bad |= long(OPT_NOSIGNAL, 1);
        bad |= long(OPT_MAXFILESIZE, max.min(c_long::MAX as u64) as c_long);
        bad |= long(OPT_PROTOCOLS, protocols);
        bad |= long(OPT_REDIR_PROTOCOLS, PROTO_HTTPS);
        let code = if bad != 0 { -1 } else { (c.perform)(h) };
        (c.cleanup)(h);
        code
    };
    let result = if code == 0 {
        sink.file
            .sync_all()
            .map(|_| sink.written)
            .map_err(|e| format!("{}: {e}", out.display()))
    } else if let Some(why) = sink.failed.take() {
        Err(why)
    } else if code == -1 {
        Err("this libcurl is too old for MPFree's settings".into())
    } else {
        let detail = unsafe { CStr::from_ptr(err.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let general = unsafe { CStr::from_ptr((c.strerror)(code)) }
            .to_string_lossy()
            .into_owned();
        let why = if detail.is_empty() { general } else { detail };
        // libcurl's whole account of OpenSSL failing to make random numbers.
        Err(if why.contains("randomness") {
            format!("{why} ({})", check_tls())
        } else {
            why
        })
    };
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    #[test]
    fn only_https_on_the_catalogs_hosts() {
        assert!(allowed("https://open-mpc.github.io/catalog.json"));
        assert!(allowed("https://github.com/a/b/releases/download/v1/x.zip"));
        assert!(!allowed("http://github.com/x.zip"));
        assert!(!allowed("https://github.com.evil.example/x.zip"));
        assert!(!allowed("https://evil.example/?https://github.com/"));
        assert!(!allowed("file:///etc/passwd"));
        let out = std::env::temp_dir().join("mpfree-mpcstore-never");
        assert!(fetch("https://example.com/x", &out, 10, &mut |_, _| true).is_err());
        assert!(!out.exists());
    }

    #[test]
    fn the_tls_library_is_checked_as_libcurl_starts() {
        let Some(tls) = tls() else {
            eprintln!("no libcurl on this machine: not exercised");
            return;
        };
        // A workstation's OpenSSL makes random numbers at once.
        assert!(
            tls == "TLS not through OpenSSL 3"
                || (tls.starts_with("OpenSSL 3") && tls.ends_with(" ready")),
            "{tls}"
        );
    }

    /// One HTTP answer on a local port, for the libcurl path (plain HTTP:
    /// the TLS is libcurl's own).
    fn serve(body: Vec<u8>, status: &'static str) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        format!("http://{addr}/x.zip")
    }

    /// A stand-in for the helper: a shell script that says `lines` and
    /// writes `body` to its OUT argument.
    fn fake_helper(dir: &Path, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("fake-fetch");
        std::fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn a_static_build_downloads_through_the_helper() {
        let dir = std::env::temp_dir().join(format!("mpfree-fetch-helper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("a.zip");
        let url = "https://github.com/a/b/releases/download/v1/x.zip";
        // It reports progress and the bytes, and the file is its.
        let h = fake_helper(
            &dir,
            "[ \"$1\" = https://github.com/a/b/releases/download/v1/x.zip ] && [ \"$3\" = 4096 ] || exit 9\n\
             echo progress 0 0; printf zip > \"$2\"; echo progress 3 3; echo done 3",
        );
        let mut seen = Vec::new();
        let n = through_helper(&h, url, &out, 4096, &mut |d, t| {
            seen.push((d, t));
            true
        })
        .unwrap();
        assert_eq!(n, 3);
        assert_eq!(seen, [(0, 0), (3, 3)]);
        assert_eq!(std::fs::read(&out).unwrap(), b"zip");
        // Its reason, and no file left.
        let h = fake_helper(
            &dir,
            "printf half > \"$2\"; echo failed Could not resolve host; exit 1",
        );
        let e = through_helper(&h, url, &out, 4096, &mut |_, _| true).unwrap_err();
        assert_eq!(e, "Could not resolve host");
        assert!(!out.exists());
        // Cancelled: the helper is stopped, and no file left.
        let h = fake_helper(
            &dir,
            "printf half > \"$2\"; while :; do echo progress 1 10; sleep 0.05; done",
        );
        let e = through_helper(&h, url, &out, 4096, &mut |_, _| false).unwrap_err();
        assert_eq!(e, "cancelled");
        assert!(!out.exists());
        // A helper that dies says so.
        let h = fake_helper(&dir, "printf half > \"$2\"; kill -SEGV $$");
        let e = through_helper(&h, url, &out, 4096, &mut |_, _| true).unwrap_err();
        assert!(e.starts_with("mpfree-fetch stopped ("), "{e}");
        assert!(!out.exists());
        let e = through_helper(&dir.join("none"), url, &out, 4096, &mut |_, _| true).unwrap_err();
        assert!(e.contains("none"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_static_build_asks_the_helper_what_its_openssl_said() {
        let dir = std::env::temp_dir().join(format!("mpfree-fetch-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wait = Duration::from_secs(5);
        // Its account, asked with --tls.
        let h = fake_helper(
            &dir,
            "[ \"$1\" = --tls ] || exit 9\necho tls OpenSSL 3.5.4 ready",
        );
        assert_eq!(tls_through_helper(&h, wait), "OpenSSL 3.5.4 ready");
        // Its reason when libcurl didn't start.
        let h = fake_helper(&dir, "echo failed libcurl.so.4 not found; exit 1");
        assert_eq!(
            tls_through_helper(&h, wait),
            "TLS not checked: libcurl.so.4 not found"
        );
        // A helper that dies, or says nothing.
        let h = fake_helper(&dir, "kill -SEGV $$");
        let said = tls_through_helper(&h, wait);
        assert!(
            said.starts_with("TLS not checked: mpfree-fetch stopped ("),
            "{said}"
        );
        // One that takes too long is stopped.
        let h = fake_helper(&dir, "sleep 5; echo tls late");
        let start = Instant::now();
        assert_eq!(
            tls_through_helper(&h, Duration::from_millis(200)),
            "TLS not checked: mpfree-fetch gave no answer in 0.2 s"
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        let said = tls_through_helper(&dir.join("none"), wait);
        assert!(
            said.starts_with("TLS not checked: ") && said.contains("none"),
            "{said}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn downloads_into_a_file_with_progress_and_limits() {
        if !available() {
            eprintln!("no libcurl on this machine: the download path is not exercised");
            return;
        }
        let dir = std::env::temp_dir().join(format!("mpfree-fetch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("a.zip");
        let body: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        let url = serve(body.clone(), "200 OK");
        let mut last = (0, 0);
        let n = fetch_any(
            &url,
            &out,
            1 << 20,
            &mut |d, t| {
                last = (d, t);
                true
            },
            PROTO_HTTP,
        )
        .unwrap();
        assert_eq!(n, body.len() as u64);
        assert_eq!(std::fs::read(&out).unwrap(), body);
        assert_eq!(last.0, body.len() as u64);

        let url = serve(body.clone(), "200 OK");
        let e = fetch_any(&url, &out, 1000, &mut |_, _| true, PROTO_HTTP).unwrap_err();
        assert!(!out.exists(), "a failed download leaves no file ({e})");

        let url = serve(b"gone".to_vec(), "404 Not Found");
        let e = fetch_any(&url, &out, 1 << 20, &mut |_, _| true, PROTO_HTTP).unwrap_err();
        assert!(e.contains("404"), "{e}");

        let url = serve(body, "200 OK");
        let e = fetch_any(&url, &out, 1 << 20, &mut |_, _| false, PROTO_HTTP).unwrap_err();
        assert_eq!(e, "cancelled");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
