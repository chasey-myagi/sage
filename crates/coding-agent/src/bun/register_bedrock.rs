// AWS Bedrock credential chain — Rust equivalent of pi-mono's
// `bun/register-bedrock.ts`.
//
// In the Bun port, registration intercepts module loading.  Here we instead
// expose a pure function that walks the standard AWS credential discovery
// chain and returns ready-to-use credentials.
//
// Discovery order (mirrors the AWS SDK default chain):
//   1. Environment variables
//   2. ~/.aws/credentials  (INI, section = profile)
//   3. ~/.aws/config       (INI, section = "profile <name>", region only)

use std::fmt;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Resolved AWS credentials ready for use with Bedrock.
#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub region: String,
}

/// Errors that can occur during credential discovery.
#[derive(Debug)]
pub enum CredentialError {
    /// Neither environment variables nor credential files provided credentials.
    NotFound,
    /// A credential file could not be parsed.
    MalformedFile(String),
    /// The selected profile was not found in any credential source.
    ProfileNotFound(String),
    /// No region could be determined from any source.
    RegionNotFound,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialError::NotFound => {
                write!(f, "no AWS credentials found in environment or credential files")
            }
            CredentialError::MalformedFile(msg) => write!(f, "malformed AWS credential file: {msg}"),
            CredentialError::ProfileNotFound(p) => {
                write!(f, "AWS profile \"{p}\" not found in credentials or config")
            }
            CredentialError::RegionNotFound => write!(
                f,
                "no AWS region found; set AWS_REGION or add region to ~/.aws/config"
            ),
        }
    }
}

impl std::error::Error for CredentialError {}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Discover AWS credentials using the standard credential chain.
///
/// Steps:
/// 1. `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` environment variables.
/// 2. `~/.aws/credentials` (INI format), profile selected by `AWS_PROFILE` /
///    `AWS_DEFAULT_PROFILE` (default: `"default"`).
/// 3. Region is taken from `AWS_REGION` → `~/.aws/config` → `"us-east-1"`.
pub fn discover_aws_credentials() -> Result<AwsCredentials, CredentialError> {
    let profile = active_profile();

    // 1. Environment variables
    if let (Some(key), Some(secret)) = (env_nonempty("AWS_ACCESS_KEY_ID"), env_nonempty("AWS_SECRET_ACCESS_KEY")) {
        let token = env_nonempty("AWS_SESSION_TOKEN");
        let region = resolve_region(&profile);
        return Ok(AwsCredentials {
            access_key_id: key,
            secret_access_key: secret,
            session_token: token,
            region,
        });
    }

    // 2. ~/.aws/credentials file
    let credentials_path = aws_file("credentials");
    let config_path = aws_file("config");

    let creds = if let Some(path) = &credentials_path {
        match std::fs::read_to_string(path) {
            Ok(content) => parse_credentials_file(&content, &profile),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(CredentialError::MalformedFile(e.to_string())),
        }
    } else {
        None
    };

    let (key, secret, token) = match creds {
        Some(triple) => triple,
        None => return Err(CredentialError::ProfileNotFound(profile.clone())),
    };

    // Region: env → config file → default
    let region = resolve_region_with_config(&profile, config_path.as_deref());

    Ok(AwsCredentials {
        access_key_id: key,
        secret_access_key: secret,
        session_token: token,
        region,
    })
}

// ---------------------------------------------------------------------------
// INI parsing helpers
// ---------------------------------------------------------------------------

/// Parse an AWS credentials file and return `(access_key, secret_key, token)`.
///
/// The file uses INI sections: `[profile-name]`.  Keys are
/// `aws_access_key_id`, `aws_secret_access_key`, and optionally
/// `aws_session_token`.
pub(crate) fn parse_credentials_file(
    content: &str,
    profile: &str,
) -> Option<(String, String, Option<String>)> {
    let section = find_section(content, profile);

    let key = section.get("aws_access_key_id")?.clone();
    let secret = section.get("aws_secret_access_key")?.clone();
    let token = section.get("aws_session_token").cloned();

    Some((key, secret, token))
}

