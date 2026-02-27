// Note, most all of this library code is centered around testing and building
// wesl shaders into wgsl and abusing wgpu to render a shader to an image for
// unit testing purposes. This is mostly here for me to work out how to unit
// test shaders be they fragment/vertex/compute etc... and to get a better idea
// of if a platform differs visually.
//
// FOR NOW thats not happening (I'm just rendering the image first round then
// testing against that on every platform.)
//
// BUTTTTTTT... its something that could happen in the future if I get a bug up
// my butt to do it.
//
// `shaders` crate — centralised WESL shader compilation and registration.
//
// build.rs scans src/shaders/*.wesl, compiles every variant into
// $OUT_DIR/bevy/{default,webgl}/{material,ui}/<name>.wgsl, then generates
// shader_registry.rs which is include!()-ed below.
//
// Public api is mostly `shaders::BEVY_{DEFAULT|WEBGL}_{UI|MATERIAL}_THING`
//
// And the `ShadersPLugin` to make it easy to add to any bevy app so the asset
// server can hand over the shaders at runtime.

// Future mitch note, `render` feature is for unit testing with wgpu.
//   * `shaders::wesl`     — "compile" wesl->wgsl crap
//   * `shaders::render`   — "render" wgsl to a bucket of bits via wgpu
//   * `shaders::snapshot` — "snapshot" compare rendered crap to reference png images
use bevy::app::{App, Plugin};
use bevy::shader::Shader;

#[cfg(feature = "render")]
pub mod render;
#[cfg(feature = "render")]
pub mod snapshot;
#[cfg(feature = "render")]
pub mod wesl;

// build.rs generates this crap based off the input wesl shaders to make a
// plugin I can abuse that sets up embedded assets for bevy usage.
//
// Note bevy exists not cause I think its the best, but because outside of unit
// tests I'm not using wgpu directly to render crap. I might in future but until
// then, bevy is assumed.
include!(concat!(env!("OUT_DIR"), "/shaders.rs"));

// This will probably eventually become the flan crate once that becomes its own
// thing. But while I figure that crap out for now its here, cause I've got
// nowhere better to yeet all this crap.
pub struct ShadersPlugin;

impl Plugin for ShadersPlugin {
    fn build(&self, app: &mut App) {
        _register_shaders(app);
    }
}
