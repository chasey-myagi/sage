//! Tools manager — download and manage helper binaries (fd, rg).
//!
//! Translated from pi-mono `packages/coding-agent/src/utils/tools-manager.ts`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn is_offline_mode_enabled() -> bool {
    match std::env::var("SAGE_OFFLINE").or_else(|_| std::env::var("PI_OFFLINE")) {
        Ok(val) => val == "1" || val.eq_ignore_ascii_case("true") || val.eq_ignore_ascii_case("yes"),
        Err(_) => false,
    }
}

fn tools_dir() -> PathBuf {
    // Use the same bin directory convention as the TypeScript side:
    // $XDG_DATA_HOME/sage/bin or ~/.local/share/sage/bin.
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("sage").join("bin");
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".local")
                .join("share")
        })
        .join("sage")
        .join("bin")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Fd,
    Rg,
}

impl Tool {
    pub fn binary_name(self) -> &'static str {
        match self {
            Tool::Fd => "fd",
            Tool::Rg => "rg",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Tool::Fd => "fd",
            Tool::Rg => "ripgrep",
        }
    }

    pub fn github_repo(self) -> &'static str {
        match self {
            Tool::Fd => "sharkdp/fd",
            Tool::Rg => "BurntSushi/ripgrep",
        }
    }

    /// Return the release asset name for the current platform, or `None` if unsupported.
    pub fn asset_name(self, version: &str) -> Option<String> {
        let (os, arch) = platform_triple();
        match self {
            Tool::Fd => {
                let name = match os {
                    "macos" => format!("fd-v{version}-{arch}-apple-darwin.tar.gz"),
                    "linux" => format!("fd-v{version}-{arch}-unknown-linux-gnu.tar.gz"),
                    "windows" => format!("fd-v{version}-{arch}-pc-windows-msvc.zip"),
                    _ => return None,
                };
                Some(name)
            }
            Tool::Rg => {
                let name = match os {
                    "macos" => format!("ripgrep-{version}-{arch}-apple-darwin.tar.gz"),
                    "linux" => {
                        if arch == "aarch64" {
                            format!("ripgrep-{version}-aarch64-unknown-linux-gnu.tar.gz")
                        } else {
                            format!("ripgrep-{version}-x86_64-unknown-linux-musl.tar.gz")
                        }
                    }
                    "windows" => format!("ripgrep-{version}-{arch}-pc-windows-msvc.zip"),
                    _ => return None,
                };
                Some(name)
            }
        }
    }

    pub fn tag_prefix(self) -> &'static str {
        match self {
            Tool::Fd => "v",
            Tool::Rg => "",
        }
    }
}

fn platform_triple() -> (&'static str, &'static str) {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    };
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    (os, arch)
}

fn command_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|_| true)
        .unwrap_or(false)
}

/// Return the path to the given tool, or `None` if not found.
pub fn get_tool_path(tool: Tool) -> Option<String> {
    let dir = tools_dir();
    let bin_name = if cfg!(target_os = "windows") {
        format!("{}.exe", tool.binary_name())
    } else {
        tool.binary_name().to_owned()
    };
    let local = dir.join(&bin_name);
    if local.exists() {
        return Some(local.to_string_lossy().into_owned());
    }
    if command_exists(tool.binary_name()) {
        return Some(tool.binary_name().to_owned());
    }
    None
}

/// Fetch the latest release version from the GitHub API.
async fn get_latest_version(repo: &str) -> anyhow::Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(10_000))
        .build()?;
    let resp = client
        .get(&url)
        .header("User-Agent", "sage-coding-agent")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("GitHub API error: {}", resp.status()));
    }
    let data: serde_json::Value = resp.json().await?;
    let tag = data["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_owned();
    Ok(tag)
}

/// Download a file to `dest`.
async fn download_file(url: &str, dest: &Path) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(120_000))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("Failed to download {url}: {}", resp.status()));
    }
    let bytes = resp.bytes().await?;
    std::fs::write(dest, &bytes)?;
    Ok(())
}

fn find_binary_recursively(root: &Path, binary: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                stack.push(path);
            } else if entry.file_name() == binary {
                return Some(path);
            }
        }
    }
    None
}

