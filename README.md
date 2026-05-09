# shader-tools

CLI for inspecting and dumping shader bytecode bundles from League of Legends
`Shaders.wad` / `ShaderCache.<platform>.wad.client` files.

Built on top of the [league-toolkit](https://github.com/LeagueToolkit/league-toolkit)
Rust workspace.

## Commands

| Command | Description |
| --- | --- |
| `dump` | Extract shader bytecode bundles into a mirrored directory tree with `manifest.json` sidecars. |
| `list` | List shader objects defined in `Shaders.bin`, with metadata and per-platform presence. |
| `info` | Print metadata for a single shader (parameters, switches, textures, feature defines). |
| `download-hashes` | Download/refresh the WAD hashtables from CommunityDragon. |

## Status

Pre-release. APIs and CLI surface may change.
