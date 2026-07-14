use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

use crate::logging::timestamp_utc;

#[derive(Debug, Serialize)]
struct Manifest {
    created_at: String,
    files: BTreeMap<String, u64>,
}

pub fn create_support_bundle(
    data_dir: &Path,
    doctor_json: &str,
    redacted_config: &str,
) -> Result<PathBuf, String> {
    let output = data_dir.join(format!("deskpilot-support-{}.zip", safe_timestamp()));
    let file = File::create(&output).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut manifest = Manifest { created_at: timestamp_utc(), files: BTreeMap::new() };

    add_bytes(&mut zip, options, "doctor.json", doctor_json.as_bytes(), &mut manifest)?;
    add_bytes(&mut zip, options, "deskpilot.redacted.toml", redacted_config.as_bytes(), &mut manifest)?;
    add_bytes(
        &mut zip,
        options,
        "version.txt",
        format!("DeskPilot {}\n", crate::APP_VERSION).as_bytes(),
        &mut manifest,
    )?;

    let logs_dir = data_dir.join("logs");
    if let Ok(entries) = fs::read_dir(&logs_dir) {
        for entry in entries.flatten().take(10) {
            let path = entry.path();
            if path.is_symlink() || !path.is_file() { continue; }
            let mut data = Vec::new();
            if File::open(&path).and_then(|mut file| file.take(2 * 1024 * 1024).read_to_end(&mut data)).is_ok() {
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    add_bytes(&mut zip, options, &format!("logs/{name}"), &data, &mut manifest)?;
                }
            }
        }
    }

    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    add_bytes(&mut zip, options, "manifest.json", &manifest_json, &mut Manifest {
        created_at: String::new(),
        files: BTreeMap::new(),
    })?;
    zip.finish().map_err(|error| error.to_string())?;
    Ok(output)
}

fn add_bytes(
    zip: &mut zip::ZipWriter<File>,
    options: SimpleFileOptions,
    name: &str,
    data: &[u8],
    manifest: &mut Manifest,
) -> Result<(), String> {
    if name.contains("..") || name.starts_with(['/', '\\']) {
        return Err("unsafe support bundle path".to_string());
    }
    zip.start_file(name.replace('\\', "/"), options).map_err(|error| error.to_string())?;
    zip.write_all(data).map_err(|error| error.to_string())?;
    manifest.files.insert(name.to_string(), data.len() as u64);
    Ok(())
}

fn safe_timestamp() -> String {
    timestamp_utc().replace([':', '.'], "-")
}
