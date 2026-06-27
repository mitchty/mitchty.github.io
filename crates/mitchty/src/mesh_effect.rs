//! Per-mesh vertex effect material extension.
//!
//! Wraps every [`StandardMaterial`] mesh that loads from the GLTF scene in an
//! [`ExtendedMaterial<StandardMaterial, MeshEffectExtension>`]. Right now the
//! vertex shader is a pure passthrough cause I don't know how to use it yet
//! identical output to StandardMaterial so there should be no visible change.
//! The wiring is here so real effects can be dropped into a mesh shader later.

use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::*;

use crate::plugins::scene::LoadedScene;
use crate::plugins::text3d::Text3d;

/// Convenience alias so the type doesn't eat the whole line everywhere.
type ExtMat = ExtendedMaterial<StandardMaterial, MeshEffectExtension>;

/// Tracks the [`AssetId`] of every [`ExtMat`] created for the current scene.
///
/// When the scene is replaced, [`cleanup_ext_materials_on_scene_replace`]
/// drains this list and calls [`Assets::remove`] on each entry directly.
/// This forces immediate synchronous eviction rather than waiting for Bevy's
/// async `AssetEvent::Unused` GC cycle, which can lag by several frames and
/// let dead assets accumulate across multiple scene reloads.
#[derive(Resource, Default)]
pub struct TrackedExtMaterials {
    ids: Vec<AssetId<ExtMat>>,
}

/// Plugin that registers the extended material pipeline and the system that
/// swaps newly-spawned [`StandardMaterial`] meshes over to it.
///
/// Note: this plugin isn't part of the toggle system it more a render graph plugin.
pub struct MeshEffectPlugin;

impl Plugin for MeshEffectPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ExtMat>::default())
            .init_resource::<TrackedExtMaterials>()
            .add_systems(Update, apply_mesh_effect)
            .add_systems(Update, cleanup_ext_materials_on_scene_replace);
    }
}

/// Material extension with no extra uniforms right now just the vertex shader hook.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct MeshEffectExtension {
    // No uniforms yet for future `#[uniform(100)] pub time: f32,`
}

impl MaterialExtension for MeshEffectExtension {
    // No vertex_shader() override for now defaults to ShaderRef::Default which
    // uses Bevy's standard mesh vertex pipeline. The custom WESL vertex shader
    // is a passthrough that reproduced that behavior exactly, but WESL cannot
    // import bevy_pbr naga_oil modules so it panicked at pipeline compile time.
    // this experiment of vertex shaders sharing code with fragment via wesl
    // modules etc... is a future problem once wesl is more fleshed out over
    // naga_oil in bevy.
}

/// Swap every newly-added [`StandardMaterial`] mesh to the extended material we
/// can use here.
///
/// Uses [`Added`] so it only fires once per entity, the frame the component
/// appears. After the swap the entity has
/// `MeshMaterial3d<ExtendedMaterial<...>>` and no longer matches, so the system
/// never runs again for that entity.
///
/// `Without<Text3d>` guards against hijacking the 3D text mesh entities spawned
/// by flan's slug text renderer. SlugText only entities use `SlugMaterial` and
/// must not have the extended mesh effect material applied on top of them.
/// Future mitch sould probably make "3d" slugtext entities a totally different
/// component but I'm still learning what makes sense as an api so wrong it is
/// for now. Future me can figure out the better option based on use
/// cases/callsites and fix it then.
///
/// Thinking about how I can deal with this in a node editor way too.
#[allow(clippy::type_complexity)]
pub fn apply_mesh_effect(
    query: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        (Added<MeshMaterial3d<StandardMaterial>>, Without<Text3d>),
    >,
    standard_materials: Res<Assets<StandardMaterial>>,
    mut extended_materials: ResMut<Assets<ExtMat>>,
    mut tracked: ResMut<TrackedExtMaterials>,
    mut commands: Commands,
) {
    for (entity, mat_handle) in query.iter() {
        let Some(std_mat) = standard_materials.get(mat_handle) else {
            // Asset not yet loaded by the gltf loader, we can catch it next
            // tick with Added once the handle resolves. StandardMaterial meshes
            // that never load are simply ignored. Not sure what those would be
            // but why not.
            continue;
        };

        let ext_handle = extended_materials.add(ExtMat {
            base: std_mat.clone(),
            extension: MeshEffectExtension {},
        });

        // Record the id so cleanup_ext_materials_on_scene_replace can evict it
        // immediately when the scene is torn down, rather than waiting on
        // Bevy's async AssetEvent::Unused GC cycle.
        tracked.ids.push(ext_handle.id());

        // Use a world-level command so the entity existence check happens at
        // execution time, not command queue time.
        //
        // We need this for the cases where the gltf loader spawns an entity and
        // then modifies it in the same tick. Or between two ticks, either way
        // its a race where if replace_scene tears the gltf scene down and up in
        // the same frame. get_entity_mut can return an Err so just ignore it.
        //
        // This is not good error handling but I need to learn more about the
        // world level gltf scene behavior before I know the right thing to do.
        commands.queue(move |world: &mut World| {
            let Ok(mut e) = world.get_entity_mut(entity) else {
                return;
            };
            e.remove::<MeshMaterial3d<StandardMaterial>>();
            e.insert(MeshMaterial3d(ext_handle));
        });
    }
}

/// Explicitly evict all tracked [`ExtMat`] assets the moment the loaded scene
/// is despawned.
///
/// Bevy's normal asset cleanup only runs when it processes [`AssetEvent::Unused`]
/// events, which can lag several frames behind the actual handle drops. Calling
/// [`Assets::remove`] here ensures the backing data is freed immediately on the
/// same frame the scene tears down, preventing dead assets from accumulating
/// across multiple scene reloads.
///
/// [`Assets::remove`] is a no-op for IDs that were already cleaned up (e.g.
/// if the entity-command error branch dropped the only handle before this runs),
/// so there is no double-free concern.
pub fn cleanup_ext_materials_on_scene_replace(
    mut removed_scenes: RemovedComponents<LoadedScene>,
    mut tracked: ResMut<TrackedExtMaterials>,
    mut extended_materials: ResMut<Assets<ExtMat>>,
) {
    if removed_scenes.read().next().is_none() {
        return;
    }
    let count = tracked.ids.len();
    for id in tracked.ids.drain(..) {
        extended_materials.remove(id);
    }
    if count > 0 {
        debug!("evicted {} extended materials on scene replace", count);
    }
}
