use bevy::prelude::*;

use crate::plugins::fonts::RegisteredFonts;
use crate::plugins::{PluginEnabled, PluginRegistry, run_if_enabled};

/// Font used to render reverie *content* in the 3-D typst MVP view.
///
/// TODO: Future me needs to deal with fonts that have ligatures more as typst
/// dumps out ligatures a lot. And some fonts apparently are broken.
const REVERIE_FONT_NAME: &str = "FiraMono-Medium.ttf";

// Build.rs generated data from the reveries dir typst files.
include!(concat!(env!("OUT_DIR"), "/reveries_generated.rs"));

/// All lookup keys for a specific reverie.
///
/// `keys[0]` is the canonical path key (e.g. `"lorem_ipsum"`,
/// `"some_ns/baz_qux"`, `"a/b/c/deep"`). Additional keys are just aliases for
/// possible future backwards compatibility to keep existing links working.
#[derive(Component)]
pub struct ReverieKey(pub &'static [&'static str]);

// TODO: Should I just use a hashkey instead for all this stuff? This is a lot
// of code for key/val lookup basically.
impl ReverieKey {
    /// Canonical key just first element
    pub fn canonical(&self) -> &'static str {
        self.0[0]
    }

    /// Path segments of the canonical key split via `'/'`, mostly here for
    /// future menu item support.
    #[cfg(test)]
    pub fn segments(&self) -> Vec<&'static str> {
        self.canonical().split('/').collect()
    }

    /// `true` if `name` matches any key in this slice, case-insensitively.
    pub fn matches(&self, name: &str) -> bool {
        let lower = name.to_lowercase();
        self.0.iter().any(|k| k.to_lowercase() == lower)
    }
}

