//! `kumikit theme check <dir | file.kkp> [--strict] [--json]` (`docs/spec/08-themes.md#validation`):
//! the manifest's structure (the same check as the desktop app's plugin store), then the
//! theme's tokens, contrast and fonts (the same rules as the UI kernel).

use std::path::Path;

use anyhow::{Context, Result};
use kumikit_theme::Report;

use crate::cli::Out;

/// Checks a theme (or a UI plugin's manifest) in a directory or a `.kkp`.
pub fn check(target: &Path) -> Result<Report> {
    let work = tempfile::tempdir()?;
    let dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        let dir = work.path().join("p");
        std::fs::create_dir(&dir)?;
        kumikit_plugin_host::package::unpack(target, &dir)
            .with_context(|| format!("cannot unpack {}", target.display()))?;
        dir
    };
    let mut report = Report::default();
    let kind = match kumikit_assets::ui_manifest::check(&dir) {
        Ok((_, _, manifest)) => manifest["kind"][0].as_str().map(str::to_owned),
        Err(problems) => {
            report
                .errors
                .extend(problems.split("; ").map(str::to_owned));
            None
        }
    };
    if kind.as_deref() == Some("theme") {
        let theme = kumikit_theme::check_theme(&dir);
        report.errors.extend(theme.errors);
        report.warnings.extend(theme.warnings);
    }
    Ok(report)
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
            (true, false) => "The theme can be installed; users are warned about its contrast.",
            (false, _) => "The theme cannot be installed.",
        },
    );
    ok
}