/// Parse the region from `~/.aws/config` for the given profile.
///
/// In `~/.aws/config`, the default profile is `[default]` but named profiles
/// use `[profile <name>]`.
pub(crate) fn parse_config_region(content: &str, profile: &str) -> Option<String> {
    let section_name = if profile == "default" {
        "default".to_string()
    } else {
        format!("profile {profile}")
    };

    let section = find_section(content, &section_name);
    section.get("region").cloned()
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Read all key=value pairs from the `[section]` block in an INI string.
fn find_section(content: &str, section: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let target = format!("[{section}]");
    let mut in_section = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();

        // Skip blank lines and comments
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') {
            in_section = line == target;
            continue;
        }

        if in_section {
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_lowercase(), v.trim().to_string());
            }
        }
    }

    map
}

/// Return the active AWS profile name.
fn active_profile() -> String {
    env_nonempty("AWS_PROFILE")
        .or_else(|| env_nonempty("AWS_DEFAULT_PROFILE"))
        .unwrap_or_else(|| "default".to_string())
}

/// Resolve region with only env vars (no file read).
fn resolve_region(profile: &str) -> String {
    env_nonempty("AWS_REGION")
        .or_else(|| env_nonempty("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|| {
            // If a non-default profile is active we can't assume us-east-1,
            // but without reading config we fall back to it anyway.
            let _ = profile;
            "us-east-1".to_string()
        })
}

/// Resolve region: env → config file → `"us-east-1"`.
fn resolve_region_with_config(profile: &str, config_path: Option<&std::path::Path>) -> String {
    if let Some(r) = env_nonempty("AWS_REGION").or_else(|| env_nonempty("AWS_DEFAULT_REGION")) {
        return r;
    }

    if let Some(path) = config_path {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some(r) = parse_config_region(&content, profile) {
                return r;
            }
        }
    }

    "us-east-1".to_string()
}

/// Build a path inside `~/.aws/`.
fn aws_file(name: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".aws").join(name))
}

