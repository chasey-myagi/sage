//! Changelog parser utilities.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/changelog.ts`.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangelogEntry {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub content: String,
}

impl ChangelogEntry {
    pub fn version_string(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Parse changelog entries from a `CHANGELOG.md` file.
///
/// Scans for `## ` lines, collects content until the next `## ` or EOF.
pub fn parse_changelog(changelog_path: &Path) -> Vec<ChangelogEntry> {
    if !changelog_path.exists() {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(changelog_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: Could not parse changelog: {e}");
            return Vec::new();
        }
    };

    let mut entries = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();
    let mut current_version: Option<(u32, u32, u32)> = None;
    let re = regex::Regex::new(r"##\s+\[?(\d+)\.(\d+)\.(\d+)\]?").unwrap();

    for line in content.lines() {
        if line.starts_with("## ") {
            // Save previous entry if any.
            if let Some((major, minor, patch)) = current_version.take() {
                let content = current_lines.join("\n").trim().to_owned();
                entries.push(ChangelogEntry {
                    major,
                    minor,
                    patch,
                    content,
                });
            }

            // Parse version from this line: ## [x.y.z] or ## x.y.z
            if let Some(caps) = re.captures(line) {
                let major: u32 = caps[1].parse().unwrap_or(0);
                let minor: u32 = caps[2].parse().unwrap_or(0);
                let patch: u32 = caps[3].parse().unwrap_or(0);
                current_version = Some((major, minor, patch));
                current_lines = vec![line];
            } else {
                current_version = None;
                current_lines = Vec::new();
            }
        } else if current_version.is_some() {
            current_lines.push(line);
        }
    }

    // Save last entry.
    if let Some((major, minor, patch)) = current_version {
        let content = current_lines.join("\n").trim().to_owned();
        entries.push(ChangelogEntry {
            major,
            minor,
            patch,
            content,
        });
    }

    entries
}

/// Compare two `ChangelogEntry` versions.
/// Returns `Ordering::Less` if `a < b`, `Equal`, or `Greater`.
pub fn compare_versions(a: &ChangelogEntry, b: &ChangelogEntry) -> std::cmp::Ordering {
    a.major
        .cmp(&b.major)
        .then(a.minor.cmp(&b.minor))
        .then(a.patch.cmp(&b.patch))
}

/// Return entries newer than `last_version` (e.g. `"1.2.3"`).
pub fn get_new_entries(entries: &[ChangelogEntry], last_version: &str) -> Vec<ChangelogEntry> {
    let parts: Vec<u32> = last_version
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    let last = ChangelogEntry {
        major: parts.first().copied().unwrap_or(0),
        minor: parts.get(1).copied().unwrap_or(0),
        patch: parts.get(2).copied().unwrap_or(0),
        content: String::new(),
    };
    entries
        .iter()
        .filter(|e| compare_versions(e, &last) == std::cmp::Ordering::Greater)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_changelog(tmp: &TempDir, content: &str) -> std::path::PathBuf {
        let path = tmp.path().join("CHANGELOG.md");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn parse_empty_file() {
        let tmp = TempDir::new().unwrap();
        let path = write_changelog(&tmp, "");
        let entries = parse_changelog(&path);
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_nonexistent_file() {
        let entries = parse_changelog(Path::new("/nonexistent/CHANGELOG.md"));
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_single_entry() {
        let tmp = TempDir::new().unwrap();
        let path = write_changelog(&tmp, "## [1.2.3] - 2024-01-01\n\n### Added\n- Feature X\n");
        let entries = parse_changelog(&path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].major, 1);
        assert_eq!(entries[0].minor, 2);
        assert_eq!(entries[0].patch, 3);
        assert!(entries[0].content.contains("Feature X"));
    }

    #[test]
    fn parse_multiple_entries() {
        let tmp = TempDir::new().unwrap();
        let path = write_changelog(
            &tmp,
            "## [2.0.0]\nBreaking change.\n## [1.5.0]\nMinor update.\n",
        );
        let entries = parse_changelog(&path);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].major, 2);
        assert_eq!(entries[1].major, 1);
    }

    #[test]
    fn get_new_entries_filters_correctly() {
        let entries = vec![
            ChangelogEntry {
                major: 2,
                minor: 0,
                patch: 0,
                content: "v2".into(),
            },
            ChangelogEntry {
                major: 1,
                minor: 5,
                patch: 0,
                content: "v1.5".into(),
            },
            ChangelogEntry {
                major: 1,
                minor: 0,
                patch: 0,
                content: "v1".into(),
            },
        ];
        let new = get_new_entries(&entries, "1.5.0");
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].major, 2);
    }

    #[test]
    fn compare_versions_ordering() {
        let v1 = ChangelogEntry {
            major: 1,
            minor: 0,
            patch: 0,
            content: "".into(),
        };
        let v2 = ChangelogEntry {
            major: 2,
            minor: 0,
            patch: 0,
            content: "".into(),
        };
        assert_eq!(compare_versions(&v1, &v2), std::cmp::Ordering::Less);
        assert_eq!(compare_versions(&v2, &v1), std::cmp::Ordering::Greater);
        assert_eq!(compare_versions(&v1, &v1), std::cmp::Ordering::Equal);
    }
}
