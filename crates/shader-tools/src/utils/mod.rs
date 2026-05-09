pub mod config;
mod hashtable;

use camino::{Utf8Path, Utf8PathBuf};
use eyre::Result;
use std::fs::File;

pub use hashtable::*;

#[allow(dead_code)]
pub fn format_chunk_path_hash(path_hash: u64) -> String {
    format!("{:016x}", path_hash)
}

/// Common helper used by every command that needs a `WadHashtable`. Loads
/// from (in this order):
///
/// 1. `--hashtable-dir` if provided, else [`default_hashtable_dir`] if it exists.
/// 2. `--hashtable <file>` if provided (additive).
pub fn load_hashtable(
    hashtable: Option<&str>,
    hashtable_dir: Option<&str>,
) -> Result<WadHashtable> {
    let mut table = WadHashtable::new()?;
    if let Some(dir_override) = hashtable_dir {
        table.add_from_dir(Utf8Path::new(dir_override))?;
    } else if let Some(dir) = default_hashtable_dir() {
        if dir.exists() {
            table.add_from_dir(dir)?;
        }
    }
    if let Some(file_path) = hashtable {
        tracing::info!("loading hashtable from {}", file_path);
        table.add_from_file(&File::open(file_path)?)?;
    }
    Ok(table)
}

/// Returns the default directory where wad hashtables should be looked up.
/// On Windows, prefers the user's Documents folder: Documents/LeagueToolkit/wad_hashtables
/// On other platforms, uses platform-appropriate data directory via directories_next.
///
/// Note: shares the same directory as `wadtools` so installing one set of
/// hashtables works for both tools.
pub fn default_hashtable_dir() -> Option<Utf8PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(mut doc_dir) = dirs_next::document_dir() {
            doc_dir.push("LeagueToolkit");
            doc_dir.push("wad_hashtables");
            return Utf8PathBuf::from_path_buf(doc_dir).ok();
        }
    }

    if let Some(proj) = directories_next::ProjectDirs::from("io", "LeagueToolkit", "shader-tools") {
        let mut path = proj.data_dir().to_path_buf();
        path.push("wad_hashtables");
        return Utf8PathBuf::from_path_buf(path).ok();
    }

    None
}
