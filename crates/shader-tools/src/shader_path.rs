//! Helpers for splitting a `*.{vs|ps}.{platform}` chunk path into its
//! component parts.
//!
//! Riot stores shader chunks as:
//! - TOC:    `<base>.{vs|ps}.{platform}`           (e.g. `…/foo.vs.metal`)
//! - bundle: `<base>.{vs|ps}.{platform}_<id>`      (e.g. `…/foo.vs.metal_0`)
//!
//! `base` is what we call the "shader object path" — it is the value
//! `ltk_shader::ShaderLoader` expects (it appends `.{type}.{platform}`
//! itself and lowercases).

use crate::args::{PlatformArg, ShaderTypeArg};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShaderObjectPath {
    /// The portion of the chunk path before `.{type}.{platform}` (lowercase).
    pub base: String,
    pub shader_type: ShaderTypeArg,
    pub platform: PlatformArg,
}

impl ShaderObjectPath {
    /// Format back into a chunk path (TOC, no bundle suffix).
    #[allow(dead_code)]
    pub fn to_chunk_path(&self) -> String {
        format!(
            "{}.{}.{}",
            self.base,
            self.shader_type.as_extension(),
            self.platform.as_extension()
        )
    }

    #[allow(dead_code)]
    pub fn bundle_chunk_path(&self, bundle_id: u32) -> String {
        format!("{}_{}", self.to_chunk_path(), bundle_id)
    }
}

/// Try to parse a chunk path as a TOC path (`<base>.{vs|ps}.{platform}`).
///
/// Returns `None` if the path doesn't end with one of the recognized
/// `(type, platform)` pairs, or if there is a bundle-id suffix.
pub fn parse_toc_path(path: &str) -> Option<ShaderObjectPath> {
    let lower = path.to_ascii_lowercase();
    for &shader_type in ShaderTypeArg::all() {
        for &platform in PlatformArg::all() {
            let suffix = format!(
                ".{}.{}",
                shader_type.as_extension(),
                platform.as_extension()
            );
            if let Some(stripped) = lower.strip_suffix(&suffix) {
                return Some(ShaderObjectPath {
                    base: stripped.to_string(),
                    shader_type,
                    platform,
                });
            }
        }
    }
    None
}

/// Try to parse a chunk path as a bundle path
/// (`<base>.{vs|ps}.{platform}_<bundle_id>`).
pub fn parse_bundle_path(path: &str) -> Option<(ShaderObjectPath, u32)> {
    let (head, id_part) = path.rsplit_once('_')?;
    let bundle_id: u32 = id_part.parse().ok()?;
    let object = parse_toc_path(head)?;
    Some((object, bundle_id))
}
