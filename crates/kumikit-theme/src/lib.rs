//! Theme checks (`docs/spec/08-themes.md#validation`) for `kumikit theme check`: token
//! values by type, WCAG contrast of the text tokens on their surfaces, and fonts.
//!
//! The rules are the UI kernel's (`web/kernel/src/theme/validate.ts`, `contrast.ts`),
//! ported. Their data is shared, not copied: the token set is the published
//! `web/sdk/api/<version>/tokens.json` and the contrast pairs are
//! `web/sdk/theme-rules.json`, both read at compile time. Both implementations must pass
//! `web/sdk/tests/theme-vectors.json`, so they cannot drift apart unnoticed.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;

const TOKENS_JSON: &str = include_str!("../../../web/sdk/api/0.1.0/tokens.json");
const RULES_JSON: &str = include_str!("../../../web/sdk/theme-rules.json");

#[derive(Deserialize)]
struct TokenSet {
    tokens: Vec<Token>,
}

#[derive(Deserialize)]
struct Token {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(Deserialize)]
struct Rules {
    contrast: Vec<(String, String, f64)>,
}

/// Every token's name and type, from the published token set.
pub fn token_types() -> &'static BTreeMap<String, String> {
    static TYPES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    TYPES.get_or_init(|| {
        // The file is part of the source tree and checked by tests; an empty map would make
        // every token unknown, which the tests catch.
        serde_json::from_str::<TokenSet>(TOKENS_JSON)
            .map(|s| s.tokens.into_iter().map(|t| (t.name, t.ty)).collect())
            .unwrap_or_default()
    })
}

/// Text tokens, the surfaces they are used on, and the minimum contrast.
pub fn contrast_pairs() -> &'static [(String, String, f64)] {
    static PAIRS: OnceLock<Vec<(String, String, f64)>> = OnceLock::new();
    PAIRS.get_or_init(|| {
        serde_json::from_str::<Rules>(RULES_JSON)
            .map(|r| r.contrast)
            .unwrap_or_default()
    })
}

struct Patterns {
    color_fn: Regex,
    hex: Regex,
    shadow_one: Regex,
    font_name: Regex,
    length: Regex,
    number: Regex,
    duration: Regex,
    easing_word: Regex,
    bezier: Regex,
    forbidden: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let num = r"-?(?:\d+\.?\d*|\.\d+)";
        let len = format!(r"(?:0|{num}(?:px|rem|em))");
        let pct = format!(r"{num}%");
        // These patterns are fixed strings, ported from validate.ts; they always compile.
        #[allow(clippy::unwrap_used)]
        let re = |s: &str| Regex::new(s).unwrap();
        Patterns {
            color_fn: re(&format!(
                r"^(?:rgba?|hsla?)\(\s*{num}(?:deg)?%?\s*(?:,\s*|\s+){num}%?\s*(?:,\s*|\s+){num}%?\s*(?:(?:,|/)\s*(?:{num}|{pct})\s*)?\)$"
            )),
            hex: re(r"(?i)^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$"),
            shadow_one: re(&format!(r"^(?:inset\s+)?{len}\s+{len}(?:\s+{len}){{0,2}}\s+(.+)$")),
            font_name: re(r#"(?i)^(?:"[^";{}]+"|'[^';{}]+'|[a-z][a-z0-9 -]*)$"#),
            length: re(&format!("^{len}$")),
            number: re(&format!("^{num}$")),
            duration: re(&format!("^{num}(?:ms|s)$")),
            easing_word: re("^(?:linear|ease|ease-in|ease-out|ease-in-out)$"),
            bezier: re(&format!(
                r"^cubic-bezier\(\s*{num}\s*,\s*{num}\s*,\s*{num}\s*,\s*{num}\s*\)$"
            )),
            forbidden: re(r"[;{}\\<>]|/\*"),
        }
    })
}

/// Whether `v` is a color the theme engine accepts.
pub fn is_color(v: &str) -> bool {
    let s = v.trim();
    let p = patterns();
    s == "transparent" || p.hex.is_match(s) || p.color_fn.is_match(&s.to_lowercase())
}

/// Splits on commas outside parentheses.
fn split_top(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in v.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if ch == ',' && depth == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_owned()).collect()
}

/// Whether `value` is a valid value of the token type `ty` (`color`, `length`, `number`,
/// `duration`, `easing`, `font-list`, `shadow`).
pub fn is_valid(ty: &str, value: &str) -> bool {
    let p = patterns();
    if value.chars().count() > 200 || p.forbidden.is_match(value) {
        return false;
    }
    let v = value.trim();
    match ty {
        "color" => is_color(v),
        "length" => p.length.is_match(v),
        "number" => p.number.is_match(v),
        "duration" => p.duration.is_match(v),
        "easing" => p.easing_word.is_match(v) || p.bezier.is_match(v),
        "font-list" => split_top(v).iter().all(|f| p.font_name.is_match(f)),
        "shadow" => {
            v == "none"
                || split_top(v).iter().all(|s| {
                    p.shadow_one
                        .captures(s)
                        .and_then(|c| c.get(1))
                        .is_some_and(|c| is_color(c.as_str()))
                })
        }
        _ => false,
    }
}

