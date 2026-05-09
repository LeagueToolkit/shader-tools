use std::fs::File;

use colored::Colorize;
use eyre::Result;
use ltk_shader::loader::ShaderLoader;
use ltk_wad::Wad;

use crate::args::{PlatformArg, ShaderTypeArg};
use crate::utils::load_hashtable;
use crate::wad_index::{make_shader_object_path, WadIndex};

pub struct InfoArgs {
    pub input: String,
    pub shader: String,
    pub hashtable: Option<String>,
    pub hashtable_dir: Option<String>,
}

pub fn info(args: InfoArgs) -> Result<()> {
    let wad_file = File::open(&args.input)?;
    let mut wad = Wad::mount(wad_file)?;
    let hashtable = load_hashtable(args.hashtable.as_deref(), args.hashtable_dir.as_deref())?;

    let index = WadIndex::build(&wad, &hashtable)?;
    let (base, info) = index.find(&args.shader)?;

    println!("{}", base.bold());

    for &platform in PlatformArg::all() {
        for &shader_type in ShaderTypeArg::all() {
            let key = (shader_type, platform);
            let Some(toc_path) = info.combinations.get(&key) else {
                continue;
            };

            print!(
                "  {}.{}  ",
                shader_type.as_extension(),
                platform.as_extension()
            );

            let object_path = make_shader_object_path(base, shader_type, platform);
            let toc = ShaderLoader::load_toc(
                &object_path.base,
                shader_type.into(),
                platform.into(),
                &mut wad,
            );

            match toc {
                Ok(toc) => {
                    let bundles = info
                        .bundle_ids
                        .get(&key)
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]);
                    println!(
                        "{} shaders, {} base defines, {} bundle(s) {:?}",
                        toc.shader_ids.len(),
                        toc.base_defines.len(),
                        bundles.len(),
                        bundles
                    );
                    if !toc.base_defines.is_empty() {
                        println!(
                            "      base defines: {}",
                            toc.base_defines
                                .iter()
                                .map(|d| d.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                    if let (Some(&first), Some(&last)) =
                        (toc.shader_ids.first(), toc.shader_ids.last())
                    {
                        println!("      shader_id range: [{first}..={last}]");
                    }
                }
                Err(e) => {
                    println!("{} (toc path: {})", format!("error: {e}").red(), toc_path);
                }
            }
        }
    }

    Ok(())
}
