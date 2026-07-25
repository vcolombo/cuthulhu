// SPDX-License-Identifier: GPL-3.0-or-later
use clap::{Parser, Subcommand};
use cli::pipeline::{build_bytes, check_interactive, cutpass_from_color_pass, format_pass_color, pass_stream_bytes, plan_from_svg, CliBackendFactory, Device};
use driver_core::manager::{CutPass, DeviceManager, DeviceState};
use driver_core::{DeviceBackendFactory, DeviceInfo, Settings, Transport, TransportKind};
use std::io::IsTerminal;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "cuthulhu", about = "SVG → cutter byte streams")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Cut an SVG file on a device (or --dry-run to print the byte stream)
    Cut {
        /// SVG file to cut
        file: std::path::PathBuf,
        /// Device id (cameo5, puma)
        #[arg(long)]
        device: String,
        /// Print the encoded bytes instead of sending them
        #[arg(long)]
        dry_run: bool,
        /// Cut speed (device units; omit for machine default)
        #[arg(long)]
        speed: Option<u32>,
        /// Cut force (device units; omit for machine default)
        #[arg(long)]
        force: Option<u32>,
        /// Serial port (HPGL devices)
        #[arg(long)]
        port: Option<String>,
        /// Serial baud rate
        #[arg(long, default_value_t = 9600)]
        baud: u32,
        /// Cut each stroke color as a separate pass, pausing between passes for a tool swap
        #[arg(long)]
        by_color: bool,
        /// Skip cutting shapes with this stroke color (RRGGBBAA); may be repeated
        #[arg(long = "skip-color")]
        skip_color: Vec<String>,
        /// Comma-separated color order (RRGGBBAA,...) for --by-color passes
        #[arg(long)]
        order: Option<String>,
    },
    /// List known devices
    ListDevices,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    match Cli::parse().command {
        Command::Cut { file, device, dry_run, speed, force, port, baud, by_color, skip_color, order } => {
            let device = Device::from_id(&device)?;
            let svg = std::fs::read(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
            let settings = Settings { speed, force, repeat_count: 1 };

            if !by_color {
                let bytes = build_bytes(&svg, device, &settings)?;
                if dry_run {
                    print_hex_ascii(&bytes);
                    return Ok(());
                }
                let mut transport: Box<dyn Transport> = match device {
                    Device::Cameo5 => Box::new(
                        driver_silhouette::UsbTransport::open()
                            .map_err(|e| format!("open USB: {e:?}"))?,
                    ),
                    Device::Puma => {
                        let port = port.ok_or("--port required for serial devices")?;
                        Box::new(
                            driver_hpgl::SerialTransport::open(&port, baud)
                                .map_err(|e| format!("open {port}: {e:?}"))?,
                        )
                    }
                };
                // write_all, not a single write(): a partial write would silently
                // truncate the job while still reporting the full byte count.
                driver_core::write_all(transport.as_mut(), &bytes).map_err(|e| format!("write: {e:?}"))?;
                println!("sent {} bytes", bytes.len());
                return Ok(());
            }

            cut_by_color(&svg, device, &settings, &skip_color, order, dry_run, port, baud)
        }
        Command::ListDevices => {
            for d in [Device::Cameo5, Device::Puma] {
                let p = d.driver().profile().clone();
                println!("{}\t{}\t{} x {} mm", p.id, p.name, p.width_mm, p.height_mm);
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cut_by_color(
    svg: &[u8],
    device: Device,
    settings: &Settings,
    skip_color: &[String],
    order: Option<String>,
    dry_run: bool,
    port: Option<String>,
    baud: u32,
) -> Result<(), String> {
    let passes = plan_from_svg(svg, skip_color, order)?;
    if passes.is_empty() {
        return Err("no cuttable paths in SVG".into());
    }

    if dry_run {
        let d = device.driver();
        for (i, pass) in passes.iter().enumerate() {
            println!("-- pass {}/{} (color {}) --", i + 1, passes.len(), format_pass_color(pass.color));
            let cutpass = cutpass_from_color_pass(pass, settings);
            let bytes = pass_stream_bytes(d.as_ref(), &cutpass.job, i, passes.len())?;
            print_hex_ascii(&bytes);
        }
        return Ok(());
    }

    if let Err(e) = check_interactive(std::io::stdin().is_terminal(), passes.len()) {
        eprintln!("error: {e}");
        std::process::exit(2);
    }

    let info = resolve_device_info(device, port.as_deref(), baud)?;
    let factory: Arc<dyn DeviceBackendFactory> = Arc::new(CliBackendFactory);
    let (mgr, _events) = DeviceManager::spawn(factory);
    let mgr = Arc::new(mgr);
    mgr.connect(info).map_err(|e| format!("connect: {e:?}"))?;

    // ponytail: the handler holds a permanent Arc clone for the life of the
    // process, so `mgr` is never uniquely owned again — skip a graceful
    // `shutdown()` and let the (short-lived CLI) process exit reap the worker.
    let ctrlc_mgr = mgr.clone();
    ctrlc::set_handler(move || ctrlc_mgr.cancel()).map_err(|e| format!("ctrlc: {e}"))?;

    let cutpasses: Vec<CutPass> = passes.iter().map(|p| cutpass_from_color_pass(p, settings)).collect();
    mgr.cut(cutpasses).map_err(|e| format!("cut: {e:?}"))?;

    loop {
        match mgr.snapshot() {
            DeviceState::WaitingForColorSwap { next_pass_index, .. } => {
                println!(
                    "Pass {}/{} (color {}): swap tool, press Enter to resume",
                    next_pass_index + 1,
                    passes.len(),
                    format_pass_color(passes[next_pass_index].color),
                );
                if !wait_for_enter_or_cancel(&mgr) {
                    continue; // re-check snapshot: cancel() already landed
                }
                mgr.resume().map_err(|e| format!("resume: {e:?}"))?;
            }
            DeviceState::AwaitingCompletion { pass_index, .. } => {
                println!(
                    "Pass {}/{} (color {}) cutting; press Enter once the machine finishes",
                    pass_index + 1,
                    passes.len(),
                    format_pass_color(passes[pass_index].color),
                );
                if !wait_for_enter_or_cancel(&mgr) {
                    continue;
                }
                mgr.confirm_pass_done().map_err(|e| format!("confirm: {e:?}"))?;
            }
            DeviceState::Idle => {
                println!("done: {} passes cut", passes.len());
                return Ok(());
            }
            DeviceState::Cancelled { pass_index, submitted_bytes, .. } => {
                println!("cancelled at pass {pass_index} ({submitted_bytes} bytes sent)");
                return Ok(());
            }
            DeviceState::Error(e) => return Err(format!("device error: {e:?}")),
            _ => return Err("unexpected device state".into()),
        }
    }
}

fn resolve_device_info(device: Device, port: Option<&str>, baud: u32) -> Result<DeviceInfo, String> {
    match device {
        Device::Cameo5 => CliBackendFactory
            .list_devices()
            .into_iter()
            .find(|d| d.machine_id == "cameo5")
            .ok_or_else(|| "no cameo5 device found".to_string()),
        Device::Puma => {
            let path = port.ok_or("--port required for serial devices")?.to_string();
            Ok(DeviceInfo {
                instance_id: format!("serial:{path}"),
                machine_id: "puma".into(),
                transport: TransportKind::Serial { path, baud },
                candidate: true,
            })
        }
    }
}

/// Block until the operator presses Enter (`true`) or a cancel lands via
/// Ctrl-C/`DeviceManager::cancel` (`false`). The reader thread is left
/// parked on stdin if cancel wins — fine for a process that's about to exit.
fn wait_for_enter_or_cancel(mgr: &DeviceManager) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = std::io::stdin().read_line(&mut buf);
        let _ = tx.send(());
    });
    loop {
        if rx.try_recv().is_ok() {
            return true;
        }
        if matches!(mgr.snapshot(), DeviceState::Cancelled { .. }) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn print_hex_ascii(bytes: &[u8]) {
    for chunk in bytes.chunks(16) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk.iter()
            .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' })
            .collect();
        println!("{:<48} {ascii}", hex.join(" "));
    }
}
