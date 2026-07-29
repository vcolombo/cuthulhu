// SPDX-License-Identifier: GPL-3.0-or-later
use clap::{Parser, Subcommand};
use cli::cut::{self, format_pass_color};
use cli::pipeline::{
    check_color_flag_scope, check_interactive, driver_for, dry_run_pass_bytes, plan_cut_from_svg, plan_plain_cut,
    resolve_device_info,
};
use driver_registry::{machine_ids, HardwareBackendFactory};
use driver_core::{DeviceBackendFactory, Driver, Settings};
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
        #[arg(long, help = trace::SPECKLE.help, default_value_t = trace::SPECKLE.default as u8)]
        speckle: u8,
        #[arg(long, help = trace::SMOOTHING.help, default_value_t = trace::SMOOTHING.default as u8)]
        smoothing: u8,
        #[arg(long, help = trace::DETAIL.help, default_value_t = trace::DETAIL.default)]
        detail: f64,
        #[arg(long, help = trace::COLORS.help, default_value_t = trace::COLORS.default as u8)]
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
fn drive_cut(plan: &cutplan::CutPlan, machine_id: &str, port: Option<&str>, baud: u32) -> Result<(), String> {
    let info = resolve_device_info(machine_id, port, baud)?;
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
            let driver = driver_for(&device)?;
            check_color_flag_scope(&skip_color, &order, by_color)?;
            let svg = std::fs::read(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
            let settings = Settings { speed, force, repeat_count: 1 };

            if !by_color {
                let plan = plan_plain_cut(&svg, driver.as_ref(), &settings, allow_out_of_bounds)?;
                if dry_run {
                    let bytes = dry_run_pass_bytes(driver.as_ref(), &plan.passes[0].job, 0, 1)?;
                    print_hex_ascii(&bytes);
                    return Ok(());
                }
                return drive_cut(&plan, &device, port.as_deref(), baud);
            }

            cut_by_color(&svg, driver.as_ref(), &device, &settings, &skip_color, order, dry_run, port, baud, allow_out_of_bounds)
        }
        Command::ListDevices => {
            for id in machine_ids() {
                let p = driver_for(id)?.profile().clone();
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
            let controls = trace::TraceControls { mode, speckle, smoothing, detail, colors };
            let bytes = trace::read_image(&file).map_err(|e| e.to_string())?;
            let result = trace::trace(&bytes, &controls).map_err(|e| e.to_string())?;
            std::fs::write(&output, result.svg)
                .map_err(|e| format!("cannot write {}: {e}", output.display()))?;
            println!("{} paths → {}", result.path_count, output.display());
            if result.downscaled {
                println!("large image reduced to {} px for tracing", trace::MAX_DIM);
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cut_by_color(
    svg: &[u8],
    driver: &dyn Driver,
    machine_id: &str,
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
    let plan = plan_cut_from_svg(svg, driver, settings, skip_color, order, allow_out_of_bounds)?;
    let passes = &plan.passes;

    if dry_run {
        for (i, pass) in passes.iter().enumerate() {
            println!("-- pass {}/{} (color {}) --", i + 1, passes.len(), format_pass_color(pass.color));
            let bytes = dry_run_pass_bytes(driver, &pass.job, i, passes.len())?;
            print_hex_ascii(&bytes);
        }
        return Ok(());
    }

    if let Err(e) = check_interactive(std::io::stdin().is_terminal(), passes.len()) {
        eprintln!("error: {e}");
        std::process::exit(2);
    }

    drive_cut(&plan, machine_id, port.as_deref(), baud)
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
