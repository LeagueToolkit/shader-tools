use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use camino::Utf8Path;
use eyre::{eyre, Context, Result};
use fancy_regex::Regex;
use indicatif::ProgressStyle;
use ltk_shader::loader::ShaderLoader;
use ltk_wad::Wad;
use rayon::prelude::*;
use tracing::{debug, info, warn};
use tracing_indicatif::span_ext::IndicatifSpanExt;

use crate::args::{PlatformArg, ShaderTypeArg};
use crate::manifest::{
    BaseDefine, PerPlatformManifest, ShaderEntry, TopManifest, TopManifestCombo, TopManifestObject,
};
use crate::shaders_bin::{self, ShaderEntryMeta};
use crate::utils::load_hashtable;
use crate::wad_index::{ShaderObjectIndex, WadIndex};

pub struct DumpArgs {
    pub input: String,
    pub output: Option<String>,
    pub r#type: Option<Vec<ShaderTypeArg>>,
    pub platform: Option<Vec<PlatformArg>>,
    pub pattern: Option<String>,
    pub shader: Option<Vec<String>>,
    pub metadata: Option<String>,
    pub hashtable: Option<String>,
    pub hashtable_dir: Option<String>,
}

const SHADERS_PER_BUNDLE: u32 = 100;
const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn dump(args: DumpArgs) -> Result<()> {
    // Load the WAD once for the index pass; per-rayon-thread Wads are mounted
    // inside the parallel loop below.
    let wad_file = File::open(&args.input)
        .with_context(|| format!("failed to open input WAD {:?}", args.input))?;
    let main_wad = Wad::mount(wad_file)?;
    let hashtable = load_hashtable(args.hashtable.as_deref(), args.hashtable_dir.as_deref())?;
    let index = WadIndex::build(&main_wad, &hashtable)?;
    drop(main_wad); // close the file; workers each open their own.

    let metadata = match &args.metadata {
        Some(p) => Some(shaders_bin::load_metadata_map(p)?),
        None => None,
    };

    let output = match args.output {
        Some(p) => PathBuf::from(p),
        None => {
            let stem = Utf8Path::new(&args.input)
                .file_stem()
                .unwrap_or("shader-dump");
            PathBuf::from(format!("{stem}_dump"))
        }
    };
    fs::create_dir_all(&output)?;
    info!("output dir: {}", output.display());

    let pattern = compile_pattern(args.pattern.as_deref())?;
    let shader_filter = args
        .shader
        .as_ref()
        .map(|v| v.iter().map(|s| s.to_ascii_lowercase()).collect::<Vec<_>>());
    let want_types: Vec<ShaderTypeArg> = args
        .r#type
        .clone()
        .unwrap_or_else(|| ShaderTypeArg::all().to_vec());
    let want_platforms: Vec<PlatformArg> = args
        .platform
        .clone()
        .unwrap_or_else(|| PlatformArg::all().to_vec());

    // Pre-filter the work list so we don't spin up rayon on objects we'll skip.
    let work_items: Vec<(&String, &ShaderObjectIndex)> = index
        .objects
        .iter()
        .filter(|(base, _)| matches_filters(base, &pattern, shader_filter.as_deref()))
        .collect();

    let span = tracing::info_span!("dumping");
    let _entered = span.enter();
    span.pb_set_style(
        &ProgressStyle::with_template(
            "{msg} {wide_bar:40.cyan/blue} {pos}/{len} ({per_sec}) {elapsed}",
        )
        .unwrap(),
    );
    span.pb_set_length(work_items.len() as u64);

    let total_bytes = AtomicU64::new(0);
    let total_shaders = AtomicU64::new(0);
    let errors_log: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let top_objects_log: Mutex<Vec<TopManifestObject>> = Mutex::new(Vec::new());

    work_items.par_iter().try_for_each_init(
        || {
            File::open(&args.input)
                .map_err(|e| eyre!("worker failed to open input WAD: {e}"))
                .and_then(|f| Wad::mount(f).map_err(|e| eyre!("worker failed to mount WAD: {e}")))
        },
        |wad_result, (base, info)| -> Result<()> {
            let wad = match wad_result {
                Ok(w) => w,
                Err(_e) => return Ok(()), // already logged on init failure
            };
            span.pb_set_message(base);
            span.pb_inc(1);

            let mut combos_emitted: Vec<TopManifestCombo> = Vec::new();
            for &shader_type in &want_types {
                for &platform in &want_platforms {
                    let key = (shader_type, platform);
                    if !info.combinations.contains_key(&key) {
                        debug!("skip {base} {shader_type:?} {platform:?}: no TOC chunk");
                        continue;
                    }

                    let result = dump_combination(
                        &output,
                        base,
                        shader_type,
                        platform,
                        info.bundle_ids
                            .get(&key)
                            .map(|v| v.as_slice())
                            .unwrap_or(&[]),
                        metadata.as_ref(),
                        wad,
                    );

                    match result {
                        Ok(summary) => {
                            total_bytes.fetch_add(summary.total_bytes, Ordering::Relaxed);
                            total_shaders.fetch_add(summary.shader_count as u64, Ordering::Relaxed);
                            combos_emitted.push(TopManifestCombo {
                                shader_type: shader_type.as_extension().to_string(),
                                platform: platform.as_extension().to_string(),
                                shader_count: summary.shader_count,
                                manifest_path: summary.manifest_relative_path,
                            });
                        }
                        Err(e) => {
                            let msg = format!("{base} {shader_type:?} {platform:?}: {e:#}");
                            warn!("{msg}");
                            errors_log.lock().unwrap().push(msg);
                        }
                    }
                }
            }

            if !combos_emitted.is_empty() {
                top_objects_log.lock().unwrap().push(TopManifestObject {
                    base: base.to_string(),
                    combinations: combos_emitted,
                });
            }
            Ok(())
        },
    )?;

    let mut top_objects = top_objects_log.into_inner().unwrap();
    // Stable-sort by base so the top manifest is reproducible regardless of
    // worker scheduling.
    top_objects.sort_by(|a, b| a.base.cmp(&b.base));

    let errors = errors_log.into_inner().unwrap();

    let top_manifest = TopManifest {
        version: 1,
        source_wad: args.input.clone(),
        tool_version: TOOL_VERSION.to_string(),
        objects: top_objects,
    };
    let top_path = output.join("manifest.json");
    let mut f = File::create(&top_path)?;
    serde_json::to_writer_pretty(&mut f, &top_manifest)?;
    f.flush()?;

    info!(
        "dump complete: {} shader objects, {} shaders, {} bytes, {} errors",
        top_manifest.objects.len(),
        total_shaders.load(Ordering::Relaxed),
        total_bytes.load(Ordering::Relaxed),
        errors.len()
    );

    if !errors.is_empty() {
        return Err(eyre!(
            "{} combinations failed during dump (see warnings above)",
            errors.len()
        ));
    }
    Ok(())
}

