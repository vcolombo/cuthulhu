// SPDX-License-Identifier: GPL-3.0-or-later
use clap::{Parser, Subcommand};
use cli::cut::format_pass_color;
use cli::pipeline::{check_color_flag_scope, check_interactive, pass_stream_bytes, plan_cut_from_svg, plan_plain_cut, Device};
use driver_registry::HardwareBackendFactory;
use driver_core::manager::{DeviceManager, DeviceState};
use driver_core::{DeviceBackendFactory, DeviceInfo, Settings, Transport, TransportKind};
use std::io::IsTerminal;
use std::sync::Arc;

/// Matches the ceiling the desktop applies before decoding. The CLI is a second entry point into
/// the same tracer, so without it one of the two ways in has no bound at all.
const MAX_INPUT_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Read an image file, refusing anything past the ceiling.
///
/// Everything goes through a single open handle rather than stat-then-read on the pathname: a
/// separate size check describes whatever the pathname pointed at when it ran, not necessarily
/// what gets read afterwards, so a file that grows in between would sail past the limit it was
/// just measured against. `fstat` on the handle cannot drift like that, and refuses an oversized
/// file for a syscall instead of an allocation; `take` is what actually bounds the read.
fn read_image_capped(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    // A failed fstat is not fatal: this is a fast path, and the capped read below is the bound.
    if file.metadata().is_ok_and(|m| m.len() > MAX_INPUT_FILE_BYTES) {
        return Err(format!(
            "file is too large to open: over {} MiB",
            MAX_INPUT_FILE_BYTES / (1024 * 1024)
        ));
    }
    let mut bytes = Vec::new();
    // One byte past the ceiling, so hitting it is distinguishable from landing exactly on it.
    file.take(MAX_INPUT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if bytes.len() as u64 > MAX_INPUT_FILE_BYTES {
        return Err(format!(
            "file is too large to open: over {} MiB",
            MAX_INPUT_FILE_BYTES / (1024 * 1024)
        ));
    }
    Ok(bytes)
}

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
        /// Send --by-color geometry that falls outside the machine's cutting area
        #[arg(long)]
        allow_out_of_bounds: bool,
    },
    /// List known devices
    ListDevices,
    /// Trace a bitmap image (PNG/JPEG/GIF/BMP) into an SVG of cuttable paths
    Trace {
        /// Input image file
        file: std::path::PathBuf,
        /// Output SVG path
        #[arg(short, long)]
        output: std::path::PathBuf,
        /// binary (single-color silhouette) or color (one path per color cluster)
        #[arg(long, default_value = "binary")]
        mode: String,
        /// Ignore speckles up to this size in px (0–16)
        #[arg(long, default_value_t = 4)]
        speckle: u8,
        /// Corner threshold in degrees (0–180); higher = smoother
        #[arg(long, default_value_t = 60)]
        smoothing: u8,
        /// Segment length threshold (3.5–10.0); lower = more detail
        #[arg(long, default_value_t = 4.0)]
        detail: f64,
        /// Color precision in bits (1–8, color mode only)
        #[arg(long, default_value_t = 6)]
        colors: u8,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    match Cli::parse().command {
        Command::Cut { file, device, dry_run, speed, force, port, baud, by_color, skip_color, order, allow_out_of_bounds } => {
            let device = Device::from_id(&device)?;
            check_color_flag_scope(&skip_color, &order, by_color)?;
            let svg = std::fs::read(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
            let settings = Settings { speed, force, repeat_count: 1 };

            if !by_color {
                let plan = plan_plain_cut(&svg, device, &settings, allow_out_of_bounds)?;
                let driver = device.driver();
                let bytes = pass_stream_bytes(driver.as_ref(), &plan.passes[0].job, 0, 1)?;
                if dry_run {
                    print_hex_ascii(&bytes);
                    return Ok(());
                }
                // transmit path lands in Task 7
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

            cut_by_color(&svg, device, &settings, &skip_color, order, dry_run, port, baud, allow_out_of_bounds)
        }
        Command::ListDevices => {
            for d in [Device::Cameo5, Device::Puma] {
                let p = d.driver().profile().clone();
                println!("{}\t{}\t{} x {} mm", p.id, p.name, p.width_mm, p.height_mm);
            }
            Ok(())
        }
        Command::Trace { file, output, mode, speckle, smoothing, detail, colors } => {
            let mode = match mode.as_str() {
                "binary" => trace::TraceMode::Binary,
                "color" => trace::TraceMode::Color,
                other => return Err(format!("--mode must be binary or color, got {other}")),
            };
            let opts = trace::TraceOptions {
                mode,
                filter_speckle: speckle,
                corner_threshold: smoothing,
                length_threshold: detail,
                color_precision: colors,
            };
            let bytes = read_image_capped(&file)?;
            let result = trace::trace(&bytes, &opts).map_err(|e| match e {
                trace::TraceError::EmptyResult =>
                    "nothing traced — lower --speckle or lower --detail".to_string(),
                other => other.to_string(),
            })?;
            std::fs::write(&output, result.svg).map_err(|e| format!("cannot write {}: {e}", output.display()))?;
            println!("{} paths → {}", result.path_count, output.display());
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
    allow_out_of_bounds: bool,
) -> Result<(), String> {
    // Preflight runs here, before the dry-run branch, so a dry run and a real
    // cut always agree on whether the job is acceptable at all.
    let plan = plan_cut_from_svg(svg, device, settings, skip_color, order, allow_out_of_bounds)?;
    let passes = &plan.passes;

    if dry_run {
        let d = device.driver();
        for (i, pass) in passes.iter().enumerate() {
            println!("-- pass {}/{} (color {}) --", i + 1, passes.len(), format_pass_color(pass.color));
            let bytes = pass_stream_bytes(d.as_ref(), &pass.job, i, passes.len())?;
            print_hex_ascii(&bytes);
        }
        return Ok(());
    }

    if let Err(e) = check_interactive(std::io::stdin().is_terminal(), passes.len()) {
        eprintln!("error: {e}");
        std::process::exit(2);
    }

    let info = resolve_device_info(device, port.as_deref(), baud)?;
    let factory: Arc<dyn DeviceBackendFactory> = Arc::new(HardwareBackendFactory);
    let (mgr, _events) = DeviceManager::spawn(factory);
    let mgr = Arc::new(mgr);
    mgr.connect(info).map_err(|e| format!("connect: {e:?}"))?;

    // ponytail: the handler holds a permanent Arc clone for the life of the
    // process, so `mgr` is never uniquely owned again — skip a graceful
    // `shutdown()` and let the (short-lived CLI) process exit reap the worker.
    let ctrlc_mgr = mgr.clone();
    ctrlc::set_handler(move || ctrlc_mgr.cancel()).map_err(|e| format!("ctrlc: {e}"))?;

    mgr.cut(plan.cut_passes()).map_err(|e| format!("cut: {e:?}"))?;

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
        Device::Cameo5 => HardwareBackendFactory
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
