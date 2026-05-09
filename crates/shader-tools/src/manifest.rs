//! Output JSON manifest types for the `dump` command.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct PerPlatformManifest {
    pub version: u32,
    pub object_path: String,
    pub shader_type: String,
    pub platform: String,
    pub base_defines: Vec<BaseDefine>,
    pub shaders: Vec<ShaderEntry>,
    /// Populated when `--metadata` is provided and the shaders.bin
    /// `ObjectPath` matches `object_path` (case-insensitive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<crate::shaders_bin::ShaderEntryMeta>,
}

#[derive(Debug, Serialize)]
pub struct BaseDefine {
    pub name: String,
    pub value: String,
    pub hash: String, // "0x........"
}

#[derive(Debug, Serialize)]
pub struct ShaderEntry {
    pub shader_id: u32,
    pub bundle_id: u32,
    pub index_in_bundle: u32,
    pub defines_hash: String, // "0x................"
    pub byte_count: usize,
    pub file: String,
}

#[derive(Debug, Serialize)]
pub struct TopManifest {
    pub version: u32,
    pub source_wad: String,
    pub tool_version: String,
    pub objects: Vec<TopManifestObject>,
}

#[derive(Debug, Serialize)]
pub struct TopManifestObject {
    pub base: String,
    pub combinations: Vec<TopManifestCombo>,
}

#[derive(Debug, Serialize)]
pub struct TopManifestCombo {
    pub shader_type: String,
    pub platform: String,
    pub shader_count: usize,
    pub manifest_path: String,
}