/// Human-readable leaf name shown in the menu (e.g. `"Lorem Ipsum"`, `"Baz Qux"`).
#[derive(Component)]
pub struct ReverieDisplayName(pub &'static str);

/// Raw typst content of the reverie file, embedded at compile time. (for now, I might make this dynamic in future)
#[derive(Component)]
pub struct ReverieContent(pub &'static str);

/// The currently displayed reverie, identified by its ECS entity.
///
/// `None`         -> nothing shown.
/// `Some(entity)` -> the reverie that  `ReverieContent` is rendered from.
#[derive(Resource, Default)]
pub struct ActiveReverie(pub Option<Entity>);

/// `SystemSet` that gates all per-frame systems owned by [`ReveriesPlugin`].
///
/// Controlled by `PluginEnabled::<ReveriesPlugin>`. Only toggles the bevy ui
/// view for now. Need to make this also control what egui displays shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct ReveriesSystems;

pub struct ReveriesPlugin;

impl Plugin for ReveriesPlugin {
    fn build(&self, app: &mut App) {
        // Spawn all reverie entities at plugin build time. Not like I'll add
        // one dynamically.
        for &(keys, display, content) in REVERIE_DATA {
            app.world_mut().spawn((
                ReverieKey(keys),
                ReverieDisplayName(display),
                ReverieContent(content),
            ));
        }

        app.insert_resource(PluginEnabled::<ReveriesPlugin>::default())
            .configure_sets(
                Update,
                ReveriesSystems.run_if(run_if_enabled::<ReveriesPlugin>()),
            )
            .init_resource::<ActiveReverie>()
            .init_resource::<ReverieViewState>()
            .add_systems(
                Update,
                (sync_typst_reverie_view, reveal_ready_reverie_view)
                    .chain()
                    .in_set(ReveriesSystems),
            );

        if let Some(mut registry) = app.world_mut().get_resource_mut::<PluginRegistry>() {
            registry.register::<ReveriesPlugin>("Reveries", true);
        }
    }
}

/// Find a reverie entity by matching any key or the display name,
/// case-insensitively.
///
/// --reverie "Lorem Ipsum" and --reverie lorem_ipsum both match equivalent via
/// `ReverieKey::matches` and then falls back to `ReverieDisplayName`. This
/// means `--reverie "Lorem Ipsum"` and `--reverie lorem_ipsum` both resolve the
/// same and correctly.
pub fn find_reverie<'a>(
    name: &str,
    mut iter: impl Iterator<Item = (Entity, &'a ReverieKey, &'a ReverieDisplayName)>,
) -> Option<Entity> {
    let lower = name.to_lowercase();
    iter.find(|(_, key, display)| key.matches(&lower) || display.0.to_lowercase() == lower)
        .map(|(e, _, _)| e)
}

/// Resolve `ui_config.initial_reverie` when a user asks for one via the cli or
/// via an http link to an entity and sets `active_reverie`. Logs a warning and
/// leaves `active_reverie` unchanged when no match is found as if it was never
/// there. Not sure this makes sense for the wasm version but for now don't care
/// too much about failure modes here.
pub fn apply_initial_reverie(
    name: &str,
    reveries: &Query<(Entity, &ReverieKey, &ReverieDisplayName)>,
    active_reverie: &mut ActiveReverie,
) {
    match find_reverie(name, reveries.iter()) {
        Some(entity) => active_reverie.0 = Some(entity),
        None => bevy::log::warn!("reverie {:?} not found, ignoring", name),
    }
}

/// A node in the reverie menu tree, built at render-time from entity key paths.
pub enum ReverieNode {
    /// A selectable leaf item.
    Leaf {
        entity: Entity,
        display: &'static str,
    },
    /// A named submenu grouping children at the next path depth.
    Group {
        /// Title-cased display name derived from the directory path segment.
        display: String,
        children: Vec<ReverieNode>,
    },
}

/// Build a `ReverieNode` tree from `(Entity, &ReverieKey, &ReverieDisplayName)`
/// entries (which should be pre-sorted by canonical key for a stable menu order).
///
/// The hierarchy is derived entirely from each entry's canonical key by splitting
/// on `'/'` so nesting can be arbitrary just like the fs its built off of.
pub fn build_reverie_tree(
    entries: &[(Entity, &ReverieKey, &ReverieDisplayName)],
) -> Vec<ReverieNode> {
    build_subtree(entries, "")
}

fn build_subtree(
    entries: &[(Entity, &ReverieKey, &ReverieDisplayName)],
    prefix: &str,
) -> Vec<ReverieNode> {
    let mut leaves: Vec<ReverieNode> = Vec::new();
    // BTreeMap for deterministic, basically alphabetic group ordering. This
    // might be fun if I have something encoded as utf8 with kanji but thats a
    // future me problem.
    let mut groups: std::collections::BTreeMap<
        String,
        Vec<(Entity, &ReverieKey, &ReverieDisplayName)>,
    > = std::collections::BTreeMap::new();

    for &(entity, key, display) in entries {
        let canonical = key.canonical();
        let remaining = if prefix.is_empty() {
            canonical
        } else {
            canonical
                .strip_prefix(prefix)
                .and_then(|s| s.strip_prefix('/'))
                .unwrap_or(canonical)
        };

        if let Some(slash) = remaining.find('/') {
            // Still has sub-path: belongs to a group named by the next segment.
            groups
                .entry(remaining[..slash].to_owned())
                .or_default()
                .push((entity, key, display));
        } else {
            // No more slashes: this is a leaf at the current level.
            leaves.push(ReverieNode::Leaf {
                entity,
                display: display.0,
            });
        }
    }

    let mut nodes = leaves;
    for (seg, group_entries) in groups {
        let new_prefix = if prefix.is_empty() {
            seg.clone()
        } else {
            format!("{}/{}", prefix, seg)
        };
        let display = seg
            .split(['_', '-'])
            .map(|w| {
                let mut ch = w.chars();
                match ch.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        nodes.push(ReverieNode::Group {
            display,
            children: build_subtree(&group_entries, &new_prefix),
        });
    }
    nodes
}

/// Build and render the full "Reveries" `menu_button` into `ui`.
///
/// Sorts entries by canonical key, builds the recursive `ReverieNode` tree,
/// renders it, then applies any pending click to `active_reverie`.
#[cfg(feature = "egui")]
pub fn reveries_egui_menu(
    ui: &mut bevy_egui::egui::Ui,
    entries: &[(Entity, &ReverieKey, &ReverieDisplayName)],
    active_reverie: &mut ActiveReverie,
) {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|(_, key, _)| key.canonical());

    let tree = build_reverie_tree(&sorted);
    let mut pending: Option<Option<Entity>> = None;

    ui.menu_button("Reveries", |ui| {
        render_reverie_nodes(ui, &tree, active_reverie.0, &mut pending);
    });

    if let Some(new_active) = pending {
        active_reverie.0 = new_active;
    }
}

/// Recursively render a `ReverieNode` slice into an egui `Ui`.
///
/// `pending` is written to on click:
/// - `Some(None)`         -> deactivate iff active
/// - `Some(Some(entity))` -> activate this entity/reverie
///
/// The caller applies `pending` to `ActiveReverie` after the menu closure so
/// the mutable resource borrow doesn't conflict with the egui closure.
#[cfg(feature = "egui")]
pub fn render_reverie_nodes(
    ui: &mut bevy_egui::egui::Ui,
    nodes: &[ReverieNode],
    active: Option<Entity>,
    pending: &mut Option<Option<Entity>>,
) {
    for node in nodes {
        match node {
            ReverieNode::Leaf { entity, display } => {
                let is_active = active == Some(*entity);
                if ui.selectable_label(is_active, *display).clicked() {
                    *pending = Some(if is_active { None } else { Some(*entity) });
                    ui.close();
                }
            }
            ReverieNode::Group { display, children } => {
                ui.menu_button(display, |ui| {
                    render_reverie_nodes(ui, children, active, pending);
                });
            }
        }
    }
}

/// Marker on a [`flan::typst_text::TypstTextNode`] entity rendering one
/// reverie's content in world space via the slug shader.
///
/// One such entity exists per reverie that has been in an app session. Entities
/// are never mutated in place or despawned once spawned. Once spawned
/// visibility and transforms control redisplay. No despawn is setup.
#[derive(Component)]
pub struct TypstReverieView {
    /// The reverie ECS entity whose content this view is showing.
    pub reverie: Entity,
}

/// Tracks which [`TypstReverieView`] entity is currently visible or being
/// waited to spawn, so [`sync_typst_reverie_view`] and
/// [`reveal_ready_reverie_view`] can coordinate the hidden-until-ready handoff
/// across frames/ticks.
#[derive(Resource, Default)]
struct ReverieViewState {
    /// The view entity currently `Visibility::Visible`, if any.
    visible: Option<Entity>,
    /// A view entity that has been spawned (or already existed) and is
    /// waiting to fully materialize before being swapped in.
    pending: Option<Entity>,
}

/// Compute a `Transform` that places a reverie view directly in front of the
/// current camera, facing it, sized to fill around 85% of the viewport height.
///
/// TODO: width too, I was lazy.
fn compute_reverie_transform(cam_gt: &GlobalTransform, proj: &Projection) -> Transform {
    let cam_mat = cam_gt.to_matrix();
    let cam_pos = cam_gt.translation();
    let cam_right = cam_mat.x_axis.truncate().normalize();
    let cam_up = cam_mat.y_axis.truncate().normalize();

    // Camera looks in its local -Z; world-space forward = -z_axis.
    let cam_forward = -cam_mat.z_axis.truncate().normalize();

    let depth = 3.0_f32;
    let clip = proj.get_clip_from_view();
    let cot_fov = clip.y_axis.y.max(0.001);
    let vis_h = 2.0 * depth / cot_fov;
    let scale = vis_h * 0.85;

    let text_pos = cam_pos + cam_forward * depth;

    let rot = Quat::from_mat3(&Mat3::from_cols(
        cam_right,
        cam_up,
        -cam_forward, // model +Z toward camera; det = +1 ✓
    ));

    Transform {
        translation: text_pos,
        rotation: rot,
        scale: Vec3::splat(scale),
    }
}

/// Dispatch [`ActiveReverie`] on changes to find-or-spawn the target reverie's
/// view entity and mark it [`ReverieViewState::pending`]. Actually swapping it
/// in is handled by [`reveal_ready_reverie_view`] once it has fully spawned.
///
/// - `ActiveReverie(None)` = hide the currently visible view and cancel any
///   in-flight pending swap. Does not despawn anything, so cached views stay
///   warm for reselection in case someone wants to click buttons fast.
/// - `ActiveReverie(Some(reverie))`:
///   - Already the visible reverie = no-op, cancels any stale pending swap to handle rapid reselections.
///   - A cached view entity already exists for this reverie = mark it pending;
///     it may already be ready, in which case [`reveal_ready_reverie_view`]
///     swaps it in as soon as next frame.
///   - No cached view entity yet = spawn a new entity with `Visibility::Hidden`
///     and [`flan::typst_text::TypstDirty`], leave it to spawn untouched, and
///     mark pending when ready.
///
/// Requires [`REVERIE_FONT_NAME`] to already be present in [`RegisteredFonts`];
/// if it hasn't loaded yet the system is a no-op. Re-runs whenever
/// [`RegisteredFonts`] changes so that selecting a reverie *before* the font
/// finishes loading still resolves correctly once registration completes,
/// without needing a dedicated one-shot setup system like
/// `text3d::setup_flan_font`.
fn sync_typst_reverie_view(
    active: Res<ActiveReverie>,
    content_q: Query<&ReverieContent>,
    view_q: Query<(Entity, &TypstReverieView)>,
    registered_fonts: Res<RegisteredFonts>,
    mut state: ResMut<ReverieViewState>,
    mut commands: Commands,
) {
    if !active.is_changed() && !registered_fonts.is_changed() {
        return;
    }

    let Some(font_id) = registered_fonts
        .0
        .iter()
        .find(|e| e.name == REVERIE_FONT_NAME)
        .map(|e| e.font_id)
    else {
        bevy::log::trace!(
            "sync_typst_reverie_view: {REVERIE_FONT_NAME} not yet registered, waiting"
        );
        return;
    };

    bevy::log::trace!(
        "sync_typst_reverie_view: ActiveReverie changed -> {:?}",
        active.0
    );

    match active.0 {
        None => {
            if let Some(visible) = state.visible.take() {
                bevy::log::trace!(
                    "sync_typst_reverie_view: hiding {:?} (deactivated)",
                    visible
                );
                commands.entity(visible).insert(Visibility::Hidden);
            }
            state.pending = None;
        }
        Some(reverie_entity) => {
            let already_visible = state.visible.is_some_and(|v| {
                view_q
                    .get(v)
                    .is_ok_and(|(_, view)| view.reverie == reverie_entity)
            });
            if already_visible {
                bevy::log::trace!(
                    "sync_typst_reverie_view: {:?} already visible, skipping",
                    reverie_entity
                );
                // Cancel a stale in-flight switch away from the reverie
                // already being displayed.
                state.pending = None;
                return;
            }

            let existing = view_q
                .iter()
                .find(|(_, view)| view.reverie == reverie_entity)
                .map(|(e, _)| e);

            match existing {
                Some(view_entity) => {
                    bevy::log::trace!(
                        "sync_typst_reverie_view: cached view {:?} found for {:?}, \
                         marking pending",
                        view_entity,
                        reverie_entity
                    );
                    state.pending = Some(view_entity);
                }
                None => {
                    let typst_source = content_q
                        .get(reverie_entity)
                        .map(|c| c.0)
                        .unwrap_or_default();

                    bevy::log::trace!(
                        "sync_typst_reverie_view: spawning new hidden view for {:?} \
                         (font={:?}, source len={})",
                        reverie_entity,
                        font_id,
                        typst_source.len()
                    );

                    let view_entity = commands
                        .spawn((
                            flan::typst_text::TypstTextNode {
                                source: typst_source.to_owned(),
                                font_id,
                                pixels_per_pt: 1.2,
                                color: [220, 220, 210, 255],
                            },
                            flan::typst_text::TypstDirty,
                            Transform::default(),
                            Visibility::Hidden,
                            Mesh3d::default(),
                            TypstReverieView {
                                reverie: reverie_entity,
                            },
                        ))
                        .id();

                    state.pending = Some(view_entity);
                }
            }
        }
    }
}

/// Swap a fully-materialized [`ReverieViewState::pending`] view entity: hide
/// anything visible, recompute the pending entity's `Transform` against the
/// *current* camera pose, and set it `Visibility::Visible` last.
///
/// "Fully materialized" means the typst pipeline has completely drained for
/// that entity. It either works or it doesn't and I gotta debug again.
///
/// Runs unconditionally every frame since readiness resolves asynchronously
/// over multiple ticks independent of any single [`ActiveReverie`] change.
#[allow(clippy::type_complexity)]
fn reveal_ready_reverie_view(
    mut state: ResMut<ReverieViewState>,
    ready_q: Query<
        Entity,
        (
            With<TypstReverieView>,
            Without<flan::typst_text::TypstDirty>,
            Without<flan::Text3dDirty>,
        ),
    >,
    camera_q: Query<(&GlobalTransform, &Projection), With<Camera3d>>,
    mut commands: Commands,
) {
    let Some(pending) = state.pending else {
        return;
    };

    if !ready_q.contains(pending) {
        // Still shaping/meshing/uploading try again next frame.
        return;
    }

    let Ok((cam_gt, proj)) = camera_q.single() else {
        bevy::log::warn!("reveal_ready_reverie_view: no single Camera3d found, skipping");
        return;
    };

    let transform = compute_reverie_transform(cam_gt, proj);

    if let Some(prev) = state.visible
        && prev != pending
    {
        bevy::log::trace!("reveal_ready_reverie_view: hiding previous view {:?}", prev);
        commands.entity(prev).insert(Visibility::Hidden);
    }

    bevy::log::trace!(
        "reveal_ready_reverie_view: revealing {:?} at {:?}",
        pending,
        transform.translation
    );
    commands
        .entity(pending)
        .insert((transform, Visibility::Visible));

    state.visible = Some(pending);
    state.pending = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_keys_non_empty() {
        for (keys, _, _) in REVERIE_DATA {
            assert!(!keys.is_empty(), "entry has empty keys slice");
            for k in *keys {
                assert!(!k.is_empty(), "entry has empty string in keys slice");
            }
        }
    }

    #[test]
    fn all_display_names_non_empty() {
        for (_, display, _) in REVERIE_DATA {
            assert!(!display.is_empty(), "entry has empty display name");
        }
    }

    #[test]
    fn all_content_non_empty() {
        for (keys, _, content) in REVERIE_DATA {
            assert!(
                !content.is_empty(),
                "reverie {:?} has empty content",
                keys[0]
            );
        }
    }

    #[test]
    fn canonical_keys_unique() {
        let mut seen = std::collections::HashSet::new();
        for (keys, _, _) in REVERIE_DATA {
            assert!(
                seen.insert(keys[0]),
                "duplicate canonical key {:?}",
                keys[0]
            );
        }
    }

    #[test]
    fn canonical_keys_sorted() {
        let keys: Vec<&str> = REVERIE_DATA.iter().map(|(k, _, _)| k[0]).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "REVERIE_DATA is not sorted by canonical key");
    }

    #[test]
    fn reverie_key_matches_canonical() {
        for (keys, _, _) in REVERIE_DATA {
            let rk = ReverieKey(keys);
            assert!(
                rk.matches(keys[0]),
                "ReverieKey::matches failed on its own canonical key {:?}",
                keys[0]
            );
        }
    }

    #[test]
    fn reverie_key_matches_case_insensitive() {
        for (keys, _, _) in REVERIE_DATA {
            let rk = ReverieKey(keys);
            assert!(
                rk.matches(&keys[0].to_uppercase()),
                "ReverieKey::matches must be case-insensitive for {:?}",
                keys[0]
            );
        }
    }

    #[test]
    fn reverie_key_no_match_on_garbage() {
        for (keys, _, _) in REVERIE_DATA {
            let rk = ReverieKey(keys);
            assert!(
                !rk.matches("__this_key_should_never_exist__"),
                "ReverieKey::matches returned true for garbage on {:?}",
                keys[0]
            );
        }
    }

    #[test]
    fn reverie_key_segments_nonempty() {
        for (keys, _, _) in REVERIE_DATA {
            let rk = ReverieKey(keys);
            let segs = rk.segments();
            assert!(!segs.is_empty());
            for seg in segs {
                assert!(!seg.is_empty(), "empty segment in key {:?}", keys[0]);
            }
        }
    }

    #[test]
    fn alias_keys_also_match() {
        for (keys, _, _) in REVERIE_DATA {
            if keys.len() > 1 {
                let rk = ReverieKey(keys);
                for alias in &keys[1..] {
                    assert!(
                        rk.matches(alias),
                        "alias {:?} on entry {:?} did not match",
                        alias,
                        keys[0]
                    );
                }
            }
        }
    }

    #[test]
    fn find_reverie_empty_iter_returns_none() {
        let result = find_reverie("anything", std::iter::empty());
        assert!(result.is_none());
    }

    #[test]
    fn find_reverie_matches_key() {
        let keys: &[&str] = &["some_ns/foo"];
        let key_comp = ReverieKey(keys);
        let display_comp = ReverieDisplayName("Foo");
        let entity = Entity::from_raw_u32(1).expect("test entity");

        let result = find_reverie(
            "some_ns/foo",
            std::iter::once((entity, &key_comp, &display_comp)),
        );
        assert_eq!(result, Some(entity));
    }

    #[test]
    fn find_reverie_matches_display_name() {
        let keys: &[&str] = &["lorem_ipsum"];
        let key_comp = ReverieKey(keys);
        let display_comp = ReverieDisplayName("Lorem Ipsum");
        let entity = Entity::from_raw_u32(2).expect("test entity");

        let result = find_reverie(
            "Lorem Ipsum",
            std::iter::once((entity, &key_comp, &display_comp)),
        );
        assert_eq!(result, Some(entity));
    }

    #[test]
    fn find_reverie_case_insensitive() {
        let keys: &[&str] = &["nova_aurora"];
        let key_comp = ReverieKey(keys);
        let display_comp = ReverieDisplayName("Nova Aurora");
        let entity = Entity::from_raw_u32(3).expect("test entity");

        assert_eq!(
            find_reverie(
                "NOVA_AURORA",
                std::iter::once((entity, &key_comp, &display_comp))
            ),
            Some(entity)
        );
        assert_eq!(
            find_reverie(
                "nova aurora",
                std::iter::once((entity, &key_comp, &display_comp))
            ),
            Some(entity),
            "display name match should also be case-insensitive"
        );
    }

    #[test]
    fn find_reverie_alias_resolves() {
        let keys: &[&str] = &["new_name", "old_name"];
        let key_comp = ReverieKey(keys);
        let display_comp = ReverieDisplayName("New Name");
        let entity = Entity::from_raw_u32(4).expect("test entity");

        // Both canonical and alias should resolve to the same entity.
        assert_eq!(
            find_reverie(
                "new_name",
                std::iter::once((entity, &key_comp, &display_comp))
            ),
            Some(entity)
        );
        assert_eq!(
            find_reverie(
                "old_name",
                std::iter::once((entity, &key_comp, &display_comp))
            ),
            Some(entity)
        );
    }

    #[test]
    fn find_reverie_unknown_returns_none() {
        let keys: &[&str] = &["lorem_ipsum"];
        let key_comp = ReverieKey(keys);
        let display_comp = ReverieDisplayName("Lorem Ipsum");
        let entity = Entity::from_raw_u32(5).expect("test entity");

        let result = find_reverie(
            "not_a_real_reverie",
            std::iter::once((entity, &key_comp, &display_comp)),
        );
        assert!(result.is_none());
    }
}
