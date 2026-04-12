//! Sandbox Runtime — child process that hosts the microVM.
//!
//! Spawned by SandboxBuilder::create(). Uses stdin/stdout as a virtio-console
//! port so the guest agent can communicate with the host.
//!
//! vm.enter() never returns on success — it calls _exit() when the guest shuts down.

use msb_krun::VmBuilder;
use sage_sandbox::VolumeMount;

fn main() {
    // Use stderr for logging (stdout is the data path to the host)
    tracing_subscriber::fmt()
        .with_env_filter("sandbox_runtime=debug")
        .with_writer(std::io::stderr)
        .init();

    let rootfs = std::env::var("SANDBOX_ROOTFS").unwrap_or_else(|_| {
        eprintln!("SANDBOX_ROOTFS not set");
        std::process::exit(1);
    });
    let krunfw = std::env::var("SANDBOX_KRUNFW").unwrap_or_default();
    let vcpus: usize = std::env::var("SANDBOX_VCPUS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap_or(1);
    let memory_mib: usize = std::env::var("SANDBOX_MEMORY_MIB")
        .unwrap_or_else(|_| "256".into())
        .parse()
        .unwrap_or(256);

    // Parse volume mounts from JSON env var
    let volumes: Vec<VolumeMount> = match std::env::var("SANDBOX_VOLUMES") {
        Ok(s) if !s.is_empty() => match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("FATAL: failed to parse SANDBOX_VOLUMES: {e}");
                std::process::exit(1);
            }
        },
        _ => Vec::new(),
    };

    tracing::info!(
        rootfs = %rootfs,
        vcpus,
        memory_mib,
        volumes = volumes.len(),
        "starting sandbox runtime"
    );

    // Validate volume tags are unique (e.g. "/a/b" and "/a_b" both map to "vol_a_b")
    {
        let mut seen = std::collections::HashSet::new();
        for vol in &volumes {
            let tag = volume_tag(&vol.guest_path);
            if !seen.insert(tag.clone()) {
                eprintln!(
                    "FATAL: duplicate virtiofs tag '{}' (from guest_path '{}'). \
                     Paths like '/a/b' and '/a_b' collide — use distinct mount paths.",
                    tag, vol.guest_path
                );
                std::process::exit(1);
            }
        }
    }

    // Build the VM.
    // stdin (FD 0) = host→guest data path
    // stdout (FD 1) = guest→host data path
    // The VMM console port bridges these FDs to the guest's /dev/vport0p0.
    let mut builder = VmBuilder::new()
        .machine(|m| m.vcpus(vcpus as u8).memory_mib(memory_mib))
        .fs(|fs| {
            let mut fs = fs.root(&rootfs);
            // Add each volume as a virtiofs share with a tag derived from
            // the guest mount path (e.g. "/workspace" → "vol_workspace").
            for vol in &volumes {
                let tag = volume_tag(&vol.guest_path);
                tracing::info!(tag = %tag, host = %vol.host_path, guest = %vol.guest_path, "adding volume");
                fs = fs.tag(&tag).path(&vol.host_path);
            }
            fs
        })
        .console(|c| {
            c.port("agent", libc::STDIN_FILENO, libc::STDOUT_FILENO)
                .disable_implicit()
        })
        .exec(|e| {
            let mut e = e.path("/init");
            // Pass volume mappings to the guest agent so it can mount them.
            // Format: JSON array of {tag, guest_path, read_only}.
            if !volumes.is_empty() {
                let guest_volumes: Vec<GuestVolume> = volumes
                    .iter()
                    .map(|v| GuestVolume {
                        tag: volume_tag(&v.guest_path),
                        guest_path: v.guest_path.clone(),
                        read_only: v.read_only,
                    })
                    .collect();
                if let Ok(json) = serde_json::to_string(&guest_volumes) {
                    e = e.env("SAGE_VOLUMES", &json);
                }
            }
            e
        });

    // Set kernel firmware path if provided
    if !krunfw.is_empty() {
        builder = builder.kernel(|k| k.krunfw_path(&krunfw));
    }

    let vm = match builder.build() {
        Ok(vm) => vm,
        Err(e) => {
            tracing::error!("VM build failed: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!("VM built, entering...");

    // enter() never returns on success — the VMM calls _exit() on guest shutdown.
    // Return type is Result<Infallible>: Ok is unreachable, only Err returns.
    match vm.enter() {
        Ok(infallible) => match infallible {},
        Err(e) => {
            tracing::error!("VM enter failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Derive a virtiofs tag from a guest mount path.
/// e.g. "/workspace" → "vol_workspace", "/data/input" → "vol_data_input"
fn volume_tag(guest_path: &str) -> String {
    let clean = guest_path.trim_start_matches('/').replace('/', "_");
    format!("vol_{clean}")
}

/// Volume info passed to the guest agent via SAGE_VOLUMES env var.
#[derive(serde::Serialize)]
struct GuestVolume {
    tag: String,
    guest_path: String,
    read_only: bool,
}
