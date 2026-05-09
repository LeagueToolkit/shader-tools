use camino::Utf8Path;
use clap::builder::{styling::AnsiColor, Styles};
use clap::error::ErrorKind;
use clap::{Parser, Subcommand, ValueEnum};
use tracing::Level;
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{filter, fmt};
use utils::config::{default_config_path, load_or_create_config, resolve_and_persist_progress};
use utils::default_hashtable_dir;

use crate::args::{PlatformArg, ShaderTypeArg};

mod args;
mod commands;
mod manifest;
mod shader_path;
mod shaders_bin;
mod utils;
mod wad_index;

use commands::*;

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum VerbosityLevel {
    /// Show errors and above
    Error,
    /// Show warnings and above
    Warning,
    /// Show info messages and above
    Info,
    /// Show debug messages and above
    Debug,
    /// Show all messages including trace
    Trace,
}

impl From<VerbosityLevel> for Level {
    fn from(level: VerbosityLevel) -> Self {
        match level {
            VerbosityLevel::Error => Level::ERROR,
            VerbosityLevel::Warning => Level::WARN,
            VerbosityLevel::Info => Level::INFO,
            VerbosityLevel::Debug => Level::DEBUG,
            VerbosityLevel::Trace => Level::TRACE,
        }
    }
}

impl VerbosityLevel {
    pub fn to_level_filter(&self) -> LevelFilter {
        LevelFilter::from_level((*self).into())
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None, styles = cli_styles())]
struct Args {
    /// Set the verbosity level
    #[arg(short = 'L', long, value_enum, default_value_t = VerbosityLevel::Info)]
    verbosity: VerbosityLevel,

    /// Optional path to a config file (TOML). Defaults to `shader-tools.toml` if present
    #[arg(long)]
    config: Option<String>,

    /// Show or hide progress bars: true/false (overrides config). Example: --progress=false
    #[arg(long, value_name = "true|false")]
    progress: Option<bool>,

    /// Optional directory to recursively load hashtable files from
    /// Overrides the default discovery directory and config value when provided
    #[arg(long, value_name = "DIR")]
    hashtable_dir: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Print the default hashtable directory
    #[command(visible_alias = "hd")]
    HashtableDir,

    /// Download/update WAD hashtables from CommunityDragon
    ///
    /// Downloads hashes.game.txt and hashes.lcu.txt to the configured hashtable directory.
    /// Hashtables are shared with `wadtools` — installing one buys both.
    #[command(visible_alias = "dl")]
    DownloadHashes,

    /// List shader objects in a bundle WAD
    ///
    /// Walks every chunk in the input WAD and groups them by shader object,
    /// showing which (type, platform) combinations exist.
    #[command(visible_alias = "ls")]
    List {
        /// Path to the input shader bundle WAD (e.g. ShaderCache.metal.wad.client)
        #[arg(short, long)]
        input: String,

        /// Optional path to a metadata source — either a `Shaders.wad.client`
        /// file or a raw `data/shaders/shaders.bin` file. Used to enrich
        /// listings with parameter/switch/texture counts.
        #[arg(short = 'm', long)]
        metadata: Option<String>,

        /// Path to a hashtable file
        #[arg(short = 'H', long)]
        hashtable: Option<String>,

        /// Only list shader objects whose path matches this regex (case-insensitive by default)
        #[arg(short = 'x', long, value_name = "REGEX")]
        pattern: Option<String>,

        /// Output format
        #[arg(short = 'F', long, value_enum, default_value_t = ListOutputFormat::Table)]
        format: ListOutputFormat,

        /// Show summary statistics
        #[arg(short = 's', long, default_value_t = true)]
        stats: bool,
    },

    /// Show details for a single shader object
    ///
    /// `--shader` is matched against the resolved chunk paths
    /// (case-insensitive substring or exact match). Use `list` to find paths.
    Info {
        /// Path to the input shader bundle WAD
        #[arg(short, long)]
        input: String,

        /// Shader object path or substring (e.g. "vfx_gloom_ground" matches the full path)
        #[arg(short, long)]
        shader: String,

        /// Path to a hashtable file
        #[arg(short = 'H', long)]
        hashtable: Option<String>,
    },

    /// Disassemble a single shader bytecode `.bin` to assembly text
    ///
    /// Auto-detects the bytecode format (Metal/DXBC/DXIL/SPIR-V) from the
    /// magic bytes and shells out to the platform tool. This produces an
    /// assembly listing — not HLSL/MSL/GLSL source.
    #[command(visible_alias = "dis")]
    Disassemble {
        /// Path to the input `.bin` file
        #[arg(short, long)]
        input: String,

        /// Optional output file (defaults to stdout)
        #[arg(short, long)]
        output: Option<String>,

        /// Force a format instead of auto-detecting from the magic
        #[arg(short = 'F', long, value_enum)]
        format: Option<DisassembleFormatArg>,

        /// Path to the disassembler binary (overrides PATH lookup and env vars)
        #[arg(long)]
        tool: Option<String>,
    },

