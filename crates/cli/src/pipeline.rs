// SPDX-License-Identifier: GPL-3.0-or-later
use driver_core::{Driver, Job, Settings};
use driver_hpgl::HpglDriver;
use driver_silhouette::SilhouetteDriver;

#[derive(Clone, Copy)]
pub enum Device { Cameo5, Puma }
impl Device {
    pub fn from_id(s: &str) -> Result<Device, String> {
        match s {
            "cameo5" => Ok(Device::Cameo5),
            "puma" => Ok(Device::Puma),
            _ => Err(format!("unknown device '{s}' (try: cameo5, puma)")),
        }
    }
    pub fn driver(&self) -> Box<dyn Driver> {
        match self {
            Device::Cameo5 => Box::new(SilhouetteDriver::new()),
            Device::Puma => Box::new(HpglDriver::new()),
        }
    }
}

pub fn build_bytes(svg: &[u8], device: Device, settings: &Settings) -> Result<Vec<u8>, String> {
    let imp = fileio::svg_to_paths(svg).map_err(|e| format!("SVG parse: {e:?}"))?;
    let polylines = imp.paths.iter()
        .flat_map(|(path, _)| path.flatten(0.1))
        .collect::<Vec<_>>();
    if polylines.is_empty() { return Err("no cuttable paths in SVG".into()); }
    let job = Job { polylines, settings: settings.clone() };
    device.driver().encode(&job).map_err(|e| format!("encode: {e:?}"))
}
