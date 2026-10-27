use super::{ReportContext, ScanStats, dup_file_count, reclaimable_bytes};
use crate::matching::Group;
use crate::util::human_bytes;
use std::fmt::Write as _;

/// # Errors
///
/// Returns an error when the HTML file cannot be written.
pub fn write_html(
    path: &std::path::Path,
    ctx: &ReportContext,
    stats: &ScanStats,
    groups: &[Group],
    mode_result: Option<&crate::modes::ModeResult>,
) -> std::io::Result<()> {
    let title = mode_result.map_or("Duplicate files", |m| crate::modes::mode_title(m.mode));
    let summary = mode_result.map_or_else(
        || {
            format!(
                "{} files scanned · {} groups · {} duplicate files · {} reclaimable",
                stats.files_scanned,
                groups.len(),
                dup_file_count(groups),
                human_bytes(reclaimable_bytes(groups))
            )
        },
        |m| {
            format!(
                "{} files scanned · {} findings · {} reclaimable",
                m.files_scanned,
                m.findings.len() + m.groups.len(),
                human_bytes(m.reclaimable())
            )
        },
    );
    let mut body = String::new();
    let _ = write!(
        body,
        "<header><h1>{}</h1><p>{}</p><p class=\"paths\">{}</p></header>",
        escape_html(title),
        escape_html(&summary),
        escape_html(&ctx.paths.join(" · ")),
    );
    if let Some(result) = mode_result {
        if !result.findings.is_empty() {
            body.push_str("<h2>Findings</h2><table><thead><tr><th>Kind</th><th>Path</th><th>Size</th><th>Detail</th></tr></thead><tbody>");
            for f in &result.findings {
                let _ = write!(
                    body,
                    "<tr><td>{}</td><td class=\"path\">{}</td><td>{}</td><td>{}</td></tr>",
                    escape_html(f.kind),
                    escape_html(&f.path.to_string_lossy()),
                    if f.is_dir {
                        "folder".to_string()
                    } else {
                        human_bytes(f.size)
                    },
                    escape_html(&f.detail),
                );
            }
            body.push_str("</tbody></table>");
        }
        for (i, group) in result.groups.iter().enumerate() {
            let _ = write!(
                body,
                "<h2>Group #{} · {}</h2><p>{}</p><table><thead><tr><th>Role</th><th>Path</th><th>Size</th></tr></thead><tbody>",
                i + 1,
                escape_html(&group.label),
                escape_html(&group.detail),
            );
            for (n, member) in group.members.iter().enumerate() {
                let size = std::fs::metadata(member).map_or(0, |m| m.len());
                let _ = write!(
                    body,
                    "<tr><td class=\"{}\">{}</td><td class=\"path\">{}</td><td>{}</td></tr>",
                    if n == 0 { "keep" } else { "dup" },
                    if n == 0 { "KEEP" } else { "DUP" },
                    escape_html(&member.to_string_lossy()),
                    human_bytes(size),
                );
            }
            body.push_str("</tbody></table>");
        }
    } else {
        for group in groups {
            let _ = write!(
                body,
                "<h2>Group #{} · {}</h2>",
                group.index,
                group.similarity.map_or_else(
                    || escape_html(&group.hash[..group.hash.len().min(24)]),
                    |s| format!("{:.1}% similar", s * 100.0)
                ),
            );
            body.push_str(
                "<table><thead><tr><th>Role</th><th>Path</th><th>Size</th></tr></thead><tbody>",
            );
            for member in &group.members {
                let _ = write!(
                    body,
                    "<tr><td class=\"{}\">{}</td><td class=\"path\">{}</td><td>{}</td></tr>",
                    if member.keep { "keep" } else { "dup" },
                    if member.keep { "KEEP" } else { "DUP" },
                    escape_html(&member.path.to_string_lossy()),
                    human_bytes(member.size),
                );
            }
            body.push_str("</tbody></table>");
        }
    }
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>dedupe report</title><style>body{{font:15px/1.5 system-ui,sans-serif;max-width:1100px;margin:2rem auto;padding:0 1rem;color:#222}}h1{{margin-bottom:.2rem}}h2{{margin-top:2rem}}p{{color:#555}}.paths{{font-size:.85rem;color:#777}}table{{border-collapse:collapse;width:100%;margin-top:.5rem}}th,td{{text-align:left;padding:.45rem .6rem;border-bottom:1px solid #e5e5e5;vertical-align:top}}th{{background:#fafafa;font-size:.8rem;text-transform:uppercase;letter-spacing:.04em}}.path{{font-family:ui-monospace,monospace;font-size:.85rem;word-break:break-all}}.keep{{color:#0a7d32;font-weight:600}}.dup{{color:#b26a00;font-weight:600}}footer{{margin-top:3rem;font-size:.8rem;color:#888}}</style></head><body>{body}<footer>Generated by dedupe</footer></body></html>"
    );
    std::fs::write(path, html)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
