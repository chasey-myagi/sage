//! Guest-side security enforcement — seccomp-bpf, Landlock LSM, resource limits.
//!
//! Security is applied in a specific order:
//! 1. Resource limits (setrlimit) — least restrictive, always available
//! 2. Landlock — filesystem access control (Linux 5.13+)
//! 3. Seccomp — syscall filter (MUST be last — restricts prctl/seccomp itself)

use anyhow::Result;
use sage_protocol::GuestSecurityConfig;

/// Load security config from SAGE_SECURITY env var.
///
/// Returns `Ok(None)` if the env var is not set or empty (dev mode, no enforcement).
/// Returns `Err` if the env var is set but contains invalid JSON — **fail-closed**:
/// a corrupted config must never silently disable security.
pub fn load_config() -> Result<Option<GuestSecurityConfig>> {
    let json = match std::env::var("SAGE_SECURITY").ok().filter(|s| !s.is_empty()) {
        Some(j) => j,
        None => return Ok(None),
    };
    let config = serde_json::from_str(&json)
        .map_err(|e| anyhow::anyhow!("SAGE_SECURITY is set but contains invalid JSON: {e}"))?;
    Ok(Some(config))
}

/// Apply all security policies. Order matters:
/// 1. Resource limits (RLIMIT_NOFILE, RLIMIT_FSIZE)
/// 2. Landlock filesystem access control
/// 3. Seccomp syscall filter (must be last)
pub fn apply(config: &GuestSecurityConfig) -> Result<()> {
    apply_resource_limits(config)?;

    #[cfg(target_os = "linux")]
    {
        if config.landlock {
            apply_landlock(config)?;
        } else {
            tracing::warn!("landlock disabled by config");
        }

        if config.seccomp {
            apply_seccomp()?;
        } else {
            tracing::warn!("seccomp disabled by config");
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        if config.seccomp || config.landlock {
            tracing::warn!("seccomp/landlock enforcement skipped on non-Linux");
        }
    }

    tracing::info!("security policy applied");
    Ok(())
}

/// Minimum values for resource limits — below these the guest agent cannot function.
const MIN_OPEN_FILES: u32 = 32;
const MIN_TMPFS_SIZE_MB: u32 = 16;
const MIN_PROCESSES: u32 = 8;

/// Device paths that Landlock must grant write access to inside the guest.
///
/// - `/dev/null`, `/dev/zero`, `/dev/urandom`: standard device contracts
/// - `/dev/vport0p0`: virtio-console used for host↔guest communication
const LANDLOCK_WRITABLE_DEVS: &[&str] = &["/dev/null", "/dev/zero", "/dev/urandom", "/dev/vport0p0"];

/// Apply resource limits via setrlimit.
fn apply_resource_limits(config: &GuestSecurityConfig) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use nix::sys::resource::{Resource, setrlimit};

        // Clamp to safe minimums — values below these would crash the guest agent.
        let nofile = config.max_open_files.max(MIN_OPEN_FILES) as u64;
        if config.max_open_files < MIN_OPEN_FILES {
            tracing::warn!(
                configured = config.max_open_files,
                enforced = MIN_OPEN_FILES,
                "max_open_files below minimum, clamped"
            );
        }

        // RLIMIT_NOFILE — max open file descriptors
        setrlimit(Resource::RLIMIT_NOFILE, nofile, nofile)
            .map_err(|e| anyhow::anyhow!("setrlimit NOFILE({nofile}): {e}"))?;
        tracing::debug!(max_open_files = nofile, "RLIMIT_NOFILE set");

        // RLIMIT_FSIZE — max file size in bytes
        let fsize_bytes = (config.max_file_size_mb as u64) * 1024 * 1024;
        setrlimit(Resource::RLIMIT_FSIZE, fsize_bytes, fsize_bytes)
            .map_err(|e| anyhow::anyhow!("setrlimit FSIZE({fsize_bytes}): {e}"))?;
        tracing::debug!(max_file_size_mb = config.max_file_size_mb, "RLIMIT_FSIZE set");

        // RLIMIT_NPROC — max processes (prevents fork bombs)
        let nproc = config.max_processes.max(MIN_PROCESSES) as u64;
        if config.max_processes < MIN_PROCESSES {
            tracing::warn!(
                configured = config.max_processes,
                enforced = MIN_PROCESSES,
                "max_processes below minimum, clamped"
            );
        }
        setrlimit(Resource::RLIMIT_NPROC, nproc, nproc)
            .map_err(|e| anyhow::anyhow!("setrlimit NPROC({nproc}): {e}"))?;
        tracing::debug!(max_processes = nproc, "RLIMIT_NPROC set");
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = config;
        tracing::warn!("resource limits skipped on non-Linux");
    }

    Ok(())
}

