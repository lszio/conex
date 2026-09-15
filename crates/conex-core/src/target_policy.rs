//! Outbound address admission. No DNS here; the caller supplies resolved IPs.
use std::net::IpAddr;

use conex_proto::v1;

use crate::types::{AllowedTarget, CallError, CallResult, Caller, Target};

pub struct TargetPolicy;

impl TargetPolicy {
    pub fn new() -> Self {
        Self
    }

    /// Validate already-resolved addresses and pin exactly one dial address.
    /// The target origin is the only URL source; no userinfo, path or dynamic
    /// query is accepted here.
    pub fn allow(
        &self,
        _caller: &Caller,
        target: &Target,
        resolved: &[IpAddr],
    ) -> CallResult<AllowedTarget> {
        let (scheme, host, port) = parse_origin(&target.origin)?;
        validate_fixed_path(&target.fixed_path)?;

        if scheme == "http" && !target.allow_loopback_http {
            return Err(forbidden("plaintext http requires allowLoopbackHttp"));
        }

        let mut candidates: Vec<IpAddr> = resolved
            .iter()
            .copied()
            .filter(|ip| permitted_address(*ip, &scheme, target.allow_loopback_http))
            .collect();
        if !target.allowed_addresses.is_empty() {
            candidates.retain(|ip| target.allowed_addresses.contains(ip));
        }
        let pinned = *candidates
            .first()
            .ok_or_else(|| forbidden("no permitted address after admission"))?;

        if scheme == "http" && !pinned.is_loopback() {
            return Err(forbidden("plaintext http is only allowed to loopback"));
        }
        if pinned.is_loopback() && !target.allow_loopback_http {
            return Err(forbidden("loopback address requires allowLoopbackHttp"));
        }

        let audience = target
            .tls_trust
            .expected_server_name
            .clone()
            .unwrap_or_else(|| host.clone());

        Ok(AllowedTarget {
            target_id: target.id.clone(),
            audience: audience.clone(),
            scheme,
            hostname: host,
            server_name: audience,
            port,
            pinned_address: std::net::SocketAddr::new(pinned, port),
            fixed_path: target.fixed_path.clone(),
            allow_loopback_http: target.allow_loopback_http,
        })
    }
}

impl Default for TargetPolicy {
    fn default() -> Self {
        Self::new()
    }
}

fn forbidden(message: &str) -> CallError {
    CallError::new(v1::ErrorCode::Forbidden, message)
}

fn parse_origin(origin: &str) -> CallResult<(String, String, u16)> {
    let (scheme, rest) = origin
        .split_once("://")
        .ok_or_else(|| forbidden("origin must be scheme://host[:port]"))?;
    if scheme != "https" && scheme != "http" {
        return Err(forbidden("origin scheme must be https or http"));
    }
    if rest.is_empty()
        || rest.contains('@')
        || rest.contains('/')
        || rest.contains('?')
        || rest.contains('#')
    {
        return Err(forbidden(
            "origin must not contain userinfo, path, query or fragment",
        ));
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
            let port = p.parse::<u16>().map_err(|_| forbidden("invalid port"))?;
            (h.to_string(), port)
        }
        _ => (rest.to_string(), if scheme == "https" { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(forbidden("origin host must not be empty"));
    }
    Ok((scheme.to_string(), host, port))
}

fn validate_fixed_path(path: &str) -> CallResult<()> {
    if !path.starts_with('/') {
        return Err(forbidden("fixedPath must be absolute"));
    }
    if path.contains('?') || path.contains('#') || path.contains('{') || path.contains('}') {
        return Err(forbidden("fixedPath must be static"));
    }
    Ok(())
}

fn permitted_address(ip: IpAddr, scheme: &str, allow_loopback: bool) -> bool {
    if ip.is_loopback() {
        return allow_loopback;
    }
    let _ = scheme;
    is_public(ip)
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.octets()[0] == 0)
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            let unique_local = (segments[0] & 0xfe00) == 0xfc00;
            let link_local = (segments[0] & 0xffc0) == 0xfe80;
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || unique_local
                || link_local)
        }
    }
}
