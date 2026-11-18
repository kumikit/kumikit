//! Protocol version and the compatibility rule (`docs/spec/02-protocol.md#handshake`).

/// The protocol version this crate describes.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 5 };

/// A `MAJOR.MINOR` protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion {
    /// Major version; a change breaks compatibility.
    pub major: u32,
    /// Minor version; additive from 1.0 on, possibly breaking during 0.x.
    pub minor: u32,
}

impl ProtocolVersion {
    /// Parses `MAJOR.MINOR`.
    pub fn parse(value: &str) -> Option<Self> {
        let (major, minor) = value.split_once('.')?;
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(major) || !digits(minor) {
            return None;
        }
        Some(ProtocolVersion {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
        })
    }

    /// Whether a peer speaking `other` can talk to us: the same major, and during 0.x
    /// also the same minor.
    pub fn compatible_with(self, other: ProtocolVersion) -> bool {
        self.major == other.major && (self.major > 0 || self.minor == other.minor)
    }
}

impl std::fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> ProtocolVersion {
        ProtocolVersion::parse(s).unwrap()
    }

    #[test]
    fn parses() {
        assert_eq!(v("0.1"), ProtocolVersion { major: 0, minor: 1 });
        assert_eq!(PROTOCOL_VERSION.to_string(), "0.5");
        for bad in ["1", "1.", ".1", "1.2.3", "a.b", "-1.0", "+1.0"] {
            assert_eq!(ProtocolVersion::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn compatibility() {
        assert!(v("0.1").compatible_with(v("0.1")));
        assert!(!v("0.1").compatible_with(v("0.2")));
        assert!(v("1.0").compatible_with(v("1.3")));
        assert!(!v("1.0").compatible_with(v("2.0")));
    }
}