/// Seccomp-bpf syscall filter — restrictive allowlist.
///
/// Only allows syscalls needed by the guest agent:
/// - File I/O: read, write, openat, close, fstat, lseek, mmap, mprotect, munmap
/// - Process: clone, execve, wait4, exit, exit_group, getpid, getppid
/// - Network: NONE (guest is airgapped)
/// - Forbidden: ptrace, mount, reboot, sethostname, kexec_load
#[cfg(target_os = "linux")]
fn apply_seccomp() -> Result<()> {
    use std::collections::HashMap;

    // Build allowlist: syscall number → empty conditions (unconditional allow).
    // NOTE: Landlock syscalls are NOT needed here — Landlock is applied before
    // seccomp, so those syscalls have already been called by the time this filter
    // is installed. Keeping them would be unnecessary attack surface.
    let allowed: Vec<i64> = vec![
        // File I/O
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_openat,
        libc::SYS_close,
        libc::SYS_fstat,
        libc::SYS_newfstatat,
        libc::SYS_lseek,
        libc::SYS_mmap,
        libc::SYS_mprotect,
        libc::SYS_munmap,
        libc::SYS_brk,
        libc::SYS_ioctl,
        libc::SYS_pread64,
        libc::SYS_pwrite64,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_access,
        libc::SYS_pipe2,
        libc::SYS_dup,
        libc::SYS_dup3,
        libc::SYS_fcntl,
        libc::SYS_flock,
        libc::SYS_fsync,
        libc::SYS_fdatasync,
        libc::SYS_truncate,
        libc::SYS_ftruncate,
        libc::SYS_getdents64,
        libc::SYS_getcwd,
        libc::SYS_chdir,
        libc::SYS_fchdir,
        libc::SYS_mkdirat,
        libc::SYS_unlinkat,
        libc::SYS_renameat2,
        libc::SYS_linkat,
        libc::SYS_symlinkat,
        libc::SYS_readlinkat,
        libc::SYS_fchmod,
        libc::SYS_fchmodat,
        libc::SYS_fchownat,
        libc::SYS_faccessat2,
        libc::SYS_statx,
        libc::SYS_copy_file_range,
        libc::SYS_close_range,
        // Memory
        libc::SYS_madvise,
        libc::SYS_mremap,
        // Process
        libc::SYS_clone,
        libc::SYS_clone3,
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_wait4,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_getpid,
        libc::SYS_getppid,
        libc::SYS_gettid,
        libc::SYS_getuid,
        libc::SYS_getgid,
        libc::SYS_geteuid,
        libc::SYS_getegid,
        libc::SYS_set_tid_address,
        libc::SYS_set_robust_list,
        libc::SYS_prlimit64,
        libc::SYS_getrlimit,
        // NOTE: SYS_setrlimit intentionally excluded — child processes must not
        // be able to raise resource limits set by apply_resource_limits().
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_yield,
        libc::SYS_kill,
        libc::SYS_tgkill,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        // Time
        libc::SYS_clock_gettime,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_gettimeofday,
        // Async I/O (tokio)
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_pwait,
        libc::SYS_eventfd2,
        libc::SYS_futex,
        libc::SYS_timerfd_create,
        libc::SYS_timerfd_settime,
        // Misc
        libc::SYS_getrandom,
        // NOTE: SYS_prctl is needed by glibc/musl for thread setup (PR_SET_NAME,
        // PR_SET_VMA). Argument filtering would be ideal but seccompiler's rule
        // syntax makes it verbose — acceptable risk given the VM isolation layer.
        libc::SYS_prctl,
        libc::SYS_arch_prctl,
        libc::SYS_rseq,
        libc::SYS_uname,
        libc::SYS_umask,
        // NOTE: SYS_seccomp intentionally excluded — the filter is already
        // installed by the time this allowlist takes effect. Allowing it would
        // let child processes load additional filters (even if only stricter ones).
    ];

    let syscall_count = allowed.len();
    let allowed_syscalls: HashMap<i64, Vec<seccompiler::SeccompRule>> = allowed
        .into_iter()
        .map(|nr| (nr as i64, vec![]))
        .collect();

    let filter = seccompiler::SeccompFilter::new(
        allowed_syscalls.into(),
        seccompiler::SeccompAction::KillProcess,
        seccompiler::SeccompAction::Allow,
        std::env::consts::ARCH.try_into().map_err(|e| {
            anyhow::anyhow!("unsupported arch for seccomp: {e}")
        })?,
    )
    .map_err(|e| anyhow::anyhow!("build seccomp filter: {e}"))?;

    let bpf: seccompiler::BpfProgram = filter
        .try_into()
        .map_err(|e| anyhow::anyhow!("compile seccomp BPF: {e}"))?;

    seccompiler::apply_filter(&bpf)
        .map_err(|e| anyhow::anyhow!("apply seccomp filter: {e}"))?;

    tracing::info!(syscall_count, bpf_instructions = bpf.len(), "seccomp-bpf filter loaded");
    Ok(())
}

