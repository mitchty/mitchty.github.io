//! Initial help overlay plugin, this is just a pretty text overlay.
//!
//! Spawns a brief touch/click/keyboard hint overlay on startup and dismisses
//! it on the first input event.
// TODO: Do I even keep this? The dep adds a lot of useless code....

use bevy::prelude::*;

/// Marker for the initial help overlay entity.
#[derive(Component)]
pub struct DisplayInitialHelp;

/// Startup: spawn the touch/click/keyboard help overlay.
pub fn setup_help_text(mut commands: Commands) {
    commands.spawn((
        Text::new("Touch and pan to rotate"),
        TextFont {
            font_size: bevy::text::FontSize::Px(22.0),
            ..default()
        },
        TextColor(Color::WHITE),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(80.0),
            left: Val::Percent(10.0),
            top: Val::Percent(50.0),
            ..default()
        },
        DisplayInitialHelp,
    ));
}

/// Despawn the help overlay on the first touch or mouse-click.
pub fn dismiss_help_on_input(
    mut touch_events: MessageReader<TouchInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    help_query: Query<Entity, With<DisplayInitialHelp>>,
    mut commands: Commands,
) {
    let touched = touch_events.read().next().is_some();
    let clicked = mouse.get_just_pressed().next().is_some();

    if touched || clicked {
        for entity in help_query.iter() {
            commands.entity(entity).despawn();
        }
    }
}

pub struct HelpPlugin;

impl Plugin for HelpPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_help_text)
            .add_systems(Update, dismiss_help_on_input);
    }
}