/// A color as `[r, g, b, a]` (0–255, alpha 0–1), for `#hex` and `rgb()`/`rgba()`.
fn parse_color(v: &str) -> Option<[f64; 4]> {
    let s = v.trim().to_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        let h: String = if h.len() <= 4 {
            h.chars().flat_map(|c| [c, c]).collect()
        } else {
            h.to_owned()
        };
        let n = |i: usize| -> Option<f64> {
            Some(f64::from(u8::from_str_radix(h.get(i..i + 2)?, 16).ok()?))
        };
        let a = if h.len() == 8 { n(6)? / 255.0 } else { 1.0 };
        return Some([n(0)?, n(2)?, n(4)?, a]);
    }
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let vals: Vec<f64> = inner
        .split(|c: char| c.is_whitespace() || c == ',' || c == '/')
        .filter(|p| !p.is_empty())
        .map(|p| match p.strip_suffix('%') {
            Some(x) => x.parse::<f64>().map(|f| f / 100.0),
            None => p.parse::<f64>(),
        })
        .collect::<Result<_, _>>()
        .ok()?;
    let get = |i: usize, d: f64| vals.get(i).copied().unwrap_or(d);
    Some([get(0, 0.0), get(1, 0.0), get(2, 0.0), get(3, 1.0)])
}