/// Landlock filesystem access control.
///
/// Restricts file access to allowed_paths (read+write) plus essential system paths (read-only).
#[cfg(target_os = "linux")]
fn apply_landlock(config: &GuestSecurityConfig) -> Result<()> {
    use landlock::{
        ABI, Access, AccessFs, BitFlags, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreatedAttr, RulesetStatus,
    };

    let abi = ABI::V3;
    let read_access = AccessFs::ReadFile | AccessFs::ReadDir | AccessFs::Refer;
    let write_access = read_access | AccessFs::WriteFile | AccessFs::RemoveFile
        | AccessFs::RemoveDir | AccessFs::MakeReg | AccessFs::MakeDir
        | AccessFs::MakeSym | AccessFs::Truncate;

    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .map_err(|e| anyhow::anyhow!("landlock handle_access: {e}"))?
        .create()
        .map_err(|e| anyhow::anyhow!("landlock create: {e}"))?;

    // Allow read+write on configured paths (e.g. /workspace, /tmp)
    for path in &config.allowed_paths {
        match PathFd::new(path) {
            Ok(fd) => {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(fd, write_access))
                    .map_err(|e| anyhow::anyhow!("landlock rule {path}: {e}"))?;
                tracing::debug!(path, "landlock: read+write allowed");
            }
            Err(e) => {
                tracing::warn!(path, error = %e, "landlock: path not found, skipping");
            }
        }
    }

    // Allow read-only on essential system paths
    for path in &["/proc", "/sys", "/bin", "/usr", "/lib", "/etc"] {
        if let Ok(fd) = PathFd::new(path) {
            ruleset = ruleset
                .add_rule(PathBeneath::new(fd, read_access))
                .map_err(|e| anyhow::anyhow!("landlock rule {path}: {e}"))?;
        }
    }

    // /dev needs special treatment: read-only for the directory itself,
    // but specific devices need write access (see LANDLOCK_WRITABLE_DEVS).
    if let Ok(fd) = PathFd::new("/dev") {
        ruleset = ruleset
            .add_rule(PathBeneath::new(fd, read_access))
            .map_err(|e| anyhow::anyhow!("landlock rule /dev: {e}"))?;
    }
    for dev_path in LANDLOCK_WRITABLE_DEVS {
        if let Ok(fd) = PathFd::new(dev_path) {
            ruleset = ruleset
                .add_rule(PathBeneath::new(fd, write_access))
                .map_err(|e| anyhow::anyhow!("landlock rule {dev_path}: {e}"))?;
        }
    }

    let status = ruleset
        .restrict_self()
        .map_err(|e| anyhow::anyhow!("landlock restrict_self: {e}"))?;

    match status.ruleset {
        RulesetStatus::FullyEnforced => {
            tracing::info!("landlock fully enforced");
        }
        RulesetStatus::PartiallyEnforced => {
            tracing::warn!("landlock partially enforced (kernel may not support all features)");
        }
        RulesetStatus::NotEnforced => {
            tracing::warn!("landlock not enforced (kernel too old?)");
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_config (testable helper) ───────────────────────────

    /// Parse JSON into GuestSecurityConfig — mirrors load_config() logic
    /// without touching env vars (Rust 2024 edition: env mutation is unsafe).
    fn parse_config(json: &str) -> Option<GuestSecurityConfig> {
        serde_json::from_str(json).ok()
    }

    // ── Config parsing ───────────────────────────────────────────

    #[test]
    fn parse_valid_full_config() {
        let json = r#"{
            "seccomp": true, "landlock": true,
            "max_file_size_mb": 50, "max_open_files": 128,
            "tmpfs_size_mb": 256, "allowed_paths": ["/workspace"]
        }"#;
        let config = parse_config(json).expect("should parse");
        assert_eq!(config.max_file_size_mb, 50);
        assert_eq!(config.max_open_files, 128);
        assert_eq!(config.tmpfs_size_mb, 256);
        assert_eq!(config.allowed_paths, vec!["/workspace"]);
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(parse_config("not-json{{{").is_none());
    }

    #[test]
    fn parse_partial_json_fills_defaults() {
        let config = parse_config(r#"{"max_file_size_mb": 42}"#).unwrap();
        assert_eq!(config.max_file_size_mb, 42);
        assert!(config.seccomp); // default true
        assert!(config.landlock); // default true
        assert_eq!(config.max_open_files, 256); // default
        assert_eq!(config.tmpfs_size_mb, 512); // default
        assert_eq!(config.allowed_paths, vec!["/workspace", "/tmp"]); // default
    }

    #[test]
    fn parse_empty_object_is_all_defaults() {
        let config = parse_config("{}").unwrap();
        assert_eq!(config, GuestSecurityConfig::default());
    }

    #[test]
    fn parse_seccomp_disabled() {
        let config = parse_config(r#"{"seccomp": false}"#).unwrap();
        assert!(!config.seccomp);
        assert!(config.landlock); // default true
    }

    #[test]
    fn parse_landlock_disabled() {
        let config = parse_config(r#"{"landlock": false}"#).unwrap();
        assert!(config.seccomp); // default true
        assert!(!config.landlock);
    }

    #[test]
    fn parse_both_disabled() {
        let config = parse_config(r#"{"seccomp": false, "landlock": false}"#).unwrap();
        assert!(!config.seccomp);
        assert!(!config.landlock);
    }

    #[test]
    fn parse_custom_allowed_paths() {
        let config =
            parse_config(r#"{"allowed_paths": ["/custom", "/data/input"]}"#).unwrap();
        assert_eq!(config.allowed_paths, vec!["/custom", "/data/input"]);
    }

    #[test]
    fn parse_empty_allowed_paths() {
        let config = parse_config(r#"{"allowed_paths": []}"#).unwrap();
        assert!(config.allowed_paths.is_empty());
    }

    #[test]
    fn parse_max_open_files_zero() {
        let config = parse_config(r#"{"max_open_files": 0}"#).unwrap();
        assert_eq!(config.max_open_files, 0);
    }

    #[test]
    fn parse_large_tmpfs_size() {
        let config = parse_config(r#"{"tmpfs_size_mb": 4096}"#).unwrap();
        assert_eq!(config.tmpfs_size_mb, 4096);
    }

    // ── apply() smoke tests ──────────────────────────────────────

    #[test]
    fn apply_with_all_disabled_succeeds_on_non_linux() {
        let config = GuestSecurityConfig {
            seccomp: false,
            landlock: false,
            allowed_paths: vec![],
            ..Default::default()
        };
        // On macOS, seccomp/landlock are no-ops, resource limits are skipped
        #[cfg(not(target_os = "linux"))]
        {
            apply(&config).expect("should succeed on non-Linux");
        }
        // Suppress unused variable warning on Linux
        #[cfg(target_os = "linux")]
        {
            let _ = config;
        }
    }

    // ── GuestSecurityConfig serialization ─────────────────────────

    #[test]
    fn guest_security_config_json_roundtrip() {
        let config = GuestSecurityConfig {
            seccomp: true,
            landlock: false,
            max_file_size_mb: 200,
            max_open_files: 512,
            tmpfs_size_mb: 1024,
            max_processes: 128,
            allowed_paths: vec!["/a".into(), "/b".into()],
        };
        let json = serde_json::to_string(&config).unwrap();
        let roundtripped: GuestSecurityConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, roundtripped);
    }

    #[test]
    fn guest_security_config_default_values() {
        let config = GuestSecurityConfig::default();
        assert!(config.seccomp);
        assert!(config.landlock);
        assert_eq!(config.max_file_size_mb, 100);
        assert_eq!(config.max_open_files, 256);
        assert_eq!(config.tmpfs_size_mb, 512);
        assert_eq!(config.allowed_paths, vec!["/workspace", "/tmp"]);
    }

    #[test]
    fn guest_security_config_full_to_json_has_all_fields() {
        let config = GuestSecurityConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"seccomp\""));
        assert!(json.contains("\"landlock\""));
        assert!(json.contains("\"max_file_size_mb\""));
        assert!(json.contains("\"max_open_files\""));
        assert!(json.contains("\"tmpfs_size_mb\""));
        assert!(json.contains("\"allowed_paths\""));
    }

    // ── Error path tests ─────────────────────────────────────────

    #[test]
    fn parse_wrong_type_seccomp_string_instead_of_bool() {
        assert!(parse_config(r#"{"seccomp": "yes"}"#).is_none());
    }

    #[test]
    fn parse_wrong_type_max_open_files_string() {
        assert!(parse_config(r#"{"max_open_files": "many"}"#).is_none());
    }

    #[test]
    fn parse_wrong_type_allowed_paths_not_array() {
        assert!(parse_config(r#"{"allowed_paths": "not-an-array"}"#).is_none());
    }

    #[test]
    fn parse_negative_value_for_u32_field() {
        assert!(parse_config(r#"{"max_file_size_mb": -1}"#).is_none());
    }

    #[test]
    fn parse_null_seccomp_uses_default() {
        // JSON null for a field with #[serde(default)] should use default
        let config = parse_config(r#"{"seccomp": null}"#);
        // serde: null for bool field is error (not missing), so this fails
        assert!(config.is_none());
    }

    #[test]
    fn parse_unknown_fields_are_ignored() {
        // serde default behavior: unknown fields are silently ignored
        let config = parse_config(r#"{"seccomp": false, "unknown_field": 42, "extra": "data"}"#)
            .expect("should parse ignoring unknown fields");
        assert!(!config.seccomp);
        assert!(config.landlock); // default
    }

    #[test]
    fn parse_json_array_instead_of_object() {
        assert!(parse_config(r#"[1, 2, 3]"#).is_none());
    }

    #[test]
    fn parse_empty_string() {
        assert!(parse_config("").is_none());
    }

    // ── Boundary tests ───────────────────────────────────────────

    #[test]
    fn parse_max_file_size_mb_zero() {
        let config = parse_config(r#"{"max_file_size_mb": 0}"#).unwrap();
        assert_eq!(config.max_file_size_mb, 0);
    }

    #[test]
    fn parse_u32_max_values() {
        let json = format!(
            r#"{{"max_file_size_mb": {}, "max_open_files": {}, "tmpfs_size_mb": {}}}"#,
            u32::MAX,
            u32::MAX,
            u32::MAX
        );
        let config = parse_config(&json).unwrap();
        assert_eq!(config.max_file_size_mb, u32::MAX);
        assert_eq!(config.max_open_files, u32::MAX);
        assert_eq!(config.tmpfs_size_mb, u32::MAX);
    }

    #[test]
    fn parse_overflow_u32_field_returns_none() {
        let too_big = (u32::MAX as u64) + 1;
        let json = format!(r#"{{"max_file_size_mb": {too_big}}}"#);
        assert!(parse_config(&json).is_none());
    }

    #[test]
    fn parse_allowed_paths_with_empty_string() {
        let config = parse_config(r#"{"allowed_paths": [""]}"#).unwrap();
        assert_eq!(config.allowed_paths, vec![""]);
    }

    #[test]
    fn parse_allowed_paths_with_unicode() {
        let config = parse_config(r#"{"allowed_paths": ["/data/日本語", "/tmp/émoji"]}"#).unwrap();
        assert_eq!(config.allowed_paths.len(), 2);
        assert_eq!(config.allowed_paths[0], "/data/日本語");
    }

    #[test]
    fn parse_allowed_paths_with_path_traversal() {
        // The config layer just parses — enforcement is Landlock's job
        let config = parse_config(r#"{"allowed_paths": ["/tmp/../etc/passwd"]}"#).unwrap();
        assert_eq!(config.allowed_paths, vec!["/tmp/../etc/passwd"]);
    }

    #[test]
    fn parse_many_allowed_paths() {
        let paths: Vec<String> = (0..100).map(|i| format!("/path/{i}")).collect();
        let json = format!(r#"{{"allowed_paths": {}}}"#, serde_json::to_string(&paths).unwrap());
        let config = parse_config(&json).unwrap();
        assert_eq!(config.allowed_paths.len(), 100);
    }

    // ── State / ordering / combination tests ─────────────────────

    #[test]
    fn apply_default_config_succeeds_on_non_linux() {
        #[cfg(not(target_os = "linux"))]
        {
            let config = GuestSecurityConfig::default();
            apply(&config).expect("default config should succeed on non-Linux");
        }
    }

    #[test]
    fn apply_only_seccomp_enabled() {
        #[cfg(not(target_os = "linux"))]
        {
            let config = GuestSecurityConfig {
                seccomp: true,
                landlock: false,
                ..Default::default()
            };
            apply(&config).expect("seccomp-only should succeed on non-Linux");
        }
    }

    #[test]
    fn apply_only_landlock_enabled() {
        #[cfg(not(target_os = "linux"))]
        {
            let config = GuestSecurityConfig {
                seccomp: false,
                landlock: true,
                ..Default::default()
            };
            apply(&config).expect("landlock-only should succeed on non-Linux");
        }
    }

    #[test]
    fn apply_idempotent_on_non_linux() {
        // Multiple apply() calls should not panic
        #[cfg(not(target_os = "linux"))]
        {
            let config = GuestSecurityConfig {
                seccomp: false,
                landlock: false,
                ..Default::default()
            };
            apply(&config).expect("first apply");
            apply(&config).expect("second apply should also succeed");
        }
    }

    // ── Full pipeline: SecurityConfig → JSON → GuestSecurityConfig ──

    #[test]
    fn full_pipeline_runner_config_to_guest_config() {
        // Simulate the host-side: build GuestSecurityConfig, serialize to JSON,
        // then parse as the guest would from SAGE_SECURITY env var.
        let host_config = GuestSecurityConfig {
            seccomp: true,
            landlock: true,
            max_file_size_mb: 75,
            max_open_files: 192,
            tmpfs_size_mb: 384,
            max_processes: 64,
            allowed_paths: vec!["/workspace".into(), "/tmp".into(), "/data".into()],
        };

        // Host serializes
        let json = serde_json::to_string(&host_config).unwrap();

        // Guest deserializes
        let guest_config: GuestSecurityConfig = serde_json::from_str(&json).unwrap();

        // All fields must survive the roundtrip
        assert_eq!(host_config, guest_config);
    }

    #[test]
    fn builder_security_config_none_produces_no_env_var() {
        // When security_config is None, no SAGE_SECURITY should be set.
        // This tests the logical invariant, not the actual env var.
        let config: Option<GuestSecurityConfig> = None;
        let json = config.map(|c| serde_json::to_string(&c).unwrap());
        assert!(json.is_none());
    }

    #[test]
    fn builder_security_config_some_produces_valid_json() {
        let config = Some(GuestSecurityConfig::default());
        let json = config.map(|c| serde_json::to_string(&c).unwrap());
        assert!(json.is_some());
        let parsed: GuestSecurityConfig = serde_json::from_str(&json.unwrap()).unwrap();
        assert_eq!(parsed, GuestSecurityConfig::default());
    }

    // ── tmpfs size format test ───────────────────────────────────

    #[test]
    fn tmpfs_mount_option_format() {
        // Verify the tmpfs mount option string format used in init.rs
        let tmpfs_size_mb = 512u32;
        let opts = format!("size={}m", tmpfs_size_mb);
        assert_eq!(opts, "size=512m");

        let tmpfs_size_mb = 0u32;
        let opts = format!("size={}m", tmpfs_size_mb);
        assert_eq!(opts, "size=0m");

        let tmpfs_size_mb = u32::MAX;
        let opts = format!("size={}m", tmpfs_size_mb);
        assert_eq!(opts, format!("size={}m", u32::MAX));
    }

    // ── Regression: Landlock must allow console device ───────────

    #[test]
    fn test_fix_landlock_allows_console_device() {
        // The virtio-console (/dev/vport0p0) must be in the writable device list,
        // otherwise Landlock blocks the guest from reaching Ready state.
        assert!(
            LANDLOCK_WRITABLE_DEVS.contains(&"/dev/vport0p0"),
            "Landlock must allow write access to virtio console device"
        );
    }
}
