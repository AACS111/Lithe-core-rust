//! The endpoint a `tcpAnnounced` adapter prints on its own output.
//!
//! The host reads the adapter's output line by line and asks Core about each
//! one until a line announces the endpoint, then connects to it. Core decides
//! which line is the announcement and which endpoints are acceptable, so every
//! platform host reads the same announcements the same way. Core never reads
//! the process output, opens a socket, resolves a name or keeps a clock: the
//! host bounds the wait with the transport's discovery deadline.

use super::*;
use lithe_protocol::{CoreError, ErrorCode};

/// Longest endpoint text after the prefix, in bytes: room for a bracketed IPv6
/// address and a port, with margin.
const MAX_ANNOUNCED_ENDPOINT: usize = 128;

/// Where a `port` announcement listens: adapters that print only a port listen
/// on the IPv4 loopback address.
const ANNOUNCED_PORT_HOST: &str = "127.0.0.1";

/// `debug.adapters.readAnnouncement`: whether one output line of a
/// `tcpAnnounced` adapter announces its endpoint.
///
/// The payload is `{ endpoint, stream, line }`: the `endpoint` of the
/// adapter's launch plan, the stream the line came from, and the line without
/// its line break. A line on another stream, or one without the prefix, is
/// `{ status: "none" }`. A line with the prefix is the announcement: the rest
/// of the line after the prefix's first occurrence, without surrounding
/// whitespace, is `{ status: "announced", host, port }` when it is a loopback
/// endpoint, and `{ status: "invalid", reason }` otherwise. A malformed
/// payload fails with `invalid_request`.
pub fn read_announcement(payload: Value) -> Result<Value, CoreError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        endpoint: DebugEndpoint,
        stream: String,
        line: String,
    }
    let request: Request = serde_json::from_value(payload).map_err(|error| {
        CoreError::new(ErrorCode::InvalidRequest, "Invalid announcement request.")
            .with_details(error.to_string())
    })?;
    if !request.endpoint.is_announced() {
        return Err(CoreError::new(
            ErrorCode::InvalidRequest,
            "An announced endpoint names a stream, a one-line prefix, and port or hostPort.",
        ));
    }
    if !matches!(request.stream.as_str(), "stdout" | "stderr") {
        return Err(CoreError::new(
            ErrorCode::InvalidRequest,
            "A line comes from stdout or stderr.",
        ));
    }
    let endpoint = request.endpoint;
    let prefix = endpoint.prefix.as_deref().unwrap_or_default();
    if endpoint.stream.as_deref() != Some(request.stream.as_str()) {
        return Ok(json!({ "status": "none" }));
    }
    let Some((_, rest)) = request.line.split_once(prefix) else {
        return Ok(json!({ "status": "none" }));
    };
    let announced = rest.trim();
    let parsed = match endpoint.value.as_deref() {
        Some("port") => announced_port(announced).map(|port| (ANNOUNCED_PORT_HOST, port)),
        _ => announced_host_port(announced),
    };
    Ok(match parsed {
        Err(reason) => json!({ "status": "invalid", "reason": reason }),
        Ok((host, port)) => json!({ "status": "announced", "host": host, "port": port }),
    })
}

/// Why an announcement is not an endpoint the host may connect to.
type Refusal = &'static str;

/// The text is not a port, or not `host:port` with a valid host.
const MALFORMED: Refusal = "malformed";

/// The text names a host Core cannot tell is this machine.
const NOT_LOOPBACK: Refusal = "notLoopback";

/// A port from 1 to 65535 written in decimal digits only.
fn announced_port(text: &str) -> Result<u16, Refusal> {
    if text.is_empty() || text.len() > 5 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(MALFORMED);
    }
    match text.parse::<u16>() {
        Ok(port) if port >= 1 => Ok(port),
        _ => Err(MALFORMED),
    }
}

/// `host:port`, or `[address]:port` for IPv6, with a loopback host.
fn announced_host_port(text: &str) -> Result<(&str, u16), Refusal> {
    if text.len() > MAX_ANNOUNCED_ENDPOINT {
        return Err(MALFORMED);
    }
    let (host, port) = match text.strip_prefix('[') {
        Some(bracketed) => {
            let (address, port) = bracketed.split_once("]:").ok_or(MALFORMED)?;
            if !address.contains(':') {
                return Err(MALFORMED);
            }
            (address, port)
        }
        None => {
            let (host, port) = text.rsplit_once(':').ok_or(MALFORMED)?;
            if host.contains(':') {
                return Err(MALFORMED);
            }
            (host, port)
        }
    };
    let port = announced_port(port)?;
    if host_problem(host).is_some() {
        return Err(MALFORMED);
    }
    if !is_loopback_host(host) {
        return Err(NOT_LOOPBACK);
    }
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcement_fixtures_match_the_reading() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/debug/adapter-announcements-v1.json"
        )))
        .expect("announcement fixture is JSON");
        let cases = fixture["cases"].as_array().expect("fixture cases");
        assert!(!cases.is_empty());
        for case in cases {
            let result = read_announcement(case["input"].clone());
            match case.get("result") {
                Some(expected) => assert_eq!(result.expect("a reading"), *expected, "{case}"),
                None => assert!(result.is_err(), "{case}"),
            }
        }
    }
}
