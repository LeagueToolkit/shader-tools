//! Walks a mounted bundle WAD and groups its chunks into a usable index.
//!
//! Output is keyed by `base` (the portion of the chunk path before
//! `.{type}.{platform}`), so a single shader object with multiple type/platform
//! combinations rolls up into one entry.

use std::collections::BTreeMap;
use std::fs::File;

use eyre::{eyre, Result};
use ltk_wad::Wad;

use crate::args::{PlatformArg, ShaderTypeArg};
use crate::shader_path::{parse_bundle_path, parse_toc_path, ShaderObjectPath};
use crate::utils::WadHashtable;

#[derive(Debug, Clone, Default)]
pub struct ShaderObjectIndex {
    /// Combinations available for this object (TOC chunks present).
    /// Keyed by `(type, platform)`; value is the WAD-relative chunk path.
    pub combinations: BTreeMap<(ShaderTypeArg, PlatformArg), String>,
    /// Bundle ids present for each combination.
    pub bundle_ids: BTreeMap<(ShaderTypeArg, PlatformArg), Vec<u32>>,
}

#[derive(Debug, Default)]
pub struct WadIndex {
    /// Keyed by lowercased `base` (e.g. `assets/shaders/generated/shaders/particles/vfx_gloom_ground`).
    pub objects: BTreeMap<String, ShaderObjectIndex>,
    /// Total chunk count in the WAD (for stats).
    pub total_chunks: usize,
    /// Number of chunks that resolved to a name through the hashtable.
    pub resolved_chunks: usize,
}

impl WadIndex {
    pub fn build(wad: &Wad<File>, hashtable: &WadHashtable) -> Result<Self> {
        let mut idx = WadIndex {
            total_chunks: wad.chunks().len(),
            ..Default::default()
        };

        for chunk in wad.chunks().iter() {
            let resolved = hashtable.resolve_path(chunk.path_hash);
            let path: &str = resolved.as_ref();
            // Hashtable falls back to a 16-char hex string for unknown hashes;
            // those can never match a TOC/bundle pattern, so skip cheaply.
            if path.len() == 16 && path.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            idx.resolved_chunks += 1;

            if let Some((object, bundle_id)) = parse_bundle_path(path) {
                let entry = idx.objects.entry(object.base.clone()).or_default();
                entry
                    .bundle_ids
                    .entry((object.shader_type, object.platform))
                    .or_default()
                    .push(bundle_id);
                continue;
            }

            if let Some(object) = parse_toc_path(path) {
                let entry = idx.objects.entry(object.base.clone()).or_default();
                entry
                    .combinations
                    .insert((object.shader_type, object.platform), path.to_string());
            }
        }

        // Sort bundle id lists for deterministic iteration.
        for entry in idx.objects.values_mut() {
            for ids in entry.bundle_ids.values_mut() {
                ids.sort_unstable();
                ids.dedup();
            }
        }

        Ok(idx)
    }

    /// Look up a shader object by either an exact `base` or a case-insensitive
    /// substring.
    pub fn find(&self, query: &str) -> Result<(&str, &ShaderObjectIndex)> {
        let q = query.to_ascii_lowercase();
        if let Some((k, v)) = self.objects.get_key_value(&q) {
            return Ok((k.as_str(), v));
        }
        let matches: Vec<_> = self
            .objects
            .iter()
            .filter(|(k, _)| k.contains(&q))
            .collect();
        match matches.len() {
            0 => Err(eyre!("no shader object matches {query:?}")),
            1 => Ok((matches[0].0.as_str(), matches[0].1)),
            _ => Err(eyre!(
                "{} shader objects match {query:?}; refine the query (e.g. {:?})",
                matches.len(),
                matches[0].0
            )),
        }
    }
}

/// Resolve a `(base, type, platform)` triple into a [`ShaderObjectPath`] —
/// without checking the WAD. Useful when the caller already knows what to
/// look up.
pub fn make_shader_object_path(
    base: &str,
    shader_type: ShaderTypeArg,
    platform: PlatformArg,
) -> ShaderObjectPath {
    ShaderObjectPath {
        base: base.to_string(),
        shader_type,
        platform,
    }
}
