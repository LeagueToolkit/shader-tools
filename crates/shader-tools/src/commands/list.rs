use std::fs::File;

use clap::ValueEnum;
use colored::Colorize;
use eyre::Result;
use fancy_regex::Regex;
use ltk_wad::Wad;

use crate::args::{PlatformArg, ShaderTypeArg};
use crate::shaders_bin::{self, ShaderEntryMeta};
use crate::utils::load_hashtable;
use crate::wad_index::WadIndex;

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ListOutputFormat {
    Table,
    Json,
    Csv,
    Flat,
}

pub struct ListArgs {
    pub input: String,
    pub metadata: Option<String>,
    pub pattern: Option<String>,
    pub format: ListOutputFormat,
    pub show_stats: bool,
    pub hashtable: Option<String>,
    pub hashtable_dir: Option<String>,
}

pub fn list(args: ListArgs) -> Result<()> {
    let wad_file = File::open(&args.input)?;
    let wad = Wad::mount(wad_file)?;
    let hashtable = load_hashtable(args.hashtable.as_deref(), args.hashtable_dir.as_deref())?;

    let index = WadIndex::build(&wad, &hashtable)?;

    let metadata = match &args.metadata {
        Some(path) => Some(shaders_bin::load_metadata_map(path)?),
        None => None,
    };

    let pattern = compile_pattern(args.pattern.as_deref())?;

    let rows: Vec<Row> = index
        .objects
        .iter()
        .filter(|(base, _)| {
            pattern
                .as_ref()
                .map_or(true, |p| p.is_match(base).unwrap_or(false))
        })
        .map(|(base, info)| {
            let entry_meta = metadata
                .as_ref()
                .and_then(|m| shaders_bin::lookup_by_base(m, base));
            Row {
                base: base.clone(),
                presence: presence_grid(info),
                meta: entry_meta.cloned(),
            }
        })
        .collect();

    match args.format {
        ListOutputFormat::Table => render_table(&rows, args.show_stats, &index),
        ListOutputFormat::Json => render_json(&rows)?,
        ListOutputFormat::Csv => render_csv(&rows)?,
        ListOutputFormat::Flat => render_flat(&rows),
    }

    Ok(())
}

struct Row {
    base: String,
    /// 4×2 grid of (platform, type) presence flags
    /// (Dx9, Dx11, Glsl, Metal) × (Vs, Ps).
    presence: Vec<(PlatformArg, ShaderTypeArg, bool)>,
    meta: Option<ShaderEntryMeta>,
}

fn presence_grid(
    info: &crate::wad_index::ShaderObjectIndex,
) -> Vec<(PlatformArg, ShaderTypeArg, bool)> {
    let mut out = Vec::with_capacity(8);
    for &platform in PlatformArg::all() {
        for &shader_type in ShaderTypeArg::all() {
            let present = info.combinations.contains_key(&(shader_type, platform));
            out.push((platform, shader_type, present));
        }
    }
    out
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

fn render_table(rows: &[Row], show_stats: bool, index: &WadIndex) {
    let header = format!(
        "{:<70}  {:>4}  {:>4}  {:>4}  {:>4}  {:>4}  {:>4}  {:>4}  {:>4}",
        "Shader Object", "VS9", "PS9", "VS11", "PS11", "VSGL", "PSGL", "VSMT", "PSMT"
    );
    println!("{}", header.bold());
    println!("{}", "─".repeat(118));
    for row in rows {
        let cells: Vec<String> = row
            .presence
            .iter()
            .map(|(_, _, present)| {
                if *present {
                    "✓".green().to_string()
                } else {
                    "·".dimmed().to_string()
                }
            })
            .collect();
        println!(
            "{:<70}  {:>4}  {:>4}  {:>4}  {:>4}  {:>4}  {:>4}  {:>4}  {:>4}",
            truncate_middle(&row.base, 70),
            cells[0],
            cells[1],
            cells[2],
            cells[3],
            cells[4],
            cells[5],
            cells[6],
            cells[7]
        );
        if let Some(meta) = &row.meta {
            println!(
                "    {} params, {} switches, {} textures, FeatureMask={}",
                meta.parameter_names.len(),
                meta.static_switches.len(),
                meta.textures.len(),
                meta.feature_mask
            );
        }
    }
    if show_stats {
        println!();
        println!("{}", "Stats".bold());
        println!("  shader objects:   {}", rows.len());
        println!("  total chunks:     {}", index.total_chunks);
        println!("  resolved chunks:  {}", index.resolved_chunks);
    }
}

fn render_flat(rows: &[Row]) {
    for row in rows {
        println!("{}", row.base);
    }
}

fn render_csv(rows: &[Row]) -> Result<()> {
    println!(
        "base,vs_dx9,ps_dx9,vs_dx11,ps_dx11,vs_glsl,ps_glsl,vs_metal,ps_metal,parameters,switches,textures,feature_mask"
    );
    for row in rows {
        let p: Vec<&str> = row
            .presence
            .iter()
            .map(|(_, _, p)| if *p { "1" } else { "0" })
            .collect();
        let (params, switches, textures, mask) = match &row.meta {
            Some(m) => (
                m.parameter_names.len().to_string(),
                m.static_switches.len().to_string(),
                m.textures.len().to_string(),
                m.feature_mask.to_string(),
            ),
            None => ("".into(), "".into(), "".into(), "".into()),
        };
        println!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{}",
            csv_quote(&row.base),
            p[0],
            p[1],
            p[2],
            p[3],
            p[4],
            p[5],
            p[6],
            p[7],
            params,
            switches,
            textures,
            mask
        );
    }
    Ok(())
}

fn csv_quote(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[derive(serde::Serialize)]
struct JsonRow<'a> {
    base: &'a str,
    presence: Vec<JsonPresence>,
    metadata: Option<&'a ShaderEntryMeta>,
}

#[derive(serde::Serialize)]
struct JsonPresence {
    shader_type: String,
    platform: String,
    present: bool,
}

fn render_json(rows: &[Row]) -> Result<()> {
    let json_rows: Vec<JsonRow> = rows
        .iter()
        .map(|r| JsonRow {
            base: &r.base,
            presence: r
                .presence
                .iter()
                .map(|(plat, ty, present)| JsonPresence {
                    shader_type: ty.as_extension().to_string(),
                    platform: plat.as_extension().to_string(),
                    present: *present,
                })
                .collect(),
            metadata: r.meta.as_ref(),
        })
        .collect();

    // ShaderEntryMeta isn't serializable yet; serialize compatibly via a wrapper later.
    // For now, drop metadata in JSON to keep the impl small.
    #[derive(serde::Serialize)]
    struct JsonRowSlim<'a> {
        base: &'a str,
        presence: &'a [JsonPresence],
        metadata: Option<JsonMetaSlim<'a>>,
    }
    #[derive(serde::Serialize)]
    struct JsonMetaSlim<'a> {
        parameter_names: &'a [String],
        feature_mask: u32,
    }
    let slim: Vec<JsonRowSlim> = json_rows
        .iter()
        .map(|r| JsonRowSlim {
            base: r.base,
            presence: &r.presence,
            metadata: r.metadata.map(|m| JsonMetaSlim {
                parameter_names: &m.parameter_names,
                feature_mask: m.feature_mask,
            }),
        })
        .collect();

    println!("{}", serde_json::to_string_pretty(&slim)?);
    Ok(())
}

fn truncate_middle(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(3);
    let left = keep / 2;
    let right = keep - left;
    format!("{}...{}", &s[..left], &s[s.len() - right..])
}
