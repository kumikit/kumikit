//! Pairing codes and device tokens (`docs/spec/02-protocol.md#pairing`).
//!
//! A code lives only in the running core's memory. It is valid for [`CODE_TTL`], works
//! once, is burnt after [`CODE_ATTEMPTS`] wrong tries, and a new code replaces the old.
//! Times are passed in, so tests need no clock.

use std::time::Duration;

use base64::Engine;
use kumikit_protocol::remote::{CODE_ALPHABET, CODE_LEN, TOKEN_PREFIX, normalize_code};
use sha2::{Digest, Sha256};
use tokio::time::Instant;

/// How long a pairing code works.
pub const CODE_TTL: Duration = Duration::from_secs(5 * 60);

/// Wrong attempts that burn a code.
pub const CODE_ATTEMPTS: u32 = 5;

/// The live pairing code, if any.
#[derive(Debug, Default)]
pub struct Pairing {
    live: Option<Live>,
}

#[derive(Debug)]
struct Live {
    code: String,
    expires: Instant,
    failures: u32,
}

/// Why a code was refused. The client is told only "wrong or expired".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// No code is live (never made, used, burnt or replaced).
    NoCode,
    /// The code is past its time.
    Expired,
    /// The code does not match.
    Wrong,
}

impl Pairing {
    /// Makes a new code, replacing any live one. Returns it unformatted (8 characters).
    pub fn create(&mut self, now: Instant) -> std::io::Result<String> {
        let mut bytes = [0u8; CODE_LEN];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        // 256 is a multiple of 32, so taking the low 5 bits is unbiased.
        let code: String = bytes
            .iter()
            .map(|b| CODE_ALPHABET[usize::from(b & 31)] as char)
            .collect();
        self.live = Some(Live {
            code: code.clone(),
            expires: now + CODE_TTL,
            failures: 0,
        });
        Ok(code)
    }

    /// Checks a code as the user typed it. A match uses the code up; a miss counts
    /// towards burning it.
    pub fn redeem(&mut self, input: &str, now: Instant) -> Result<(), Refused> {
        let live = self.live.as_mut().ok_or(Refused::NoCode)?;
        if now >= live.expires {
            self.live = None;
            return Err(Refused::Expired);
        }
        let matches = normalize_code(input)
            .is_some_and(|c| constant_time_eq(c.as_bytes(), live.code.as_bytes()));
        if matches {
            self.live = None;
            return Ok(());
        }
        live.failures += 1;
        if live.failures >= CODE_ATTEMPTS {
            self.live = None;
        }
        Err(Refused::Wrong)
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A new device token: 32 random bytes, base64url, with the `kkd_` prefix.
pub fn new_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    Ok(format!(
        "{TOKEN_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    ))
}

/// What the core stores for a token: its SHA-256, hex.
pub fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (Pairing, String, Instant) {
        let now = Instant::now();
        let mut p = Pairing::default();
        let code = p.create(now).unwrap();
        (p, code, now)
    }

    #[test]
    fn codes_use_the_alphabet() {
        let (_, code, _) = fresh();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)));
    }

    #[test]
    fn a_code_works_once_however_it_is_typed() {
        let (mut p, code, now) = fresh();
        let typed = format!(" {}-{} ", code[..4].to_lowercase(), &code[4..]);
        assert_eq!(p.redeem(&typed, now), Ok(()));
        assert_eq!(p.redeem(&code, now), Err(Refused::NoCode));
    }

    #[test]
    fn a_code_expires() {
        let (mut p, code, now) = fresh();
        assert_eq!(p.redeem(&code, now + CODE_TTL), Err(Refused::Expired));
        assert_eq!(p.redeem(&code, now), Err(Refused::NoCode));
    }

    #[test]
    fn five_wrong_tries_burn_a_code() {
        let (mut p, code, now) = fresh();
        let wrong = if code == "00000000" {
            "11111111"
        } else {
            "00000000"
        };
        for _ in 0..CODE_ATTEMPTS - 1 {
            assert_eq!(p.redeem(wrong, now), Err(Refused::Wrong));
        }
        assert_eq!(p.redeem(wrong, now), Err(Refused::Wrong));
        assert_eq!(p.redeem(&code, now), Err(Refused::NoCode));
    }

    #[test]
    fn four_wrong_tries_still_allow_the_right_one() {
        let (mut p, code, now) = fresh();
        let wrong = if code == "00000000" {
            "11111111"
        } else {
            "00000000"
        };
        for _ in 0..CODE_ATTEMPTS - 1 {
            assert_eq!(p.redeem(wrong, now), Err(Refused::Wrong));
        }
        assert_eq!(p.redeem(&code, now), Ok(()));
    }

    #[test]
    fn a_new_code_replaces_the_old() {
        let (mut p, old, now) = fresh();
        let new = p.create(now).unwrap();
        if old != new {
            assert_eq!(p.redeem(&old, now), Err(Refused::Wrong));
        }
        assert_eq!(p.redeem(&new, now), Ok(()));
    }

    #[test]
    fn tokens_are_random_and_hashed() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_ne!(a, b);
        assert!(a.starts_with("kkd_"));
        assert_eq!(a.len(), 4 + 43);
        assert_eq!(token_hash(&a).len(), 64);
        assert_ne!(token_hash(&a), token_hash(&b));
        assert!(!token_hash(&a).contains(&a[4..]));
    }
}
