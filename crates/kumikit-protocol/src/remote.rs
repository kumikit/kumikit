//! Pure rules of the remote transport and pairing (`docs/spec/02-protocol.md#remote-transport`,
//! `#pairing`), shared so the core, the CLI and the shell print and read the same text.

/// The port `kumikit serve --listen <host>` uses when none is given.
pub const DEFAULT_PORT: u16 = 7463;

/// The WebSocket path of the protocol.
pub const RPC_PATH: &str = "/rpc";

/// WebSocket close code sent when the connection's device was revoked.
pub const CLOSE_REVOKED: u16 = 4001;

/// Prefix of device tokens, so a leaked one is recognisable.
pub const TOKEN_PREFIX: &str = "kkd_";

/// Name of the browser client's device cookie.
pub const DEVICE_COOKIE: &str = "kk_device";

/// Characters of a pairing code: Crockford base32 (no I, L, O, U).
pub const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Length of a pairing code, without its separator.
pub const CODE_LEN: usize = 8;

/// Longest device name, in characters.
pub const DEVICE_NAME_MAX: usize = 64;

/// Reads `host`, `host:port`, `[v6]`, `[v6]:port` or a bare IPv6 address; the port
/// defaults to [`DEFAULT_PORT`]. A `wss://` prefix and a `/rpc` suffix are ignored, so a
/// user can paste the URL `kumikit pair` printed. Used by `serve --listen` and by clients.
pub fn parse_host_port(value: &str) -> Option<(String, u16)> {
    let value = value.trim();
    let value = value.strip_prefix("wss://").unwrap_or(value);
    let value = value.strip_suffix(RPC_PATH).unwrap_or(value);
    if let Some(rest) = value.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if after.is_empty() => DEFAULT_PORT,
            None => return None,
        };
        return (!host.is_empty()).then(|| (host.to_owned(), port));
    }
    if value.chars().any(|c| c.is_whitespace() || c == '/') {
        return None;
    }
    match value.rsplit_once(':') {
        // A bare IPv6 address has several colons and no port.
        Some((host, _)) if host.contains(':') => Some((value.to_owned(), DEFAULT_PORT)),
        Some((host, port)) if !host.is_empty() => Some((host.to_owned(), port.parse().ok()?)),
        Some(_) => None,
        None if !value.is_empty() => Some((value.to_owned(), DEFAULT_PORT)),
        None => None,
    }
}

/// Formats a certificate's SHA-256 as `SHA256:` and 64 lowercase hex digits in groups of
/// four separated by spaces.
pub fn format_fingerprint(sha256: &[u8; 32]) -> String {
    let hex: Vec<String> = sha256
        .chunks(2)
        .map(|pair| pair.iter().map(|b| format!("{b:02x}")).collect())
        .collect();
    format!("SHA256:{}", hex.join(" "))
}

/// Reads a fingerprint written by [`format_fingerprint`], ignoring case and whitespace, with
/// or without the `SHA256:` prefix. `None` if it is not 32 bytes of hex.
pub fn parse_fingerprint(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    let text = text
        .strip_prefix("SHA256:")
        .or_else(|| text.strip_prefix("sha256:"))
        .unwrap_or(text);
    let digits: Vec<u8> = text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b':')
        .collect();
    if digits.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in digits.chunks(2).enumerate() {
        let s = std::str::from_utf8(pair).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

/// Formats 8 code characters as `ABCD-EFGH`.
pub fn format_code(code: &str) -> String {
    match code.get(..4).zip(code.get(4..)) {
        Some((a, b)) => format!("{a}-{b}"),
        None => code.to_owned(),
    }
}

/// Reads a pairing code as a user typed it: case, `-` and whitespace are ignored, and the
/// Crockford look-alikes `I`/`L` (1) and `O` (0) are accepted. `None` if it is not 8
/// characters of the alphabet.
pub fn normalize_code(input: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_LEN);
    for c in input.chars() {
        if c == '-' || c.is_whitespace() {
            continue;
        }
        let c = match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            c => c,
        };
        if !c.is_ascii() || !CODE_ALPHABET.contains(&(c as u8)) {
            return None;
        }
        out.push(c);
    }
    (out.len() == CODE_LEN).then_some(out)
}

/// Whether a device name is acceptable: 1–64 characters, no control characters, not only
/// whitespace.
pub fn valid_device_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=DEVICE_NAME_MAX).contains(&n)
        && !name.chars().any(char::is_control)
        && !name.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_port() {
        assert_eq!(parse_host_port("0.0.0.0"), Some(("0.0.0.0".into(), 7463)));
        assert_eq!(
            parse_host_port("127.0.0.1:0"),
            Some(("127.0.0.1".into(), 0))
        );
        assert_eq!(
            parse_host_port("devbox:9000"),
            Some(("devbox".into(), 9000))
        );
        assert_eq!(parse_host_port("[::1]:9000"), Some(("::1".into(), 9000)));
        assert_eq!(parse_host_port("[::1]"), Some(("::1".into(), 7463)));
        assert_eq!(parse_host_port("::1"), Some(("::1".into(), 7463)));
        assert_eq!(parse_host_port(""), None);
        assert_eq!(parse_host_port(":9000"), None);
        assert_eq!(parse_host_port("host:port"), None);
        assert_eq!(parse_host_port("[::1]x"), None);
        assert_eq!(
            parse_host_port("wss://devbox:7463/rpc"),
            Some(("devbox".into(), 7463))
        );
        assert_eq!(
            parse_host_port("wss://[::1]:9/rpc"),
            Some(("::1".into(), 9))
        );
        assert_eq!(parse_host_port("dev box"), None);
        assert_eq!(parse_host_port("[]:9"), None);
    }

    #[test]
    fn fingerprints_round_trip() {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i * 7) as u8;
        }
        let text = format_fingerprint(&bytes);
        assert!(text.starts_with("SHA256:0007 0e15 "));
        assert_eq!(text.len(), "SHA256:".len() + 64 + 15);
        assert_eq!(parse_fingerprint(&text), Some(bytes));
        assert_eq!(
            parse_fingerprint(&text.to_uppercase().replace("SHA256:", "")),
            Some(bytes)
        );
        assert_eq!(parse_fingerprint("SHA256:abcd"), None);
        assert_eq!(parse_fingerprint(&"zz".repeat(32)), None);
    }

    #[test]
    fn codes_are_forgiving_about_how_they_are_typed() {
        assert_eq!(format_code("ABCD2345"), "ABCD-2345");
        assert_eq!(normalize_code("abcd-2345").as_deref(), Some("ABCD2345"));
        assert_eq!(normalize_code(" ab cd 23 45 ").as_deref(), Some("ABCD2345"));
        assert_eq!(normalize_code("OIL0-0000").as_deref(), Some("01100000"));
        assert_eq!(normalize_code("ABCD-234"), None);
        assert_eq!(normalize_code("ABCD-23456"), None);
        assert_eq!(normalize_code("ABCD-23U5"), None);
        assert_eq!(normalize_code("ABCD-23é5"), None);
    }

    #[test]
    fn device_names() {
        assert!(valid_device_name("Work laptop"));
        assert!(!valid_device_name(""));
        assert!(!valid_device_name("   "));
        assert!(!valid_device_name("a\nb"));
        assert!(!valid_device_name(&"x".repeat(65)));
        assert!(valid_device_name(&"é".repeat(64)));
    }
}
