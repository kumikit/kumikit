//! `kumikit theme check <file.json | dir> [--strict] [--json]`
//! (`docs/spec/08-themes.md#validation`, ADR 0037): checks a theme JSON file or a
//! directory of theme JSON files for structure, token types, and WCAG AA contrast.

use std::path::Path;

use anyhow::Result;
use kumikit_theme::Report;

use crate::cli::Out;

/// Checks a `.json` theme resource file or every `.json` theme file in a directory.
pub fn check(target: &Path) -> Result<Report> {
    if target.is_dir() {
        Ok(kumikit_theme::check_theme_dir(target))
    } else {
        Ok(kumikit_theme::check_theme_file(target))
    }
}

/// Prints the report; returns whether the theme passes (with `strict`, warnings fail it).
pub fn print(report: &Report, strict: bool, json: bool, out: &mut dyn Out) -> bool {
    let ok = report.errors.is_empty() && (!strict || report.warnings.is_empty());
    if json {
        out.line(
            &serde_json::json!({ "ok": ok, "errors": report.errors, "warnings": report.warnings })
                .to_string(),
        );
        return ok;
    }
    for e in &report.errors {
        out.line(&format!("error: {e}"));
    }
    for w in &report.warnings {
        out.line(&format!("warning: {w}"));
    }
    out.line(
        match (report.errors.is_empty(), report.warnings.is_empty()) {
            (true, true) => "The theme passes every check.",
            (true, false) if strict => {
                "The theme fails: --strict treats contrast warnings as errors."
            }
            (true, false) => "The theme can be imported; users are warned about its contrast.",
            (false, _) => "The theme cannot be imported.",
        },
    );
    ok
}
