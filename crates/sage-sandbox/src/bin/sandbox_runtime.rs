//! Sandbox Runtime — child process that hosts the microVM.
//!
//! Spawned by SandboxBuilder::create(). Uses stdin/stdout as a virtio-console
//! port so the guest agent can communicate with the host.
//!
//! vm.enter() never returns on success — it calls _exit() when the guest shuts down.

use msb_krun::VmBuilder;

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

    tracing::info!(rootfs = %rootfs, vcpus, memory_mib, "starting sandbox runtime");

    // Build the VM.
    // stdin (FD 0) = host→guest data path
    // stdout (FD 1) = guest→host data path
    // The VMM console port bridges these FDs to the guest's /dev/vport0p0.
    let mut builder = VmBuilder::new()
        .machine(|m| m.vcpus(vcpus as u8).memory_mib(memory_mib))
        .fs(|fs| fs.root(&rootfs))
        .console(|c| {
            c.port("agent", libc::STDIN_FILENO, libc::STDOUT_FILENO)
                .disable_implicit()
        })
        .exec(|e| e.path("/init"));

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