fn luminance(c: [f64; 3]) -> f64 {
    let ch = |c: f64| {
        let x = c / 255.0;
        if x <= 0.03928 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
}

/// The WCAG contrast ratio of `fg` on `bg` (1–21); a translucent `fg` is blended onto
/// `bg`. Colors it cannot read give 1.
pub fn contrast(fg: &str, bg: &str) -> f64 {
    let (Some(f), Some(b)) = (parse_color(fg), parse_color(bg)) else {
        return 1.0;
    };
    let a = f[3];
    let blended = [
        f[0] * a + b[0] * (1.0 - a),
        f[1] * a + b[1] * (1.0 - a),
        f[2] * a + b[2] * (1.0 - a),
    ];
    let (l1, l2) = (luminance(blended), luminance([b[0], b[1], b[2]]));
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

/// What a check found. Errors refuse the theme; warnings (contrast) are shown and, with
/// `--strict`, refuse it too.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Report {
    /// Problems that make the theme invalid.
    pub errors: Vec<String>,
    /// Contrast below WCAG AA, and pairs that could not be checked.
    pub warnings: Vec<String>,
}

/// Checks one variant's tokens: every token known and of a valid value, and the contrast
/// pairs whose tokens it sets.
pub fn check_tokens(variant: &str, tokens: &serde_json::Value, report: &mut Report) {
    let Some(map) = tokens.as_object() else {
        report
            .errors
            .push(format!("{variant}: the tokens file must be a JSON object"));
        return;
    };
    let types = token_types();
    let mut good: BTreeMap<&str, &str> = BTreeMap::new();
    for (name, value) in map {
        if name == "$schema" {
            continue;
        }
        match (types.get(name), value.as_str()) {
            (None, _) => report
                .errors
                .push(format!("{variant}: unknown token {name}")),
            (Some(ty), Some(v)) if is_valid(ty, v) => {
                good.insert(name, v);
            }
            (Some(ty), _) => report
                .errors
                .push(format!("{variant}: {name} is not a valid {ty}")),
        }
    }
    for (fg, bg, min) in contrast_pairs() {
        let (Some(f), Some(b)) = (good.get(fg.as_str()), good.get(bg.as_str())) else {
            continue;
        };
        let ratio = contrast(f, b);
        if ratio + 1e-9 < *min {
            report.warnings.push(format!(
                "{variant}: {fg} on {bg} has contrast {ratio:.2}, below {min} (WCAG AA)"
            ));
        }
    }
}

#[derive(Deserialize)]
struct ThemeManifest {
    #[serde(default)]
    theme: Option<ThemeSection>,
}

#[derive(Deserialize)]
struct ThemeSection {
    #[serde(default)]
    variants: Vec<Variant>,
    #[serde(default)]
    fonts: Vec<String>,
}

#[derive(Deserialize)]
struct Variant {
    id: String,
    tokens: String,
}

/// Whether the file is a WOFF2 web font, by its first bytes (the app's plugin store
/// accepts only `.woff2` fonts).
pub fn is_web_font(bytes: &[u8]) -> bool {
    bytes.starts_with(b"wOF2")
}

/// Checks a theme unpacked in `dir` (its manifest's structure is checked separately):
/// each variant's tokens and contrast, and each font.
pub fn check_theme(dir: &Path) -> Report {
    let mut report = Report::default();
    let manifest = std::fs::read_to_string(dir.join("plugin.toml"))
        .map_err(|e| e.to_string())
        .and_then(|t| toml::from_str::<ThemeManifest>(&t).map_err(|e| e.to_string()));
    let section = match manifest {
        Ok(ThemeManifest { theme: Some(t) }) => t,
        Ok(_) => {
            report
                .errors
                .push("the manifest has no [theme] section".into());
            return report;
        }
        Err(e) => {
            report.errors.push(format!("plugin.toml: {e}"));
            return report;
        }
    };
    for v in &section.variants {
        let tokens = std::fs::read_to_string(dir.join(&v.tokens))
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()));
        match tokens {
            Ok(t) => check_tokens(&v.id, &t, &mut report),
            Err(e) => report.errors.push(format!("{}: {e}", v.tokens)),
        }
    }
    for font in &section.fonts {
        match std::fs::read(dir.join(font)) {
            Ok(bytes) if is_web_font(&bytes) => {}
            Ok(_) => report
                .errors
                .push(format!("{font} is not a WOFF2 web font")),
            Err(_) => report
                .errors
                .push(format!("{font} is missing from the package")),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Vectors {
        values: Vec<ValueCase>,
        contrast: Vec<ContrastCase>,
    }

    #[derive(Deserialize)]
    struct ValueCase {
        #[serde(rename = "type")]
        ty: String,
        value: String,
        valid: bool,
    }

    #[derive(Deserialize)]
    struct ContrastCase {
        fg: String,
        bg: String,
        ratio: f64,
    }

    #[test]
    fn the_shared_vectors_pass() {
        let v: Vectors =
            serde_json::from_str(include_str!("../../../web/sdk/tests/theme-vectors.json"))
                .unwrap();
        for c in &v.values {
            assert_eq!(is_valid(&c.ty, &c.value), c.valid, "{} {:?}", c.ty, c.value);
        }
        for c in &v.contrast {
            let got = contrast(&c.fg, &c.bg);
            assert!(
                (got - c.ratio).abs() < 0.005,
                "{} on {}: {got} vs {}",
                c.fg,
                c.bg,
                c.ratio
            );
        }
    }

    #[test]
    fn the_shared_data_loads() {
        assert!(token_types().len() > 40);
        assert_eq!(token_types()["accent"], "color");
        assert!(
            contrast_pairs()
                .iter()
                .any(|(f, b, _)| f == "text.primary" && b == "surface.panel")
        );
    }

    fn theme(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (p, bytes) in files {
            let f = d.path().join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, bytes).unwrap();
        }
        d
    }

    const MANIFEST: &[u8] = b"id = \"acme.night\"\nname = \"Night\"\nversion = \"1.0.0\"\nkind = [\"theme\"]\napi = \"^0.1\"\n[theme]\nvariants = [{ id = \"dark\", appearance = \"dark\", tokens = \"dark.json\" }]\nfonts = [\"fonts/a.woff2\"]\n";

    #[test]
    fn a_good_theme_passes_and_a_bad_one_is_explained() {
        let good = theme(&[
            ("plugin.toml", MANIFEST),
            (
                "dark.json",
                br##"{"surface.panel": "#1a1b26", "text.primary": "#c0caf5"}"##,
            ),
            ("fonts/a.woff2", b"wOF2...."),
        ]);
        assert_eq!(check_theme(good.path()), Report::default());

        let bad = theme(&[
            ("plugin.toml", MANIFEST),
            (
                "dark.json",
                br##"{"surface.panel": "#1a1b26", "text.primary": "#24283b", "accent": "red;x", "nope": "#fff"}"##,
            ),
            ("fonts/a.woff2", b"<html>"),
        ]);
        let r = check_theme(bad.path());
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("accent is not a valid color")),
            "{r:?}"
        );
        assert!(
            r.errors.iter().any(|e| e.contains("unknown token nope")),
            "{r:?}"
        );
        assert!(
            r.errors.iter().any(|e| e.contains("not a WOFF2 web font")),
            "{r:?}"
        );
        assert!(
            r.warnings
                .iter()
                .any(|w| w.contains("text.primary on surface.panel")),
            "{r:?}"
        );

        let missing = theme(&[("plugin.toml", MANIFEST)]);
        let r = check_theme(missing.path());
        assert!(r.errors.iter().any(|e| e.contains("dark.json")), "{r:?}");
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("missing from the package")),
            "{r:?}"
        );
    }
}
