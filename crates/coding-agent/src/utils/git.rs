//! Git URL parsing for package source specifiers.
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/git.ts`.
//!
//! Accepts inputs of the form:
//! - `git:github.com/user/repo`
//! - `git:github:user/repo` (host shorthand)
//! - `git:user/repo` (GitHub shorthand)
//! - `https://github.com/user/repo.git`
//! - `git@github.com:user/repo.git`
//! - `ssh://git@github.com/user/repo.git`
//! - With optional `@<ref>` suffix pinning a branch/tag/commit.
//!
//! When the input omits the `git:` prefix, only explicit protocol URLs
//! (`https?://`, `ssh://`, `git://`) are accepted — matching pi-mono.

/// Parsed git URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitSource {
    /// Clone URL, always valid for `git clone`.
    pub repo: String,
    /// Git host domain (e.g. `github.com`).
    pub host: String,
    /// Repository path (e.g. `user/repo`).
    pub path: String,
    /// Git ref (branch, tag, commit) if specified.
    pub ref_: Option<String>,
    /// True if a ref was specified (package will not auto-update).
    pub pinned: bool,
}

/// Known shorthand hosts (mirrors the subset of hosted-git-info we rely on).
const KNOWN_HOSTS: &[(&str, &str)] = &[
    ("github", "github.com"),
    ("gitlab", "gitlab.com"),
    ("bitbucket", "bitbucket.org"),
    ("gist", "gist.github.com"),
];

fn resolve_host_shorthand(host: &str) -> Option<&'static str> {
    KNOWN_HOSTS
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(host))
        .map(|(_, v)| *v)
}

fn split_ref(url: &str) -> (String, Option<String>) {
    // scp-like: `git@host:path[@ref]`
    if let Some(rest) = url.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            if let Some((repo_path, ref_)) = path.split_once('@') {
                if !repo_path.is_empty() && !ref_.is_empty() {
                    return (format!("git@{host}:{repo_path}"), Some(ref_.to_string()));
                }
            }
            return (url.to_string(), None);
        }
        return (url.to_string(), None);
    }

    // URL with scheme: split @ref from the pathname
    if url.contains("://") {
        if let Some(scheme_end) = url.find("://") {
            let after_scheme = &url[scheme_end + 3..];
            // host portion runs until the next `/`.
            if let Some(slash) = after_scheme.find('/') {
                let (host, path_part) = after_scheme.split_at(slash);
                let path_part = &path_part[1..]; // skip the `/`
                if let Some(at) = path_part.find('@') {
                    let repo_path = &path_part[..at];
                    let ref_ = &path_part[at + 1..];
                    if !repo_path.is_empty() && !ref_.is_empty() {
                        let new_url = format!("{}://{host}/{repo_path}", &url[..scheme_end]);
                        let trimmed = new_url.trim_end_matches('/').to_string();
                        return (trimmed, Some(ref_.to_string()));
                    }
                }
            }
            return (url.to_string(), None);
        }
    }

    // Bare form `host/path[@ref]`
    if let Some(slash) = url.find('/') {
        let host = &url[..slash];
        let path_with_ref = &url[slash + 1..];
        if let Some(at) = path_with_ref.find('@') {
            let repo_path = &path_with_ref[..at];
            let ref_ = &path_with_ref[at + 1..];
            if !repo_path.is_empty() && !ref_.is_empty() {
                return (format!("{host}/{repo_path}"), Some(ref_.to_string()));
            }
        }
    }

    (url.to_string(), None)
}

/// Parse the generic fallback form (no hosted-info match).
fn parse_generic(url: &str) -> Option<GitSource> {
    let (repo_without_ref, ref_) = split_ref(url);
    let (repo, host, path);

    if let Some(rest) = repo_without_ref.strip_prefix("git@") {
        let (h, p) = rest.split_once(':')?;
        host = h.to_string();
        path = p.to_string();
        repo = repo_without_ref.clone();
    } else if repo_without_ref.starts_with("https://")
        || repo_without_ref.starts_with("http://")
        || repo_without_ref.starts_with("ssh://")
        || repo_without_ref.starts_with("git://")
    {
        let scheme_end = repo_without_ref.find("://")?;
        let after = &repo_without_ref[scheme_end + 3..];
        let (h, p) = after.split_once('/')?;
        host = h.to_string();
        path = p.trim_start_matches('/').to_string();
        repo = repo_without_ref.clone();
    } else {
        let (h, p) = repo_without_ref.split_once('/')?;
        if !h.contains('.') && h != "localhost" {
            return None;
        }
        host = h.to_string();
        path = p.to_string();
        repo = format!("https://{repo_without_ref}");
    }

    let normalized_path = path
        .trim_start_matches('/')
        .trim_end_matches(".git")
        .trim_start_matches('/')
        .to_string();
    if host.is_empty() || normalized_path.is_empty() || normalized_path.matches('/').count() < 1 {
        return None;
    }

    Some(GitSource {
        repo,
        host,
        path: normalized_path,
        pinned: ref_.is_some(),
        ref_,
    })
}

