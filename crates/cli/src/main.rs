// SPDX-License-Identifier: GPL-3.0-or-later
use clap::{Parser, Subcommand};
use cli::cut::{self, format_pass_color};
use cli::pipeline::{check_color_flag_scope, check_interactive, pass_stream_bytes, plan_cut_from_svg, plan_plain_cut, Device};
use driver_registry::HardwareBackendFactory;
use driver_core::{DeviceBackendFactory, DeviceInfo, Settings, TransportKind};
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
        /// Send geometry that falls outside the machine's cutting area
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

/// A terminal means a human can answer the machine's pauses; anything else
/// (a script, a CI job) must not be left blocking on stdin.
fn operator() -> cut::Operator {
    if std::io::stdin().is_terminal() { cut::Operator::Interactive } else { cut::Operator::Unattended }
}

/// Drive a planned cut on real hardware and report how it ended.
///
/// Ctrl-C is installed here rather than inside `cut::run`: a process-wide signal
/// handler belongs to the binary, and `set_handler` errors on a second call, so a
/// library function that installs one can only ever be called once per process.
fn drive_cut(plan: &cutplan::CutPlan, device: Device, port: Option<&str>, baud: u32) -> Result<(), String> {
    let info = resolve_device_info(device, port, baud)?;
    let factory: Arc<dyn DeviceBackendFactory> = Arc::new(HardwareBackendFactory);
    let outcome = cut::run(plan, info, factory, operator(), |mgr| {
        // ponytail: the handler holds a permanent Arc clone for the life of the
        // process, so the manager is never uniquely owned again — skip a graceful
        // `shutdown()` and let the (short-lived CLI) process exit reap the worker.
        ctrlc::set_handler(move || mgr.cancel()).map_err(|e| format!("ctrlc: {e}"))
    })?;
    println!("{}", cut::ended_message(&outcome));
    Ok(())
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
                if dry_run {
                    let driver = device.driver();
                    let bytes = pass_stream_bytes(driver.as_ref(), &plan.passes[0].job, 0, 1)?;
                    print_hex_ascii(&bytes);
                    return Ok(());
                }
                return drive_cut(&plan, device, port.as_deref(), baud);
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

    drive_cut(&plan, device, port.as_deref(), baud)
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

fn print_hex_ascii(bytes: &[u8]) {
    for chunk in bytes.chunks(16) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk.iter()
            .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' })
            .collect();
        println!("{:<48} {ascii}", hex.join(" "));
    }
}
