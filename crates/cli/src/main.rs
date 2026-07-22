// SPDX-License-Identifier: GPL-3.0-or-later
use clap::{Parser, Subcommand};
use cli::pipeline::{build_bytes, Device};
use driver_core::{Settings, Transport};

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
        Command::Cut { file, device, dry_run, speed, force, port, baud } => {
            let device = Device::from_id(&device)?;
            let svg = std::fs::read(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
            let settings = Settings { speed, force, passes: 1 };
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
            transport.write(&bytes).map_err(|e| format!("write: {e:?}"))?;
            println!("sent {} bytes", bytes.len());
            Ok(())
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

fn print_hex_ascii(bytes: &[u8]) {
    for chunk in bytes.chunks(16) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk.iter()
            .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' })
            .collect();
        println!("{:<48} {ascii}", hex.join(" "));
    }
}
