use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::Instant,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub version: u32,
    pub revision: String,
    pub dirty: bool,
    pub source_tree_sha256: String,
    pub binary_sha256: String,
    pub profile: String,
    pub rustc: String,
}

pub fn nanos(start: Instant) -> u64 {
    start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "cannot open benchmark input")?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read benchmark input")?;
    if bytes.len() as u64 > limit {
        return Err("benchmark input exceeds its bound".into());
    }
    Ok(bytes)
}
pub fn file_digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "cannot open benchmark executable")?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut total = 0u64;
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|_| "cannot hash benchmark executable")?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > 536_870_912 {
            return Err("benchmark executable exceeds its hash bound".into());
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|_| "cannot create benchmark artifact".into())
}
pub fn write_json(path: &Path, value: &Json) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "cannot encode benchmark artifact")?;
    if bytes.len() > 16_777_216 {
        return Err("benchmark report exceeds its bound".into());
    }
    write_new(path, &bytes)
}
pub fn proc_stat() -> String {
    fs::read_to_string("/proc/self/stat").unwrap_or_default()
}
