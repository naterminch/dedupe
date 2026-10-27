use super::{ReportContext, ScanStats};
use crate::matching::Group;

/// # Errors
///
/// Returns an error when the CSV file cannot be written.
pub fn write_csv(
    path: &std::path::Path,
    ctx: &ReportContext,
    stats: &ScanStats,
    groups: &[Group],
    mode_result: Option<&crate::modes::ModeResult>,
) -> std::io::Result<()> {
    let mut out = String::from("mode,kind,group,path,size,detail\n");
    if let Some(result) = mode_result {
        for finding in &result.findings {
            out.push_str(&csv_row(&[
                crate::modes::mode_name(result.mode),
                finding.kind,
                "",
                &finding.path.to_string_lossy(),
                &finding.size.to_string(),
                &finding.detail,
            ]));
        }
        for group in &result.groups {
            for member in &group.members {
                let size = std::fs::metadata(member).map_or(0, |m| m.len());
                out.push_str(&csv_row(&[
                    crate::modes::mode_name(result.mode),
                    "group",
                    &group.label,
                    &member.to_string_lossy(),
                    &size.to_string(),
                    &group.detail,
                ]));
            }
        }
    } else {
        let _ = stats;
        for group in groups {
            for member in &group.members {
                let kind = if member.keep { "keep" } else { "duplicate" };
                let detail = group.similarity.map_or_else(
                    || group.hash.clone(),
                    |s| format!("{:.1}% similar", s * 100.0),
                );
                out.push_str(&csv_row(&[
                    "duplicates",
                    kind,
                    &format!("#{}", group.index),
                    &member.path.to_string_lossy(),
                    &member.size.to_string(),
                    &detail,
                ]));
            }
        }
        let _ = ctx;
    }
    std::fs::write(path, out)
}

fn csv_row(fields: &[&str]) -> String {
    let mut row = String::new();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            row.push(',');
        }
        if field.contains([',', '"', '\n', '\r']) {
            row.push('"');
            row.push_str(&field.replace('"', "\"\""));
            row.push('"');
        } else {
            row.push_str(field);
        }
    }
    row.push('\n');
    row
}
