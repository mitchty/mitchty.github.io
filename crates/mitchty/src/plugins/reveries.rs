use bevy::{
    input::mouse::{MouseScrollUnit, MouseWheel},
    picking::hover::HoverMap,
    prelude::*,
};
use markdown::{ParseOptions, mdast::Node as MdNode, to_mdast};

use crate::plugins::{PluginEnabled, PluginRegistry, run_if_enabled};

// Build.rs generated data from the reveries dir markdown files.
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

/// Raw markdown content of the reverie file, embedded at compile time. (for now, I might make this dynamic in future)
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
            .add_systems(Update, sync_scroll_view_visibility.in_set(ReveriesSystems))
            .add_observer(on_scroll);

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

/// Marker on the single Bevy UI node rendering the active reverie.
/// Stores the reverie entity it currently displays to detect changes.
#[derive(Component)]
pub struct LoremScrollView {
    pub reverie: Entity,
}

const LINE_HEIGHT: f32 = 20.0;

/// Spawn, swap, or despawn the scroll-view node when `ActiveReverie` changes.
fn sync_scroll_view_visibility(
    active: Res<ActiveReverie>,
    content_q: Query<&ReverieContent>,
    view_q: Query<(Entity, &LoremScrollView)>,
    mut commands: Commands,
) {
    if !active.is_changed() {
        return;
    }

    match active.0 {
        None => {
            if let Ok((view_entity, _)) = view_q.single() {
                commands.entity(view_entity).despawn();
            }
        }
        Some(reverie_entity) => {
            let content = content_q
                .get(reverie_entity)
                .map(|c| c.0)
                .unwrap_or_default();

            if let Ok((view_entity, shown)) = view_q.single() {
                if shown.reverie != reverie_entity {
                    commands.entity(view_entity).despawn();
                    spawn_scroll_view(&mut commands, reverie_entity, content);
                }
                // same entity -> already showing the right content
            } else {
                spawn_scroll_view(&mut commands, reverie_entity, content);
            }
        }
    }
}

#[derive(EntityEvent, Debug)]
#[entity_event(propagate, auto_propagate)]
struct Scroll {
    entity: Entity,
    delta: Vec2,
}

fn on_scroll(mut ev: On<Scroll>, mut query: Query<(&mut ScrollPosition, &Node, &ComputedNode)>) {
    let Ok((mut pos, node, computed)) = query.get_mut(ev.entity) else {
        return;
    };

    let max_offset = (computed.content_size() - computed.size()) * computed.inverse_scale_factor();
    let delta = &mut ev.delta;

    if node.overflow.x == OverflowAxis::Scroll && delta.x != 0.0 {
        let at_max = if delta.x > 0.0 {
            pos.x >= max_offset.x
        } else {
            pos.x <= 0.0
        };
        if !at_max {
            pos.x += delta.x;
            delta.x = 0.0;
        }
    }

    if node.overflow.y == OverflowAxis::Scroll && delta.y != 0.0 {
        let at_max = if delta.y > 0.0 {
            pos.y >= max_offset.y
        } else {
            pos.y <= 0.0
        };
        if !at_max {
            pos.y += delta.y;
            delta.y = 0.0;
        }
    }

    if *delta == Vec2::ZERO {
        ev.propagate(false);
    }
}

pub fn send_scroll_events(
    mut wheel: MessageReader<MouseWheel>,
    hover_map: Res<HoverMap>,
    keyboard: Res<ButtonInput<KeyCode>>,
    scroll_view: Query<Entity, With<LoremScrollView>>,
    mut commands: Commands,
) {
    for ev in wheel.read() {
        let mut delta = -Vec2::new(ev.x, ev.y);
        if ev.unit == MouseScrollUnit::Line {
            delta *= LINE_HEIGHT;
        }
        if keyboard.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
            std::mem::swap(&mut delta.x, &mut delta.y);
        }

        let mut dispatched = false;
        for pointer_map in hover_map.values() {
            for &entity in pointer_map.keys() {
                commands.trigger(Scroll { entity, delta });
                dispatched = true;
            }
        }

        if !dispatched && let Ok(entity) = scroll_view.single() {
            commands.trigger(Scroll { entity, delta });
        }
    }
}

fn spawn_scroll_view(commands: &mut Commands, reverie: Entity, content: &str) {
    let sections = parse_markdown_sections(content);

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(40.0),
                bottom: Val::Px(0.0),
                left: Val::Px(210.0),
                right: Val::Px(0.0),
                overflow: Overflow::scroll_y(),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(12.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.04, 0.06, 0.88)),
            ScrollPosition::default(),
            LoremScrollView { reverie },
        ))
        .with_children(|p| {
            for (heading, body) in &sections {
                p.spawn((
                    Text::new(heading.clone()),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.75, 0.35)),
                ));
                p.spawn((
                    Text::new(body.clone()),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.88, 0.88, 0.84)),
                    Node {
                        margin: UiRect::bottom(Val::Px(6.0)),
                        ..default()
                    },
                ));
            }
        });
}

// Markdown stuff, probably better yeeted in the lib crate in future.

fn inline_text(node: &MdNode) -> String {
    match node {
        MdNode::Text(t) => t.value.clone(),
        MdNode::InlineCode(c) => c.value.clone(),
        MdNode::Strong(s) => s.children.iter().map(inline_text).collect(),
        MdNode::Emphasis(e) => e.children.iter().map(inline_text).collect(),
        MdNode::Delete(d) => d.children.iter().map(inline_text).collect(),
        MdNode::Link(l) => l.children.iter().map(inline_text).collect(),
        _ => String::new(),
    }
}

fn parse_markdown_sections(src: &str) -> Vec<(String, String)> {
    let Ok(root) = to_mdast(src, &ParseOptions::default()) else {
        return Vec::new();
    };

    let MdNode::Root(root) = root else {
        return Vec::new();
    };

    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_body = String::new();

    for node in &root.children {
        match node {
            MdNode::Heading(h) => {
                if let Some(heading) = current_heading.take() {
                    let body = current_body.trim().to_string();
                    if !body.is_empty() {
                        sections.push((heading, body));
                    }
                    current_body.clear();
                }
                current_heading = Some(h.children.iter().map(inline_text).collect::<String>());
            }
            MdNode::Paragraph(para) if current_heading.is_some() => {
                if !current_body.is_empty() {
                    current_body.push(' ');
                }
                current_body.push_str(&para.children.iter().map(inline_text).collect::<String>());
            }
            _ => {}
        }
    }

    if let Some(heading) = current_heading {
        let body = current_body.trim().to_string();
        if !body.is_empty() {
            sections.push((heading, body));
        }
    }

    sections
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
