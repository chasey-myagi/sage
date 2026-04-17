//! Minimal msb_krun VM test — Step 0 feasibility verification.
//!
//! # Prerequisites
//!
//! 1. Install microsandbox (provides libkrunfw):
//!    ```bash
//!    curl -fsSL https://get.microsandbox.dev | sh
//!    ```
//!
//! 2. Build and sign with entitlements (macOS HVF requirement):
//!    ```bash
//!    cargo build --example minimal-vm -p agent-sandbox
//!    codesign --entitlements msb-entitlements.plist --force -s - \
//!        target/debug/examples/minimal-vm
//!    ```
//!
//! 3. Run:
//!    ```bash
//!    DYLD_LIBRARY_PATH=~/.microsandbox/lib target/debug/examples/minimal-vm
//!    ```
//!
//! # What this tests
//!
//! - msb_krun compiles on macOS Apple Silicon
//! - VmBuilder API works (rootfs, exec, machine config)
//! - ConsolePortBackend trait implementation for host↔guest I/O
//! - vm.enter() behavior (never returns, calls _exit())
//!
//! # Architecture note
//!
//! vm.enter() **never returns on success** — it calls _exit() when the guest
//! shuts down, terminating the entire process. For production use, the VM must
//! run in a **separate process** (Command::new), not just a thread.

use msb_krun::{ConsolePortBackend, VmBuilder};
use std::io::{self, Write};
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Simple console backend that captures guest output to stdout.
struct SimpleConsoleBackend {
    /// Guest → Host output buffer
    tx_buf: Arc<Mutex<Vec<u8>>>,
    /// Host → Guest input buffer
    rx_buf: Arc<Mutex<Vec<u8>>>,
    /// Wake pipe: [read_fd, write_fd]
    wake_pipe: [i32; 2],
    /// Flag to signal output is ready
    has_output: Arc<AtomicBool>,
}

impl SimpleConsoleBackend {
    fn new() -> io::Result<Self> {
        let mut fds = [0i32; 2];
        unsafe {
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            // Set non-blocking on read end
            let flags = libc::fcntl(fds[0], libc::F_GETFL);
            libc::fcntl(fds[0], libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        Ok(Self {
            tx_buf: Arc::new(Mutex::new(Vec::new())),
            rx_buf: Arc::new(Mutex::new(Vec::new())),
            wake_pipe: fds,
            has_output: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl ConsolePortBackend for SimpleConsoleBackend {
    /// Host → Guest: provide data from rx_buf.
    fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut rx = self.rx_buf.lock().unwrap();
        if rx.is_empty() {
            // Drain the wake pipe
            let mut drain = [0u8; 1];
            unsafe { libc::read(self.wake_pipe[0], drain.as_mut_ptr() as *mut _, 1) };
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "no data"));
        }
        let n = buf.len().min(rx.len());
        buf[..n].copy_from_slice(&rx[..n]);
        rx.drain(..n);
        Ok(n)
    }

    /// Guest → Host: capture in tx_buf and print to stdout.
    fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let mut tx = self.tx_buf.lock().unwrap();
        tx.extend_from_slice(buf);
        self.has_output.store(true, Ordering::Release);

        // Print guest output immediately
        let _ = io::stdout().write_all(buf);
        let _ = io::stdout().flush();

        Ok(buf.len())
    }

    fn read_wake_fd(&self) -> RawFd {
        self.wake_pipe[0]
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("=== msb_krun minimal VM test ===");
    eprintln!();

    // Check for rootfs path
    let rootfs = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/agent-caster-test-rootfs".into());

    if !std::path::Path::new(&rootfs).exists() {
        eprintln!("Rootfs not found at {rootfs}");
        eprintln!("Create a minimal rootfs with busybox:");
        eprintln!("  mkdir -p {rootfs}/{{bin,proc,sys,dev,tmp}}");
        eprintln!("  # Copy a static busybox binary to {rootfs}/bin/busybox");
        eprintln!("  # Create symlinks: ln -s busybox {rootfs}/bin/sh");
        return Ok(());
    }

    eprintln!("Using rootfs: {rootfs}");

    // Check for libkrunfw
    let krunfw_path = std::env::var("LIBKRUNFW_PATH").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.microsandbox/lib/libkrunfw.5.dylib")
    });

    if !std::path::Path::new(&krunfw_path).exists() {
        eprintln!("libkrunfw not found at {krunfw_path}");
        eprintln!("Install microsandbox: curl -fsSL https://get.microsandbox.dev | sh");
        eprintln!("Or set LIBKRUNFW_PATH to the correct location.");
        return Ok(());
    }
    eprintln!("Using libkrunfw: {krunfw_path}");

    // Create console backend
    let backend = SimpleConsoleBackend::new()?;

    eprintln!("Building VM...");

    let vm = VmBuilder::new()
        .machine(|m| m.vcpus(1).memory_mib(512))
        .kernel(|k| k.krunfw_path(&krunfw_path))
        .fs(|fs| fs.root(&rootfs))
        .console(|c| c.disable_implicit().custom("agent", Box::new(backend)))
        .exec(|e| {
            e.path("/bin/sh").args([
                "-c",
                "echo 'Hello from microVM!' && echo 'VM test successful'",
            ])
        })
        .build()?;

    eprintln!("VM built successfully.");
    eprintln!("Calling enter() — process will be taken over by VMM.");
    eprintln!("On guest exit, the process terminates via _exit().");
    eprintln!();

    // enter() never returns on success
    vm.enter()?;

    unreachable!()
}
