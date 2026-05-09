//! Local clap-friendly mirrors of `ltk_shader`'s `ShaderType` and
//! `GraphicsPlatform` enums (which are not `ValueEnum`).

use clap::ValueEnum;
use ltk_shader::{GraphicsPlatform, ShaderType};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, ValueEnum)]
pub enum ShaderTypeArg {
    Vs,
    Ps,
}

impl ShaderTypeArg {
    pub fn all() -> &'static [ShaderTypeArg] {
        &[ShaderTypeArg::Vs, ShaderTypeArg::Ps]
    }
}

impl From<ShaderTypeArg> for ShaderType {
    fn from(t: ShaderTypeArg) -> Self {
        match t {
            ShaderTypeArg::Vs => ShaderType::Vertex,
            ShaderTypeArg::Ps => ShaderType::Pixel,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, ValueEnum)]
pub enum PlatformArg {
    Dx9,
    Dx11,
    Glsl,
    Metal,
}

impl PlatformArg {
    pub fn all() -> &'static [PlatformArg] {
        &[
            PlatformArg::Dx9,
            PlatformArg::Dx11,
            PlatformArg::Glsl,
            PlatformArg::Metal,
        ]
    }
}

impl From<PlatformArg> for GraphicsPlatform {
    fn from(p: PlatformArg) -> Self {
        match p {
            PlatformArg::Dx9 => GraphicsPlatform::Dx9,
            PlatformArg::Dx11 => GraphicsPlatform::Dx11,
            PlatformArg::Glsl => GraphicsPlatform::Glsl,
            PlatformArg::Metal => GraphicsPlatform::Metal,
        }
    }
}

impl PlatformArg {
    pub fn as_extension(self) -> &'static str {
        GraphicsPlatform::from(self).extension()
    }
}

impl ShaderTypeArg {
    pub fn as_extension(self) -> &'static str {
        ShaderType::from(self).extension()
    }
}