/// Download and install the given tool, returning its path.
async fn download_tool(tool: Tool) -> anyhow::Result<String> {
    let version = get_latest_version(tool.github_repo()).await?;
    let asset_name = tool
        .asset_name(&version)
        .ok_or_else(|| anyhow::anyhow!("Unsupported platform for {}", tool.display_name()))?;

    let dir = tools_dir();
    std::fs::create_dir_all(&dir)?;

    let download_url = format!(
        "https://github.com/{}/releases/download/{}{}/{}",
        tool.github_repo(),
        tool.tag_prefix(),
        version,
        asset_name,
    );
    let archive_path = dir.join(&asset_name);
    download_file(&download_url, &archive_path).await?;

    let bin_ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
    let binary_name = format!("{}{}", tool.binary_name(), bin_ext);
    let binary_path = dir.join(&binary_name);

    // Extract archive
    let extract_dir = dir.join(format!("extract_tmp_{}", std::process::id()));
    std::fs::create_dir_all(&extract_dir)?;

    let result = (|| -> anyhow::Result<()> {
        if asset_name.ends_with(".tar.gz") {
            let status = Command::new("tar")
                .args(["xzf", &archive_path.to_string_lossy(), "-C", &extract_dir.to_string_lossy()])
                .status()?;
            if !status.success() {
                return Err(anyhow::anyhow!("tar extraction failed"));
            }
        } else if asset_name.ends_with(".zip") {
            // Use unzip as a fallback (zip feature optional)
            let status = Command::new("unzip")
                .args(["-q", &archive_path.to_string_lossy(), "-d", &extract_dir.to_string_lossy()])
                .status()?;
            if !status.success() {
                return Err(anyhow::anyhow!("unzip extraction failed"));
            }
        } else {
            return Err(anyhow::anyhow!("Unsupported archive format: {}", asset_name));
        }

        // Find the binary in the extracted files.
        let extracted = find_binary_recursively(&extract_dir, &binary_name)
            .ok_or_else(|| anyhow::anyhow!("Binary {} not found in archive", binary_name))?;

        std::fs::rename(&extracted, &binary_path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))?;
        }

        Ok(())
    })();

    let _ = std::fs::remove_file(&archive_path);
    let _ = std::fs::remove_dir_all(&extract_dir);

    result?;
    Ok(binary_path.to_string_lossy().into_owned())
}

/// Ensure a tool is available, downloading if necessary.
/// Returns the path to the tool, or `None` if unavailable.
pub async fn ensure_tool(tool: Tool, silent: bool) -> Option<String> {
    if let Some(path) = get_tool_path(tool) {
        return Some(path);
    }

    if is_offline_mode_enabled() {
        if !silent {
            eprintln!("{} not found. Offline mode enabled, skipping download.", tool.display_name());
        }
        return None;
    }

    if !silent {
        eprintln!("{} not found. Downloading...", tool.display_name());
    }

    match download_tool(tool).await {
        Ok(path) => {
            if !silent {
                eprintln!("{} installed to {}", tool.display_name(), path);
            }
            Some(path)
        }
        Err(e) => {
            if !silent {
                eprintln!("Failed to download {}: {}", tool.display_name(), e);
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fd_asset_name_macos() {
        // Only verify format, not the OS-specific value.
        let name = Tool::Fd.asset_name("10.0.0");
        // On any platform the name should be Some if platform is supported.
        if cfg!(any(target_os = "macos", target_os = "linux", target_os = "windows")) {
            assert!(name.is_some());
            let n = name.unwrap();
            assert!(n.contains("fd"));
            assert!(n.contains("10.0.0"));
        }
    }

    #[test]
    fn rg_asset_name_contains_version() {
        let name = Tool::Rg.asset_name("14.1.0");
        if cfg!(any(target_os = "macos", target_os = "linux", target_os = "windows")) {
            assert!(name.is_some());
            let n = name.unwrap();
            assert!(n.contains("14.1.0"));
        }
    }

    #[test]
    fn command_exists_false_for_nonsense() {
        assert!(!command_exists("__definitely_nonexistent_binary_sage__"));
    }

    #[test]
    fn tool_binary_names() {
        assert_eq!(Tool::Fd.binary_name(), "fd");
        assert_eq!(Tool::Rg.binary_name(), "rg");
    }
}
