//! Parser for the `data/shaders/shaders.bin` metadata file inside
//! `Shaders.wad.client`.
//!
//! The bin contains one or more top-level objects (`#PROP_text`) whose
//! `entries: map[hash, embed]` property holds a `CustomShaderDef` per
//! shader. We collect all of them into a flat [`ShaderCatalog`].

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek};
use std::sync::OnceLock;

use eyre::{eyre, Context, Result};
use ltk_hash::fnv1a;
use ltk_meta::property::values;
use ltk_meta::{Bin, PropertyValueEnum};
use ltk_wad::Wad;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ShaderCatalog {
    pub entries: Vec<ShaderEntryMeta>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ShaderEntryMeta {
    pub object_path: String,
    pub parameter_names: Vec<String>,
    pub static_switches: Vec<StaticSwitch>,
    pub textures: Vec<TextureRef>,
    pub feature_defines: Vec<(String, String)>,
    pub feature_mask: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StaticSwitch {
    pub name: String,
    pub on_by_default: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TextureRef {
    pub name: String,
    pub default_path: Option<String>,
}

/// Cached FNV-1a hashes for the property/class names we look up.
struct Hashes {
    entries: u32,
    object_path: u32,
    parameters: u32,
    static_switches: u32,
    textures: u32,
    feature_defines: u32,
    feature_mask: u32,
    name: u32,
    on_by_default: u32,
    default_texture_path: u32,
}

fn hashes() -> &'static Hashes {
    static H: OnceLock<Hashes> = OnceLock::new();
    H.get_or_init(|| Hashes {
        entries: fnv1a::hash_lower("entries"),
        object_path: fnv1a::hash_lower("ObjectPath"),
        parameters: fnv1a::hash_lower("Parameters"),
        static_switches: fnv1a::hash_lower("StaticSwitches"),
        textures: fnv1a::hash_lower("Textures"),
        feature_defines: fnv1a::hash_lower("FeatureDefines"),
        feature_mask: fnv1a::hash_lower("FeatureMask"),
        name: fnv1a::hash_lower("Name"),
        on_by_default: fnv1a::hash_lower("OnByDefault"),
        default_texture_path: fnv1a::hash_lower("DefaultTexturePath"),
    })
}

pub fn read_catalog<R: Read + Seek>(reader: &mut R) -> Result<ShaderCatalog> {
    let bin = Bin::from_reader(reader).wrap_err("failed to parse shaders.bin")?;
    let h = hashes();

    let mut entries = Vec::new();
    for (_path_hash, object) in &bin.objects {
        let Some(entries_prop) = object.get_property(h.entries) else {
            continue;
        };
        let PropertyValueEnum::Map(map) = entries_prop else {
            continue;
        };

        for (_key, value) in map.entries() {
            let PropertyValueEnum::Embedded(embed) = value else {
                continue;
            };
            // embed.0 is the inner Struct (CustomShaderDef)
            entries.push(read_entry(&embed.0)?);
        }
    }

    Ok(ShaderCatalog { entries })
}

fn read_entry(s: &values::Struct) -> Result<ShaderEntryMeta> {
    let h = hashes();

    let object_path = string_prop(s, h.object_path)?
        .ok_or_else(|| eyre!("CustomShaderDef missing ObjectPath"))?;

    let parameter_names = match s.properties.get(&h.parameters) {
        Some(PropertyValueEnum::Container(c)) => collect_named_embeds(c)?,
        _ => Vec::new(),
    };

    let static_switches = match s.properties.get(&h.static_switches) {
        Some(PropertyValueEnum::Container(c)) => read_static_switches(c)?,
        _ => Vec::new(),
    };

    let textures = match s.properties.get(&h.textures) {
        Some(PropertyValueEnum::Container(c)) => read_textures(c)?,
        _ => Vec::new(),
    };

    let feature_defines = match s.properties.get(&h.feature_defines) {
        Some(PropertyValueEnum::Map(map)) => read_string_map(map),
        _ => Vec::new(),
    };

    let feature_mask = match s.properties.get(&h.feature_mask) {
        Some(PropertyValueEnum::U32(v)) => v.value,
        _ => 0,
    };

    Ok(ShaderEntryMeta {
        object_path,
        parameter_names,
        static_switches,
        textures,
        feature_defines,
        feature_mask,
    })
}

fn collect_named_embeds(c: &values::Container) -> Result<Vec<String>> {
    let h = hashes();
    let mut names = Vec::new();
    for item in c.clone().into_items() {
        let PropertyValueEnum::Embedded(embed) = item else {
            continue;
        };
        if let Some(name) = string_prop(&embed.0, h.name)? {
            names.push(name);
        }
    }
    Ok(names)
}

fn read_static_switches(c: &values::Container) -> Result<Vec<StaticSwitch>> {
    let h = hashes();
    let mut switches = Vec::new();
    for item in c.clone().into_items() {
        let PropertyValueEnum::Embedded(embed) = item else {
            continue;
        };
        let Some(name) = string_prop(&embed.0, h.name)? else {
            continue;
        };
        let on_by_default = matches!(
            embed.0.properties.get(&h.on_by_default),
            Some(PropertyValueEnum::Bool(b)) if b.value
        );
        switches.push(StaticSwitch {
            name,
            on_by_default,
        });
    }
    Ok(switches)
}

fn read_textures(c: &values::Container) -> Result<Vec<TextureRef>> {
    let h = hashes();
    let mut textures = Vec::new();
    for item in c.clone().into_items() {
        let PropertyValueEnum::Embedded(embed) = item else {
            continue;
        };
        let Some(name) = string_prop(&embed.0, h.name)? else {
            continue;
        };
        let default_path = string_prop(&embed.0, h.default_texture_path)?;
        textures.push(TextureRef { name, default_path });
    }
    Ok(textures)
}

fn read_string_map(map: &values::Map) -> Vec<(String, String)> {
    map.entries()
        .iter()
        .filter_map(|(k, v)| {
            let key = match k {
                PropertyValueEnum::String(s) => s.value.clone(),
                _ => return None,
            };
            let val = match v {
                PropertyValueEnum::String(s) => s.value.clone(),
                _ => return None,
            };
            Some((key, val))
        })
        .collect()
}

fn string_prop(s: &values::Struct, name_hash: u32) -> Result<Option<String>> {
    Ok(match s.properties.get(&name_hash) {
        Some(PropertyValueEnum::String(v)) => Some(v.value.clone()),
        Some(_) | None => None,
    })
}

/// Build a lowercased-`ObjectPath` → metadata map from either a raw
/// `shaders.bin` file or a WAD that contains it as a chunk.
pub fn load_metadata_map(path: &str) -> Result<HashMap<String, ShaderEntryMeta>> {
    let bytes = read_shaders_bin_bytes(path)?;
    let catalog = read_catalog(&mut Cursor::new(bytes))?;
    Ok(catalog
        .entries
        .into_iter()
        .map(|e| (e.object_path.to_ascii_lowercase(), e))
        .collect())
}

/// Look up a metadata entry by the bundle-WAD chunk-path-style `base`. Tries
/// the lowercased base directly, then strips the
/// `assets/shaders/generated/` prefix (which the bundle WAD adds and
/// `shaders.bin`'s `ObjectPath` doesn't).
pub fn lookup_by_base<'a>(
    metadata: &'a HashMap<String, ShaderEntryMeta>,
    base: &str,
) -> Option<&'a ShaderEntryMeta> {
    let lower = base.to_ascii_lowercase();
    if let Some(m) = metadata.get(&lower) {
        return Some(m);
    }
    let stripped = lower
        .strip_prefix("assets/shaders/generated/")
        .unwrap_or(&lower);
    metadata.get(stripped)
}

fn read_shaders_bin_bytes(path: &str) -> Result<Vec<u8>> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read metadata source {path:?}"))?;
    // League WAD magic = 0x5752 ('R','W'). Everything else is treated as a raw
    // .bin payload.
    if bytes.len() >= 2 && &bytes[..2] == b"RW" {
        let cursor = Cursor::new(&bytes);
        let mut wad = Wad::mount(cursor)?;
        let path_hash = xxhash_rust::xxh64::xxh64(b"data/shaders/shaders.bin", 0);
        let chunk = *wad
            .chunks()
            .get(path_hash)
            .ok_or_else(|| eyre!("metadata WAD does not contain `data/shaders/shaders.bin`"))?;
        Ok(wad.load_chunk_decompressed(&chunk)?.into_vec())
    } else {
        Ok(bytes)
    }
}