fn matches_filters(base: &str, pattern: &Option<Regex>, shader_filter: Option<&[String]>) -> bool {
    if let Some(p) = pattern {
        if !p.is_match(base).unwrap_or(false) {
            return false;
        }
    }
    if let Some(filters) = shader_filter {
        let lower = base.to_ascii_lowercase();
        if !filters.iter().any(|f| &lower == f || lower.contains(f)) {
            return false;
        }
    }
    true
}

struct ComboSummary {
    shader_count: usize,
    total_bytes: u64,
    manifest_relative_path: String,
}

fn dump_combination(
    output_root: &Path,
    base: &str,
    shader_type: ShaderTypeArg,
    platform: PlatformArg,
    bundle_ids_present: &[u32],
    metadata: Option<&HashMap<String, ShaderEntryMeta>>,
    wad: &mut Wad<File>,
) -> Result<ComboSummary> {
    let toc = ShaderLoader::load_toc(base, shader_type.into(), platform.into(), wad)
        .with_context(|| format!("loading TOC for {base}"))?;

    let leaf_dir = output_root
        .join(sanitize_path(base))
        .join(shader_type.as_extension())
        .join(platform.as_extension());
    fs::create_dir_all(&leaf_dir)?;

    // Group shader_ids by bundle_id = (shader_id / 100) * 100.
    let mut by_bundle: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new(); // bundle_id -> [(shader_id, idx_in_bundle)]
    for &shader_id in &toc.shader_ids {
        let bundle_id = (shader_id / SHADERS_PER_BUNDLE) * SHADERS_PER_BUNDLE;
        let idx_in_bundle = shader_id % SHADERS_PER_BUNDLE;
        by_bundle
            .entry(bundle_id)
            .or_default()
            .push((shader_id, idx_in_bundle));
    }

    let object_full = format!(
        "{}.{}.{}",
        base,
        shader_type.as_extension(),
        platform.as_extension()
    );

    let mut shaders_meta: Vec<ShaderEntry> = Vec::new();
    let mut total_bytes: u64 = 0;

    for (&bundle_id, shaders_in_bundle) in &by_bundle {
        if !bundle_ids_present.contains(&bundle_id) {
            warn!(
                "{object_full}: TOC references bundle {bundle_id} not present in WAD; skipping {} shaders",
                shaders_in_bundle.len()
            );
            continue;
        }
        let entries = ShaderLoader::read_bundle(&object_full, bundle_id, wad)
            .with_context(|| format!("reading bundle {bundle_id} of {object_full}"))?;

        for &(shader_id, idx_in_bundle) in shaders_in_bundle {
            let bytecode = entries.get(idx_in_bundle as usize).ok_or_else(|| {
                eyre!(
                    "bundle {bundle_id} only has {} entries; need index {idx_in_bundle}",
                    entries.len()
                )
            })?;

            let file_name = format!("{shader_id}.bin");
            let dst = leaf_dir.join(&file_name);
            let mut f = File::create(&dst)?;
            f.write_all(bytecode)?;
            f.flush()?;

            total_bytes += bytecode.len() as u64;

            let defines_hash = toc
                .shader_ids
                .iter()
                .position(|&id| id == shader_id)
                .map(|i| toc.shader_hashes[i])
                .unwrap_or(0);

            shaders_meta.push(ShaderEntry {
                shader_id,
                bundle_id,
                index_in_bundle: idx_in_bundle,
                defines_hash: format!("0x{defines_hash:016x}"),
                byte_count: bytecode.len(),
                file: file_name,
            });
        }
    }

    let metadata_entry = metadata.and_then(|m| shaders_bin::lookup_by_base(m, base).cloned());

    let manifest = PerPlatformManifest {
        version: 1,
        object_path: base.to_string(),
        shader_type: shader_type.as_extension().to_string(),
        platform: platform.as_extension().to_string(),
        base_defines: toc
            .base_defines
            .iter()
            .map(|d| BaseDefine {
                name: d.name.clone(),
                value: d.value.clone(),
                hash: format!("0x{:08x}", d.hash),
            })
            .collect(),
        shaders: shaders_meta,
        metadata: metadata_entry,
    };

    let manifest_path = leaf_dir.join("manifest.json");
    let mut f = File::create(&manifest_path)?;
    serde_json::to_writer_pretty(&mut f, &manifest)?;
    f.flush()?;

    let manifest_relative = manifest_path
        .strip_prefix(output_root)
        .unwrap_or(&manifest_path)
        .to_string_lossy()
        .replace('\\', "/");

    Ok(ComboSummary {
        shader_count: manifest.shaders.len(),
        total_bytes,
        manifest_relative_path: manifest_relative,
    })
}

fn compile_pattern(pattern: Option<&str>) -> Result<Option<Regex>> {
    match pattern {
        Some(p) => {
            let p = if p.contains("(?i)") || p.contains("(?-i)") {
                p.to_string()
            } else {
                format!("(?i){p}")
            };
            Ok(Some(Regex::new(&p)?))
        }
        None => Ok(None),
    }
}

/// Replace `\` with `/` and split into safe path segments for OS join.
fn sanitize_path(p: &str) -> PathBuf {
    let normalized = p.replace('\\', "/");
    let mut out = PathBuf::new();
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            continue;
        }
        out.push(segment);
    }
    out
}
