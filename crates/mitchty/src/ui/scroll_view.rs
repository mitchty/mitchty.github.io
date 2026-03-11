use bevy::{
    input::mouse::{MouseScrollUnit, MouseWheel},
    picking::hover::HoverMap,
    prelude::*,
};
use markdown::{ParseOptions, mdast::Node as MdNode, to_mdast};

pub struct ScrollViewPlugin;

impl Plugin for ScrollViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActivePost>()
            .add_systems(Update, sync_scroll_view_visibility)
            .add_observer(on_scroll);
    }
}

/// For egui menu items
pub struct PostEntry {
    pub name: &'static str,
    pub content: &'static str,
}

/// All available posts in menu order... for now
pub const POSTS: &[PostEntry] = &[
    PostEntry {
        name: "Nova Aurora",
        content: NOVA_AURORA_CONTENT,
    },
    PostEntry {
        name: "Lorem Ipsum",
        content: LOREM_IPSUM_CONTENT,
    },
];

/// Active post Resource as only one "post" can be shown at a time so no component
#[derive(Resource, Default)]
pub struct ActivePost(pub Option<usize>);

/// Use a Component for the post "index"
#[derive(Component)]
pub struct LoremScrollView {
    pub post_index: usize,
}

const LINE_HEIGHT: f32 = 20.0;

/// Set the ActivePost, None is nothing to show, Some(idx) is the post to put in
/// the scrollview.
fn sync_scroll_view_visibility(
    active_post: Res<ActivePost>,
    view: Query<(Entity, &LoremScrollView)>,
    mut commands: Commands,
) {
    if !active_post.is_changed() {
        return;
    }

    match active_post.0 {
        None => {
            if let Ok((entity, _)) = view.single() {
                commands.entity(entity).despawn();
            }
        }
        Some(idx) => {
            if let Ok((entity, shown)) = view.single() {
                if shown.post_index != idx {
                    commands.entity(entity).despawn();
                    spawn_scroll_view(&mut commands, idx);
                }
            } else {
                spawn_scroll_view(&mut commands, idx);
            }
        }
    }
}

/// Scroll event that propagates up the UI hierarchy
#[derive(EntityEvent, Debug)]
#[entity_event(propagate, auto_propagate)]
struct Scroll {
    entity: Entity,
    delta: Vec2,
}

/// For now, all scroll events get yeeted to the scroll view if its visible.
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

        // Pointer is outside of the scroll view but yeet the event there anyway
        if !dispatched && let Ok(entity) = scroll_view.single() {
            commands.trigger(Scroll { entity, delta });
        }
    }
}

fn spawn_scroll_view(commands: &mut Commands, post_index: usize) {
    let content = POSTS.get(post_index).map(|p| p.content).unwrap_or_default();
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
            LoremScrollView { post_index },
        ))
        .with_children(|p| {
            for (heading, body) in &sections {
                // Section heading aka # for now
                p.spawn((
                    Text::new(heading.clone()),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.75, 0.35)),
                ));
                // Body paragraph aka no # for now
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

/// Get the raw text from the mdast node for abuse elsewhere.
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

/// For now, this just "parses" commonmark into (heading, body) tuples.
///
/// I'll expand markdown crap later. This is an MVP/POC.
///
/// Any other ast node that isn't a heading or text is basically skipped.
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
                let text: String = para.children.iter().map(inline_text).collect();
                current_body.push_str(&text);
            }
            // every other markdown ast type can eff off this is all I need for now
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

/// Nova Aurora, new beginnings to me trying to act like I am creative
const NOVA_AURORA_CONTENT: &str = "\
# Nova Aurora

Nova Aurora, latin for roughly new beginnings. I decided to take this experiment into the realm of a what if... and to do everything entirely differently than I ever have before.

The text you're reading is technically written in markdown, commonmark, and parsed into bevy ui text nodes and rendered all via wgpu. That is, there is no css nor html here. This is all a real 3d rendering system with 2d overlays and other shenanigans.

It also is \"the same\" code running in the browser as well as on macos/windows. Though the caveat with this statement is the \"web\" version is really a wasm binary running in a browser.

# BUT WHYYYYYYYY?

Because I'm obstinate. But I also had an idea harkening back to the olde days where flash existed and thought, maybe blogs can happen again but this time, I can make them interactive and be unshackled from the past.

At least that is my plan. This entire thing is a bit of a playground for doing everything \"wrong\" and seeing where that gets me.
";

/// Here for "the future" and to act like a reference for whatever a post will become
const LOREM_IPSUM_CONTENT: &str = "\
# Lorem Ipsum

Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor \
incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud \
exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure \
dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur.

# De Finibus Bonorum

Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque \
laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi \
architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas \
sit aspernatur aut odit aut fugit.

# At vero eos

At vero eos et accusamus et iusto odio dignissimos ducimus qui blanditiis praesentium \
voluptatum deleniti atque corrupti quos dolores et quas molestias excepturi sint \
occaecati cupiditate non provident, similique sunt in culpa qui officia deserunt \
mollitia animi, id est laborum et dolorum fuga.

# Nam libero tempore

Nam libero tempore, cum soluta nobis est eligendi optio cumque nihil impedit quo minus \
id quod maxime placeat facere possimus, omnis voluptas assumenda est, omnis dolor \
repellendus. Temporibus autem quibusdam et aut officiis debitis aut rerum necessitatibus \
saepe eveniet ut et voluptates repudiandae sint et molestiae non recusandae.

# Quis autem vel

Quis autem vel eum iure reprehenderit qui in ea voluptate velit esse quam nihil molestiae \
consequatur, vel illum qui dolorem eum fugiat quo voluptas nulla pariatur? Neque porro \
quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed \
quia non numquam eius modi tempora incidunt.

# Itaque earum rerum

Itaque earum rerum hic tenetur a sapiente delectus, ut aut reiciendis voluptatibus \
maiores alias consequatur aut perferendis doloribus asperiores repellat. Et harum \
quidem rerum facilis est et expedita distinctio. Nam libero tempore, cum soluta nobis \
est eligendi optio cumque nihil impedit.
";
