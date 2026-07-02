//! Request-level network hardening for the editor server.
//!
//! The server exposes unauthenticated read/write access to the machine's
//! files, so even on loopback two browser-borne attacks must be shut out:
//!
//! * **DNS rebinding** — a page on `evil.com` re-resolves its own hostname to
//!   `127.0.0.1` and talks to us with the browser's full network stack. Its
//!   requests still carry `Host: evil.com`, so validating the Host header
//!   (any port) closes the hole.
//! * **CSRF** — a foreign page posts to `/api/upload` (raw body, no CORS
//!   preflight) or another state-changing endpoint. Browsers attach an
//!   `Origin` header to such requests; if one is present and isn't this
//!   server itself, the request is refused. Requests without an `Origin`
//!   (curl, native code) are allowed — they are not made by a victim browser.

use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// What Host/Origin values this server instance answers to.
pub(super) struct NetPolicy {
    /// Extra bare hosts (lowercase, port stripped) accepted besides loopback —
    /// the `--host` value when `--allow-remote` is on.
    extra_hosts: Vec<String>,
    /// With `--allow-remote`, any syntactically valid IP-literal Host is also
    /// accepted (LAN clients dial the bind address directly, possibly via an
    /// interface IP we cannot enumerate for 0.0.0.0 binds). DNS *names* other
    /// than localhost stay refused — a rebinding attack needs a name.
    allow_remote: bool,
}

impl NetPolicy {
    /// Loopback-only policy: `localhost`, `127.0.0.0/8` and `::1`, any port.
    pub(super) fn loopback() -> NetPolicy {
        NetPolicy {
            extra_hosts: Vec::new(),
            allow_remote: false,
        }
    }

    /// Policy for an explicit `--allow-remote` bind of `bind_host`.
    pub(super) fn remote(bind_host: &str) -> NetPolicy {
        NetPolicy {
            extra_hosts: bare_host(bind_host).into_iter().collect(),
            allow_remote: true,
        }
    }

    /// Is this Host-header value (`name`, `name:port`, `[v6]:port`, …) ours?
    pub(super) fn host_allowed(&self, value: &str) -> bool {
        let Some(host) = bare_host(value) else {
            return false;
        };
        if host == "localhost" {
            return true;
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            if ip.is_loopback() || self.allow_remote {
                return true;
            }
        }
        self.extra_hosts.iter().any(|e| e.as_str() == host)
    }

    /// Is this Origin-header value one of our own origins? `null` and every
    /// non-http(s) scheme are foreign by definition.
    pub(super) fn origin_allowed(&self, value: &str) -> bool {
        let v = value.trim();
        let Some(rest) = v
            .strip_prefix("http://")
            .or_else(|| v.strip_prefix("https://"))
        else {
            return false;
        };
        // An Origin has no path, but be tolerant of a stray trailing slash.
        self.host_allowed(rest.trim_end_matches('/'))
    }
}

/// Extract the bare, lowercased host from a Host-header-shaped value: strips
/// one `:port` suffix and IPv6 brackets. `None` when the value cannot be a
/// host at all.
fn bare_host(value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    // Bracketed IPv6, optionally with a port: "[::1]" or "[::1]:8777".
    if let Some(rest) = v.strip_prefix('[') {
        let end = rest.find(']')?;
        let (inside, tail) = (&rest[..end], &rest[end + 1..]);
        let tail_ok = tail.is_empty()
            || (tail.len() > 1
                && tail.starts_with(':')
                && tail[1..].chars().all(|c| c.is_ascii_digit()));
        if !tail_ok || inside.is_empty() {
            return None;
        }
        return Some(inside.to_ascii_lowercase());
    }
    // A bare IPv6 literal cannot carry a port without brackets.
    if v.parse::<std::net::Ipv6Addr>().is_ok() {
        return Some(v.to_ascii_lowercase());
    }
    let host = match v.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => h,
        Some(_) => return None, // "host:" or a non-numeric port
        None => v,
    };
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// Axum middleware enforcing the policy on every request.
pub(super) async fn guard(
    State(policy): State<Arc<NetPolicy>>,
    req: Request,
    next: Next,
) -> Response {
    if let Err(resp) = check_headers(&policy, req.method(), req.headers()) {
        return *resp;
    }
    next.run(req).await
}

/// The pure part of the guard, shared with the unit tests.
fn check_headers(
    policy: &NetPolicy,
    method: &Method,
    headers: &HeaderMap,
) -> Result<(), Box<Response>> {
    // DNS-rebinding protection: a browser reaching us through a foreign name
    // still says so in Host. (A missing Host means a non-browser client —
    // browsers always send it — so nothing to protect there.)
    if let Some(host) = headers.get(header::HOST) {
        let ok = host
            .to_str()
            .map(|h| policy.host_allowed(h))
            .unwrap_or(false);
        if !ok {
            return Err((
                StatusCode::FORBIDDEN,
                format!(
                    "Host '{}' is not this server (DNS-rebinding protection) — use the address it was started on",
                    host.to_str().unwrap_or("<invalid>")
                ),
            )
                .into_response()
                .into());
        }
    }
    // CSRF protection for state-changing requests: an Origin that is present
    // and foreign means a cross-site browser request. Absent Origin = curl /
    // native client → allowed.
    if !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        if let Some(origin) = headers.get(header::ORIGIN) {
            let ok = origin
                .to_str()
                .map(|o| policy.origin_allowed(o))
                .unwrap_or(false);
            if !ok {
                return Err((
                    StatusCode::FORBIDDEN,
                    format!(
                        "cross-origin request blocked (Origin '{}' is not this server)",
                        origin.to_str().unwrap_or("<invalid>")
                    ),
                )
                    .into_response()
                    .into());
            }
        }
    }
    Ok(())
}