/// Apply a known-host shorthand rewrite if the input matches one.
fn parse_known_host(url: &str) -> Option<GitSource> {
    // Forms accepted with the `git:` prefix already stripped:
    //   github:user/repo[@ref]
    //   gitlab:user/repo[@ref]
    //   user/repo[@ref]                        (default: github)
    //   github.com/user/repo[@ref]
    let (repo_without_ref, ref_) = split_ref(url);

    // `host:user/repo` shorthand (e.g. `github:user/repo`).
    if let Some((maybe_host, rest)) = repo_without_ref.split_once(':') {
        // Skip scp-like `git@host:path` which are handled by the generic parser.
        if !maybe_host.contains('/') && !maybe_host.contains('.') && maybe_host != "git@" {
            if let Some(domain) = resolve_host_shorthand(maybe_host) {
                return finalize_shorthand(domain, rest, ref_);
            }
        }
    }

    // Plain `user/repo`
    let segments: Vec<&str> = repo_without_ref.split('/').collect();
    if segments.len() == 2
        && !segments[0].contains('.')
        && !segments[0].contains(':')
        && !segments[0].is_empty()
        && !segments[1].is_empty()
    {
        return finalize_shorthand("github.com", &repo_without_ref, ref_);
    }

    // `host.com/user/repo` — fall into generic later, but catch common cases.
    if let Some(slash) = repo_without_ref.find('/') {
        let host = &repo_without_ref[..slash];
        if host.contains('.') {
            let path = &repo_without_ref[slash + 1..];
            let parts: Vec<&str> = path.split('/').collect();
            if parts.len() >= 2 {
                let path = format!("{}/{}", parts[0], parts[1].trim_end_matches(".git"));
                return Some(GitSource {
                    repo: format!("https://{host}/{path}"),
                    host: host.to_string(),
                    path,
                    pinned: ref_.is_some(),
                    ref_,
                });
            }
        }
    }

    None
}

fn finalize_shorthand(domain: &str, path: &str, ref_: Option<String>) -> Option<GitSource> {
    let path = path.trim_end_matches(".git").trim_start_matches('/');
    if path.split('/').count() < 2 {
        return None;
    }
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    let path = format!("{}/{}", parts[0], parts[1]);
    Some(GitSource {
        repo: format!("https://{domain}/{path}"),
        host: domain.to_string(),
        path,
        pinned: ref_.is_some(),
        ref_,
    })
}

/// Parse a git package source specifier.
///
/// Rules (from pi-mono):
/// - With the `git:` prefix, accept all historical shorthand forms.
/// - Without the `git:` prefix, accept only explicit protocol URLs.
pub fn parse_git_url(source: &str) -> Option<GitSource> {
    let trimmed = source.trim();
    let has_git_prefix = trimmed.starts_with("git:");
    let url = if has_git_prefix {
        trimmed[4..].trim()
    } else {
        trimmed
    };

    if !has_git_prefix {
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("https://")
            || lower.starts_with("http://")
            || lower.starts_with("ssh://")
            || lower.starts_with("git://"))
        {
            return None;
        }
    }

    if let Some(parsed) = parse_known_host(url) {
        return Some(parsed);
    }
    parse_generic(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_git_prefix_or_protocol() {
        assert!(parse_git_url("user/repo").is_none());
        assert!(parse_git_url("github.com/user/repo").is_none());
        assert!(parse_git_url("git:user/repo").is_some());
    }

    #[test]
    fn github_shorthand() {
        let g = parse_git_url("git:user/repo").unwrap();
        assert_eq!(g.host, "github.com");
        assert_eq!(g.path, "user/repo");
        assert_eq!(g.repo, "https://github.com/user/repo");
        assert!(!g.pinned);
    }

    #[test]
    fn github_shorthand_with_ref() {
        let g = parse_git_url("git:user/repo@v1.0.0").unwrap();
        assert_eq!(g.ref_.as_deref(), Some("v1.0.0"));
        assert!(g.pinned);
        assert_eq!(g.path, "user/repo");
    }

    #[test]
    fn host_shorthand_gitlab() {
        let g = parse_git_url("git:gitlab:user/repo").unwrap();
        assert_eq!(g.host, "gitlab.com");
        assert_eq!(g.path, "user/repo");
    }

    #[test]
    fn https_url() {
        let g = parse_git_url("https://github.com/user/repo.git").unwrap();
        assert_eq!(g.host, "github.com");
        assert_eq!(g.path, "user/repo");
        assert!(!g.pinned);
    }

    #[test]
    fn https_url_with_ref() {
        let g = parse_git_url("https://github.com/user/repo@main").unwrap();
        assert_eq!(g.ref_.as_deref(), Some("main"));
        assert!(g.pinned);
        assert_eq!(g.path, "user/repo");
    }

    #[test]
    fn scp_style_url() {
        let g = parse_git_url("git:git@github.com:user/repo.git").unwrap();
        assert_eq!(g.host, "github.com");
        assert_eq!(g.path, "user/repo");
    }

    #[test]
    fn scp_style_without_prefix_not_parsed() {
        // No `git:` prefix and not a protocol URL -> rejected.
        assert!(parse_git_url("git@github.com:user/repo.git").is_none());
    }

    #[test]
    fn invalid_path_rejected() {
        assert!(parse_git_url("git:user").is_none());
        assert!(parse_git_url("git:github:just-user").is_none());
    }

    #[test]
    fn preserves_dot_git_stripped_in_path() {
        let g = parse_git_url("https://gitlab.com/user/repo.git").unwrap();
        assert_eq!(g.path, "user/repo");
    }
}