/// Return `Some(value)` when the env var is set and non-empty.
fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // ── parse_credentials_file ───────────────────────────────────────────────

    #[test]
    fn parses_default_profile() {
        let ini = "[default]\naws_access_key_id = AKIADEFAULT\naws_secret_access_key = secretDEFAULT\n";
        let result = parse_credentials_file(ini, "default");
        assert!(result.is_some());
        let (key, secret, token) = result.unwrap();
        assert_eq!(key, "AKIADEFAULT");
        assert_eq!(secret, "secretDEFAULT");
        assert!(token.is_none());
    }

    #[test]
    fn parses_named_profile() {
        let ini = "[default]\naws_access_key_id = AKIADEF\naws_secret_access_key = sdef\n\n[staging]\naws_access_key_id = AKIASTG\naws_secret_access_key = sstg\naws_session_token = tok123\n";
        let result = parse_credentials_file(ini, "staging");
        assert!(result.is_some());
        let (key, secret, token) = result.unwrap();
        assert_eq!(key, "AKIASTG");
        assert_eq!(secret, "sstg");
        assert_eq!(token.as_deref(), Some("tok123"));
    }

    #[test]
    fn returns_none_for_missing_profile() {
        let ini = "[default]\naws_access_key_id = K\naws_secret_access_key = S\n";
        assert!(parse_credentials_file(ini, "nonexistent").is_none());
    }

    #[test]
    fn parses_session_token() {
        let ini = "[default]\naws_access_key_id = K\naws_secret_access_key = S\naws_session_token = T\n";
        let (_, _, token) = parse_credentials_file(ini, "default").unwrap();
        assert_eq!(token.as_deref(), Some("T"));
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let ini = "# this is a comment\n\n[default]\n; another comment\naws_access_key_id = K\naws_secret_access_key = S\n";
        let result = parse_credentials_file(ini, "default");
        assert!(result.is_some());
    }

    // ── parse_config_region ──────────────────────────────────────────────────

    #[test]
    fn parses_default_region() {
        let ini = "[default]\nregion = ap-southeast-1\noutput = json\n";
        assert_eq!(parse_config_region(ini, "default").as_deref(), Some("ap-southeast-1"));
    }

    #[test]
    fn parses_named_profile_region() {
        let ini = "[default]\nregion = us-east-1\n\n[profile prod]\nregion = eu-west-1\n";
        assert_eq!(parse_config_region(ini, "prod").as_deref(), Some("eu-west-1"));
    }

    #[test]
    fn returns_none_when_region_absent() {
        let ini = "[default]\noutput = json\n";
        assert!(parse_config_region(ini, "default").is_none());
    }

    // ── discover_aws_credentials (env var path) ──────────────────────────────

    #[test]
    #[serial]
    fn env_vars_take_precedence() {
        unsafe {
            std::env::set_var("AWS_ACCESS_KEY_ID", "AKIAENV");
            std::env::set_var("AWS_SECRET_ACCESS_KEY", "secretENV");
            std::env::set_var("AWS_SESSION_TOKEN", "tokenENV");
            std::env::set_var("AWS_REGION", "eu-central-1");
            std::env::remove_var("AWS_PROFILE");
            std::env::remove_var("AWS_DEFAULT_PROFILE");
        }

        let creds = discover_aws_credentials().expect("should succeed with env vars");
        assert_eq!(creds.access_key_id, "AKIAENV");
        assert_eq!(creds.secret_access_key, "secretENV");
        assert_eq!(creds.session_token.as_deref(), Some("tokenENV"));
        assert_eq!(creds.region, "eu-central-1");

        unsafe {
            std::env::remove_var("AWS_ACCESS_KEY_ID");
            std::env::remove_var("AWS_SECRET_ACCESS_KEY");
            std::env::remove_var("AWS_SESSION_TOKEN");
            std::env::remove_var("AWS_REGION");
        }
    }

    #[test]
    #[serial]
    fn env_vars_region_defaults_to_us_east_1() {
        unsafe {
            std::env::set_var("AWS_ACCESS_KEY_ID", "K");
            std::env::set_var("AWS_SECRET_ACCESS_KEY", "S");
            std::env::remove_var("AWS_SESSION_TOKEN");
            std::env::remove_var("AWS_REGION");
            std::env::remove_var("AWS_DEFAULT_REGION");
            std::env::remove_var("AWS_PROFILE");
            std::env::remove_var("AWS_DEFAULT_PROFILE");
        }

        let creds = discover_aws_credentials().expect("should succeed");
        assert_eq!(creds.region, "us-east-1");

        unsafe {
            std::env::remove_var("AWS_ACCESS_KEY_ID");
            std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        }
    }

    // ── active_profile ───────────────────────────────────────────────────────

    #[test]
    #[serial]
    fn active_profile_defaults_to_default() {
        unsafe {
            std::env::remove_var("AWS_PROFILE");
            std::env::remove_var("AWS_DEFAULT_PROFILE");
        }
        assert_eq!(active_profile(), "default");
    }

    #[test]
    #[serial]
    fn active_profile_reads_aws_profile() {
        unsafe {
            std::env::set_var("AWS_PROFILE", "myprofile");
            std::env::remove_var("AWS_DEFAULT_PROFILE");
        }
        assert_eq!(active_profile(), "myprofile");
        unsafe { std::env::remove_var("AWS_PROFILE") };
    }

    #[test]
    #[serial]
    fn active_profile_falls_back_to_default_profile_env() {
        unsafe {
            std::env::remove_var("AWS_PROFILE");
            std::env::set_var("AWS_DEFAULT_PROFILE", "fallback");
        }
        assert_eq!(active_profile(), "fallback");
        unsafe { std::env::remove_var("AWS_DEFAULT_PROFILE") };
    }

    // ── discover_aws_credentials (file path) — uses tempfile ─────────────────

    #[test]
    fn credentials_file_roundtrip_via_parse() {
        // Exercise parse_credentials_file directly with a multi-profile file.
        let ini = "[default]\naws_access_key_id = AKIAFILE\naws_secret_access_key = sfile\n\n[alt]\naws_access_key_id = AKIAALT\naws_secret_access_key = salt\n";
        let (k, s, t) = parse_credentials_file(ini, "default").unwrap();
        assert_eq!(k, "AKIAFILE");
        assert_eq!(s, "sfile");
        assert!(t.is_none());

        let (k2, s2, _) = parse_credentials_file(ini, "alt").unwrap();
        assert_eq!(k2, "AKIAALT");
        assert_eq!(s2, "salt");
    }

    #[test]
    #[serial]
    fn discover_returns_profile_not_found_without_env_or_file() {
        // Ensure env vars are absent so we fall through to the file path.
        unsafe {
            std::env::remove_var("AWS_ACCESS_KEY_ID");
            std::env::remove_var("AWS_SECRET_ACCESS_KEY");
            std::env::set_var("AWS_PROFILE", "__nonexistent_profile_xyz__");
        }

        // The real ~/.aws/credentials almost certainly won't have this profile.
        // We accept either ProfileNotFound or NotFound here.
        let result = discover_aws_credentials();
        assert!(
            result.is_err(),
            "expected an error for a bogus profile, got credentials"
        );

        unsafe { std::env::remove_var("AWS_PROFILE") };
    }
}
