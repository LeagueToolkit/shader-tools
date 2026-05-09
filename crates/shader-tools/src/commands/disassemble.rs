//! Format-aware shader bytecode disassembly via external tooling.
//!
//! `disassemble` does not decompile bytecode back to HLSL/MSL/GLSL source —
//! it produces *assembly listings* (e.g. SM5 asm, AIR LLVM-IR, SPIR-V text)
//! by shelling out to the platform-native tool. Decompilation is a separate
//! research-grade problem; what we ship here is "make the .bin readable".

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use eyre::{eyre, Context, Result};

pub struct DisassembleArgs {
    pub input: String,
    pub output: Option<String>,
    pub format: Option<DisassembleFormatArg>,
    pub tool: Option<String>,
}

#[derive(Copy, Clone, Debug, clap::ValueEnum)]
pub enum DisassembleFormatArg {
    /// Apple Metal library bytecode (MTLB magic). Requires `metal-objdump`.
    Metal,
    /// DirectX Shader Bytecode, Shader Model 5.x (DXBC magic). Requires `fxc.exe`.
    Dxbc,
    /// DirectX Intermediate Language, Shader Model 6.x (DXIL magic). Requires `dxc.exe`.
    Dxil,
    /// Khronos SPIR-V (0x07230203 magic). Requires `spirv-dis`.
    Spirv,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum DetectedFormat {
    Metal,
    Dxbc,
    Dxil,
    Spirv,
}

impl From<DisassembleFormatArg> for DetectedFormat {
    fn from(f: DisassembleFormatArg) -> Self {
        match f {
            DisassembleFormatArg::Metal => DetectedFormat::Metal,
            DisassembleFormatArg::Dxbc => DetectedFormat::Dxbc,
            DisassembleFormatArg::Dxil => DetectedFormat::Dxil,
            DisassembleFormatArg::Spirv => DetectedFormat::Spirv,
        }
    }
}

pub fn disassemble(args: DisassembleArgs) -> Result<()> {
    let input = PathBuf::from(&args.input);
    let bytes = fs::read(&input).with_context(|| format!("failed to read {}", input.display()))?;

    let format = match args.format {
        Some(f) => f.into(),
        None => detect_format(&bytes).ok_or_else(|| {
            eyre!(
                "could not detect bytecode format from magic {:02x?}; pass --format explicitly",
                &bytes[..bytes.len().min(8)]
            )
        })?,
    };

    let tool = resolve_tool(format, args.tool.as_deref())?;
    let output = run_tool(format, &tool, &input)?;

    match args.output {
        Some(p) => {
            fs::write(&p, output).with_context(|| format!("failed to write disassembly to {p}"))?;
            tracing::info!("wrote disassembly to {p}");
        }
        None => print!("{output}"),
    }
    Ok(())
}

fn detect_format(bytes: &[u8]) -> Option<DetectedFormat> {
    if bytes.len() < 4 {
        return None;
    }
    match &bytes[..4] {
        b"MTLB" => Some(DetectedFormat::Metal),
        b"DXBC" => Some(DetectedFormat::Dxbc),
        b"DXIL" => Some(DetectedFormat::Dxil),
        // SPIR-V magic word is 0x07230203 (LE) / 0x03022307 (BE)
        [0x03, 0x02, 0x23, 0x07] | [0x07, 0x23, 0x02, 0x03] => Some(DetectedFormat::Spirv),
        _ => None,
    }
}

/// Pick the tool name. CLI override (`--tool`) wins; otherwise env var
/// (e.g. `LTK_METAL_OBJDUMP`); otherwise the default name (resolved via PATH).
fn resolve_tool(format: DetectedFormat, override_path: Option<&str>) -> Result<String> {
    if let Some(p) = override_path {
        return Ok(p.to_string());
    }
    let (env_var, default) = match format {
        DetectedFormat::Metal => ("LTK_METAL_OBJDUMP", "metal-objdump"),
        DetectedFormat::Dxbc => ("LTK_FXC", "fxc"),
        DetectedFormat::Dxil => ("LTK_DXC", "dxc"),
        DetectedFormat::Spirv => ("LTK_SPIRV_DIS", "spirv-dis"),
    };
    if let Ok(v) = std::env::var(env_var) {
        if !v.is_empty() {
            return Ok(v);
        }
    }
    Ok(default.to_string())
}

fn run_tool(format: DetectedFormat, tool: &str, input: &Path) -> Result<String> {
    let mut cmd = Command::new(tool);
    let install_hint = match format {
        DetectedFormat::Metal => {
            cmd.arg("--disassemble").arg(input);
            "Install Apple's Metal Developer Tools (Xcode on macOS, or the Metal Developer Tools \
             for Windows from developer.apple.com)."
        }
        DetectedFormat::Dxbc => {
            // fxc /dumpbin reads an already-compiled blob and /Nologo silences the banner;
            // /Fc writes the assembly listing. We let it write to stdout via /Fc nul → stderr,
            // but fxc requires a path so we use a temp file or capture differently. The
            // simplest portable approach is to use a temp file.
            let listing = make_temp_listing_path(input);
            cmd.args([OsStr::new("/Nologo"), OsStr::new("/dumpbin")])
                .arg(input)
                .arg("/Fc")
                .arg(&listing);
            return run_with_listing_file(
                cmd,
                format,
                listing,
                "Install the Windows SDK (provides fxc.exe).",
            );
        }
        DetectedFormat::Dxil => {
            // dxc has -dumpbin which prints disassembly to stdout when no -Fc is given.
            cmd.arg("-nologo").arg("-dumpbin").arg(input);
            "Install the DirectX Shader Compiler (dxc.exe). It ships with the Windows SDK \
             and is also available standalone from microsoft/DirectXShaderCompiler."
        }
        DetectedFormat::Spirv => {
            cmd.arg(input);
            "Install the Vulkan SDK (provides spirv-dis) or build SPIRV-Tools from source."
        }
    };
    run_capturing(cmd, format, install_hint)
}

fn make_temp_listing_path(input: &Path) -> PathBuf {
    let mut tmp = std::env::temp_dir();
    let stem = input.file_stem().unwrap_or_else(|| OsStr::new("shader"));
    let nonce = std::process::id();
    let mut name = stem.to_owned();
    name.push(format!(".{nonce}.disasm.txt"));
    tmp.push(name);
    tmp
}

fn run_with_listing_file(
    mut cmd: Command,
    format: DetectedFormat,
    listing: PathBuf,
    install_hint: &str,
) -> Result<String> {
    let output = cmd
        .output()
        .map_err(|e| missing_tool_error(format, install_hint, e))?;
    if !output.status.success() {
        let _ = fs::remove_file(&listing);
        return Err(eyre!(
            "{} exited with {}: {}",
            cmd.get_program().to_string_lossy(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let listing_text = fs::read_to_string(&listing)
        .with_context(|| format!("failed to read listing from {}", listing.display()))?;
    let _ = fs::remove_file(&listing);
    Ok(listing_text)
}

fn run_capturing(mut cmd: Command, format: DetectedFormat, install_hint: &str) -> Result<String> {
    let output = cmd
        .output()
        .map_err(|e| missing_tool_error(format, install_hint, e))?;
    if !output.status.success() {
        return Err(eyre!(
            "{} exited with {}: {}",
            cmd.get_program().to_string_lossy(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn missing_tool_error(
    format: DetectedFormat,
    install_hint: &str,
    err: std::io::Error,
) -> eyre::Report {
    let tool = match format {
        DetectedFormat::Metal => "metal-objdump",
        DetectedFormat::Dxbc => "fxc",
        DetectedFormat::Dxil => "dxc",
        DetectedFormat::Spirv => "spirv-dis",
    };
    eyre!(
        "could not run {tool}: {err}\n  hint: {install_hint}\n  \
         override the binary path with --tool <path> or set the corresponding env var \
         (LTK_METAL_OBJDUMP / LTK_FXC / LTK_DXC / LTK_SPIRV_DIS)."
    )
}
