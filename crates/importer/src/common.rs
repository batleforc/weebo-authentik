//! Helpers shared by 5 of the importer's 6 non-entrypoint modules
//! (`applications`, `brands`, `groups`, `outposts`, `users`).

use std::path::Path;

/// A Kubernetes object name must be a valid RFC 1123 DNS subdomain
/// (lowercase alphanumeric and `-`, no leading/trailing `-`) — Authentik
/// names/usernames/domains have no such constraint, so every generated
/// CR's `metadata.name` goes through this, while `spec.name`/
/// `spec.username`/etc. keep the exact original value.
pub(crate) fn slugify(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut last_was_dash = false;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    let truncated = &trimmed[..trimmed.len().min(253)];
    if truncated.is_empty() {
        "unnamed".to_string()
    } else {
        truncated.to_string()
    }
}

pub(crate) fn write_cr<T: serde::Serialize>(
    out_dir: &Path,
    kind: &str,
    name: &str,
    cr: &T,
) -> anyhow::Result<()> {
    let yaml = serde_norway::to_string(cr)?;
    let path = out_dir.join(format!("{kind}-{name}.yaml"));
    std::fs::write(&path, yaml)?;
    Ok(())
}
