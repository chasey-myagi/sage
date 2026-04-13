use anyhow::Result;

/// Volume info received from the host via SAGE_VOLUMES env var.
#[derive(serde::Deserialize)]
struct GuestVolume {
    tag: String,
    guest_path: String,
    read_only: bool,
}

/// Mount essential filesystems for the guest environment.
///
/// This only runs inside a Linux VM, so we use Linux-specific mount calls.
/// On non-Linux (host dev machine), this is a no-op.
pub fn mount_filesystems(tmpfs_size_mb: u32) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use nix::mount::{MsFlags, mount};
        let none: Option<&str> = None;

        mount(
            Some("proc"),
            "/proc",
            Some("proc"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            none,
        )
        .map_err(|e| anyhow::anyhow!("mount /proc: {e}"))?;

        mount(
            Some("sysfs"),
            "/sys",
            Some("sysfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            none,
        )
        .map_err(|e| anyhow::anyhow!("mount /sys: {e}"))?;

        mount(
            Some("devtmpfs"),
            "/dev",
            Some("devtmpfs"),
            MsFlags::MS_NOSUID,
            none,
        )
        .map_err(|e| anyhow::anyhow!("mount /dev: {e}"))?;

        // Mount /tmp with size limit from security config
        let tmpfs_opts = format!("size={}m", tmpfs_size_mb);
        mount(
            Some("tmpfs"),
            "/tmp",
            Some("tmpfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            Some(tmpfs_opts.as_str()),
        )
        .map_err(|e| anyhow::anyhow!("mount /tmp (size={tmpfs_size_mb}m): {e}"))?;

        tracing::debug!(tmpfs_size_mb, "essential filesystems mounted");

        // Mount virtiofs volumes passed via SAGE_VOLUMES env var
        mount_volumes()?;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = tmpfs_size_mb;
        tracing::warn!("mount_filesystems is a no-op on non-Linux (guest-agent only runs in VM)");
    }

    Ok(())
}

/// Mount virtiofs volumes passed by the host via `SAGE_VOLUMES` env var.
///
/// Each volume is a virtiofs share identified by a tag.  The host adds
/// the share via `VmBuilder::fs()` and passes `{tag, guest_path, read_only}`
/// in JSON so we know where to mount it.
#[cfg(target_os = "linux")]
fn mount_volumes() -> Result<()> {
    use nix::mount::{MsFlags, mount};

    let volumes_json = match std::env::var("SAGE_VOLUMES") {
        Ok(s) if !s.is_empty() => s,
        _ => return Ok(()),
    };

    let volumes: Vec<GuestVolume> = serde_json::from_str(&volumes_json)
        .map_err(|e| anyhow::anyhow!("parse SAGE_VOLUMES: {e}"))?;

    for vol in &volumes {
        // Ensure the mount point directory exists
        std::fs::create_dir_all(&vol.guest_path)
            .map_err(|e| anyhow::anyhow!("mkdir {}: {e}", vol.guest_path))?;

        // Default: restrictive flags (nosuid + nodev + noexec).
        // Read-only volumes additionally get MS_RDONLY.
        let mut flags = MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC;
        if vol.read_only {
            flags |= MsFlags::MS_RDONLY;
        }

        mount(
            Some(vol.tag.as_str()),
            vol.guest_path.as_str(),
            Some("virtiofs"),
            flags,
            None::<&str>,
        )
        .map_err(|e| anyhow::anyhow!("mount virtiofs {} at {}: {e}", vol.tag, vol.guest_path))?;

        tracing::info!(
            tag = %vol.tag,
            path = %vol.guest_path,
            read_only = vol.read_only,
            "volume mounted"
        );
    }

    if !volumes.is_empty() {
        tracing::debug!(count = volumes.len(), "virtiofs volumes mounted");
    }

    Ok(())
}