    /// Dump shader bytecode bundles into a mirrored directory tree
    Dump {
        /// Path to the input shader bundle WAD
        #[arg(short, long)]
        input: String,

        /// Output directory (default: <input_stem>_dump)
        #[arg(short, long)]
        output: Option<String>,

        /// Filter by shader type (default: both vs and ps)
        #[arg(short = 't', long = "type", value_enum, num_args = 1..)]
        r#type: Option<Vec<ShaderTypeArg>>,

        /// Filter by graphics platform (default: all four)
        #[arg(short = 'p', long, value_enum, num_args = 1..)]
        platform: Option<Vec<PlatformArg>>,

        /// Only dump shader objects whose path matches this regex
        #[arg(short = 'x', long, value_name = "REGEX")]
        pattern: Option<String>,

        /// Only dump shader objects matching one of these paths/substrings (repeatable)
        #[arg(short = 's', long)]
        shader: Option<Vec<String>>,

        /// Optional metadata source (Shaders.wad.client or raw shaders.bin) — when
        /// provided, per-platform manifest.json files include ShadersBin metadata
        /// (parameters, switches, textures, feature defines).
        #[arg(short = 'm', long)]
        metadata: Option<String>,

        /// Path to a hashtable file
        #[arg(short = 'H', long)]
        hashtable: Option<String>,
    },
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            if matches!(
                e.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion | ErrorKind::MissingSubcommand
            ) {
                let _ = load_or_create_config(Some(default_config_path().as_path()));
                e.print()?;
                return Ok(());
            } else {
                e.exit();
            }
        }
    };

    let config_path = args
        .config
        .as_deref()
        .map(Utf8Path::new)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_config_path);
    let (mut config, resolved_path) = load_or_create_config(Some(config_path.as_path()))?;
    let show_progress =
        resolve_and_persist_progress(&mut config, resolved_path.as_path(), args.progress)?;

    initialize_tracing(args.verbosity, show_progress)?;

    let hashtable_dir = args.hashtable_dir.or_else(|| config.hashtable_dir.clone());

    match args.command {
        Commands::HashtableDir => {
            if let Some(dir) = default_hashtable_dir() {
                println!("{}", dir);
            } else {
                println!("<no default hashtable directory>");
            }
            Ok(())
        }
        Commands::DownloadHashes => download_hashes(DownloadHashesArgs { hashtable_dir }),
        Commands::Disassemble {
            input,
            output,
            format,
            tool,
        } => disassemble(DisassembleArgs {
            input,
            output,
            format,
            tool,
        }),
        Commands::List {
            input,
            metadata,
            hashtable,
            pattern,
            format,
            stats,
        } => list(ListArgs {
            input,
            metadata,
            pattern,
            format,
            show_stats: stats,
            hashtable,
            hashtable_dir,
        }),
        Commands::Info {
            input,
            shader,
            hashtable,
        } => info(InfoArgs {
            input,
            shader,
            hashtable,
            hashtable_dir,
        }),
        Commands::Dump {
            input,
            output,
            r#type,
            platform,
            pattern,
            shader,
            metadata,
            hashtable,
        } => dump(DumpArgs {
            input,
            output,
            r#type,
            platform,
            pattern,
            shader,
            metadata,
            hashtable,
            hashtable_dir,
        }),
    }
}

fn initialize_tracing(verbosity: VerbosityLevel, show_progress: bool) -> eyre::Result<()> {
    let indicatif_layer = IndicatifLayer::new();

    let common_format = fmt::format()
        .with_ansi(true)
        .with_level(true)
        .with_source_location(false)
        .with_line_number(false)
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::time());

    let stdout_layer = fmt::layer()
        .with_writer(indicatif_layer.get_stdout_writer())
        .event_format(common_format.clone())
        .with_filter(filter::filter_fn(move |metadata| {
            let level = *metadata.level();
            match verbosity {
                VerbosityLevel::Error => false,
                VerbosityLevel::Warning => level == Level::WARN || level == Level::ERROR,
                VerbosityLevel::Info => {
                    level == Level::INFO || level == Level::WARN || level == Level::ERROR
                }
                VerbosityLevel::Debug => level != Level::TRACE,
                VerbosityLevel::Trace => true,
            }
        }));

    let stderr_layer = fmt::layer()
        .with_writer(indicatif_layer.get_stderr_writer())
        .event_format(common_format)
        .with_filter(filter::filter_fn(move |metadata| {
            let level = *metadata.level();
            match verbosity {
                VerbosityLevel::Error => level == Level::ERROR,
                VerbosityLevel::Warning => level == Level::WARN || level == Level::ERROR,
                _ => level == Level::WARN || level == Level::ERROR,
            }
        }));

    let registry = tracing_subscriber::registry()
        .with(stdout_layer)
        .with(stderr_layer)
        .with(verbosity.to_level_filter());

    if show_progress {
        registry.with(indicatif_layer).init();
    } else {
        registry.init();
    }
    Ok(())
}

fn cli_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Yellow.on_default().bold())
        .usage(AnsiColor::Green.on_default().bold())
        .literal(AnsiColor::Cyan.on_default())
        .placeholder(AnsiColor::Magenta.on_default())
}