/// Is this `--host` value loopback? (Used by `cmd_serve` to decide whether
/// `--allow-remote` is required.) Note the listener parses the host as an IP,
/// so names other than the well-known "localhost" never bind anyway.
pub(super) fn host_is_loopback(host: &str) -> bool {
    let h = host.trim();
    if h.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = h
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .unwrap_or(h);
    bare.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_policy_accepts_local_hosts_any_port() {
        let p = NetPolicy::loopback();
        for host in [
            "localhost",
            "LOCALHOST:8777",
            "localhost:1",
            "127.0.0.1",
            "127.0.0.1:9999",
            "127.0.0.2:80", // whole 127/8 block is loopback
            "[::1]",
            "[::1]:8777",
            "::1", // tolerated bare v6
        ] {
            assert!(p.host_allowed(host), "should allow {host}");
        }
    }

    #[test]
    fn loopback_policy_rejects_foreign_hosts() {
        let p = NetPolicy::loopback();
        for host in [
            "evil.com",
            "evil.com:8777",
            "127.0.0.1.evil.com", // suffix trick
            "localhost.evil.com",
            "192.168.1.5:8777", // non-loopback IP without --allow-remote
            "[2001:db8::1]:80",
            "",
            "localhost:notaport",
            "localhost:",
            "[::1", // malformed bracket
        ] {
            assert!(!p.host_allowed(host), "should reject {host}");
        }
    }

    #[test]
    fn remote_policy_accepts_ip_literals_but_not_dns_names() {
        let p = NetPolicy::remote("0.0.0.0");
        assert!(p.host_allowed("192.168.1.5:8777"));
        assert!(p.host_allowed("10.0.0.7"));
        assert!(p.host_allowed("[2001:db8::1]:8777"));
        assert!(p.host_allowed("127.0.0.1:8777"));
        // Rebinding needs a DNS name; names stay blocked even with the flag.
        assert!(!p.host_allowed("evil.com:8777"));
        // The configured bind host itself is allowed verbatim.
        let p2 = NetPolicy::remote("192.168.1.5");
        assert!(p2.host_allowed("192.168.1.5:8777"));
    }

    #[test]
    fn origins_of_this_server_are_allowed_foreign_ones_are_not() {
        let p = NetPolicy::loopback();
        assert!(p.origin_allowed("http://localhost:8777"));
        assert!(p.origin_allowed("http://127.0.0.1:8777"));
        assert!(p.origin_allowed("https://127.0.0.1"));
        assert!(p.origin_allowed("http://[::1]:8777"));
        assert!(!p.origin_allowed("http://evil.com"));
        assert!(!p.origin_allowed("https://evil.com:8777"));
        assert!(!p.origin_allowed("null"));
        assert!(!p.origin_allowed("file://"));
        assert!(!p.origin_allowed("chrome-extension://abc"));
        assert!(!p.origin_allowed(""));
    }

    #[test]
    fn check_headers_enforces_host_for_all_methods_and_origin_for_writes() {
        let p = NetPolicy::loopback();
        let mk = |host: Option<&str>, origin: Option<&str>| {
            let mut h = HeaderMap::new();
            if let Some(v) = host {
                h.insert(header::HOST, v.parse().unwrap());
            }
            if let Some(v) = origin {
                h.insert(header::ORIGIN, v.parse().unwrap());
            }
            h
        };
        // Good host, GET: fine.
        assert!(check_headers(&p, &Method::GET, &mk(Some("127.0.0.1:1"), None)).is_ok());
        // Rebinding host: refused even for GET.
        assert!(check_headers(&p, &Method::GET, &mk(Some("evil.com"), None)).is_err());
        // POST with our own origin: fine.
        assert!(check_headers(
            &p,
            &Method::POST,
            &mk(Some("localhost:8777"), Some("http://localhost:8777"))
        )
        .is_ok());
        // POST with a foreign origin: refused (CSRF).
        assert!(check_headers(
            &p,
            &Method::POST,
            &mk(Some("localhost:8777"), Some("http://evil.com"))
        )
        .is_err());
        // GET with a foreign origin: reads are CORS-protected by the browser
        // itself; we only block state-changing methods.
        assert!(check_headers(
            &p,
            &Method::GET,
            &mk(Some("localhost:8777"), Some("http://evil.com"))
        )
        .is_ok());
        // POST without Origin (curl): allowed.
        assert!(check_headers(&p, &Method::POST, &mk(Some("127.0.0.1"), None)).is_ok());
        // No Host at all (non-browser): allowed.
        assert!(check_headers(&p, &Method::POST, &mk(None, None)).is_ok());
    }

    #[test]
    fn host_is_loopback_matches_bind_syntax() {
        assert!(host_is_loopback("127.0.0.1"));
        assert!(host_is_loopback("localhost"));
        assert!(host_is_loopback("::1"));
        assert!(host_is_loopback("[::1]"));
        assert!(!host_is_loopback("0.0.0.0"));
        assert!(!host_is_loopback("::"));
        assert!(!host_is_loopback("192.168.1.5"));
    }
}
