use crate::fullscreen_effect::FullscreenEffectEnabled;
use crate::{CameraRotation, CubeRotation, DragState, FpsDisplay, HueAnimation};
use bevy::feathers::controls::{
    ColorPlane, ColorPlaneValue, ColorSwatch, ColorSwatchValue, color_plane, color_swatch,
};
use bevy::input::touch::TouchPhase;
use bevy::prelude::*;
use bevy::text::TextColor;
use bevy::ui::{
    AlignItems, BackgroundColor, BorderColor, FlexDirection, Interaction, JustifyContent, Node,
    PositionType, UiRect, Val, widget::Text,
};
use bevy::ui_widgets::{ValueChange, observe};

/// Marker component to indicate TV effect is enabled
#[allow(dead_code)]
#[derive(Component, Default)]
pub struct TvEffectEnabled;

/// Root component for the settings UI
#[derive(Component)]
pub struct SettingsUiRoot;

/// Marker components to identify specific toggle switches
#[derive(Component, Default)]
struct FullscreenToggle;

#[derive(Component, Default)]
struct FpsToggle;

#[derive(Component, Default)]
struct CameraToggle;

#[derive(Component, Default)]
struct CubeToggle;

#[derive(Component, Default)]
struct HueToggle;

#[derive(Component, Default)]
struct CloseMenuToggle;

/// Marker for the persistent color plane widget
#[derive(Component)]
struct PersistentColorPlane;

/// Marker component for the container that should hold the color plane
#[derive(Component)]
struct ColorPlaneContainer;

/// Resource to track the color plane entity
#[derive(Resource)]
struct ColorPlaneEntity(Entity);

/// Plugin for settings UI
pub struct SettingsUiPlugin;

impl Plugin for SettingsUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_ui).add_systems(
            Update,
            (
                toggle_ui_input,
                sync_toggle_to_marker::<FullscreenToggle, FullscreenEffectEnabled>,
                sync_toggle_to_marker::<FpsToggle, FpsDisplay>,
                sync_toggle_to_marker::<CameraToggle, CameraRotation>,
                sync_toggle_to_marker::<CubeToggle, CubeRotation>,
                sync_toggle_to_marker::<HueToggle, HueAnimation>,
                handle_close_menu_toggle,
                // Sync clear color to widgets (when not being interacted with)
                sync_widgets_from_clear_color,
                manage_color_plane_parenting,
                update_toggle_button_states,
            ),
        );
    }
}

/// Spawn marker entities for UI state and color plane
fn setup_ui(mut commands: Commands) {
    commands.spawn(CameraRotation);
    commands.spawn(CubeRotation);
    commands.spawn(HueAnimation);
    commands.spawn(FullscreenEffectEnabled);
    commands.spawn(FpsDisplay);

    // Spawn the persistent color plane once - positioned absolutely
    let plane_id = commands
        .spawn((
            color_plane(ColorPlane::RedGreen, ()),
            PersistentColorPlane,
            observe(handle_plane_value_change),
        ))
        .id();

    // Now update its Node to position it absolutely and add interaction
    commands.entity(plane_id).insert((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.),
            top: Val::Px(80.), // Below title and "Background Color" label
            width: Val::Px(200.),
            height: Val::Px(200.),
            border: UiRect::all(Val::Px(2.)),
            display: Display::None, // Hidden until UI opens
            ..default()
        },
        BorderColor::all(Color::srgb(0.5, 0.5, 0.5)),
        ZIndex(101), // Above UI panel
        Interaction::None,
    ));

    commands.insert_resource(ColorPlaneEntity(plane_id));
}

/// Unified UI toggle system handling keyboard, mouse, and touch input
/// Only toggles when input doesn't hit UI controls (prevents event propagation issues)
fn toggle_ui_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut touch_events: MessageReader<TouchInput>,
    ui_entity: Query<Entity, With<SettingsUiRoot>>,
    interaction_query: Query<&Interaction>,
    drag_state: Res<DragState>,
    mut commands: Commands,
) {
    // Check for keyboard toggle (always works, regardless of UI interaction)
    let keyboard_toggle = keyboard.just_pressed(KeyCode::KeyG);

    // Check if any UI element is currently being interacted with
    let ui_is_interacted = interaction_query
        .iter()
        .any(|interaction| *interaction != Interaction::None);

    // Check for mouse click (only toggle if not on UI and not a drag)
    // A click is only valid if the mouse was released and didn't move much (< 5 pixels)
    let mouse_toggle = mouse.just_released(MouseButton::Left)
        && !ui_is_interacted
        && drag_state.drag_distance < 5.0;

    // Check for touch tap (only toggle if not on UI)
    let touch_toggle = touch_events
        .read()
        .any(|event| event.phase == TouchPhase::Ended)
        && !ui_is_interacted;

    // Determine if we should toggle
    let should_toggle = keyboard_toggle || mouse_toggle || touch_toggle;

    if should_toggle {
        let ui_exists = !ui_entity.is_empty();

        if ui_exists {
            // Close the menu
            for entity in ui_entity.iter() {
                commands.entity(entity).despawn();
            }
        } else {
            // Open the menu
            spawn_settings_ui(&mut commands);
        }
    }
}

/// Spawn the complete settings UI hierarchy
fn spawn_settings_ui(commands: &mut Commands) {
    // Create root container
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(10.),
                top: Val::Px(10.),
                flex_direction: FlexDirection::Column,
                border: UiRect::all(Val::Px(2.)),
                padding: UiRect::all(Val::Px(10.)),
                row_gap: Val::Px(5.),
                ..default()
            },
            BorderColor::from(Color::srgb(0.3, 0.3, 0.3)),
            BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.9)),
            SettingsUiRoot,
        ))
        .id();

    // Title
    let title = commands
        .spawn((
            TextColor(Color::srgb(0.9, 0.9, 0.9)),
            Text("Settings".into()),
        ))
        .id();
    commands.entity(root).add_child(title);

    // Background Color label
    let label = commands
        .spawn((
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
            Text("Background Color".into()),
        ))
        .id();
    commands.entity(root).add_child(label);

    // Color Swatch - Try only setting width, let widget control its own height
    let swatch = commands
        .spawn((color_swatch(()), observe(handle_swatch_value_change)))
        .id();

    // Only set width to fill, let height be automatic
    commands.entity(swatch).insert(Node {
        width: Val::Percent(100.0),
        margin: UiRect::all(Val::Px(5.0)),
        ..default()
    });

    commands.entity(root).add_child(swatch);

    // Color Plane Container
    let container = commands
        .spawn((
            Node {
                width: Val::Px(200.),
                height: Val::Px(200.),
                margin: UiRect::all(Val::Px(5.)),
                ..default()
            },
            ColorPlaneContainer,
        ))
        .id();
    commands.entity(root).add_child(container);

    // Fullscreen effect toggle row
    spawn_fullscreen_toggle_row(commands, root, true);

    // FPS toggle row
    spawn_fps_toggle_row(commands, root, true);

    // Camera rotation toggle row
    spawn_camera_toggle_row(commands, root, true);

    // Cube rotation toggle row
    spawn_cube_toggle_row(commands, root, true);

    // Hue animation toggle row
    spawn_hue_toggle_row(commands, root, true);

    // Close menu toggle row
    spawn_close_menu_toggle(commands, root);
}

/// Helper to spawn fullscreen effect toggle row
fn spawn_fullscreen_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<FullscreenToggle>(
        commands,
        parent,
        "Fullscreen Effect [E]",
        enabled,
    );
}

/// Helper to spawn FPS toggle row
fn spawn_fps_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<FpsToggle>(commands, parent, "FPS Display [F]", enabled);
}

/// Helper to spawn camera toggle row
fn spawn_camera_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<CameraToggle>(commands, parent, "Camera Rotation [R]", enabled);
}

/// Helper to spawn cube toggle row
fn spawn_cube_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<CubeToggle>(commands, parent, "Cube Rotation [C]", enabled);
}

/// Helper to spawn hue toggle row
fn spawn_hue_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<HueToggle>(commands, parent, "Hue Animation [H]", enabled);
}

/// Generic toggle row spawner
fn spawn_toggle_row_generic<T: Component + Default>(
    commands: &mut Commands,
    parent: Entity,
    label_text: &str,
    enabled: bool,
) {
    let row = commands
        .spawn((Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.),
            ..default()
        },))
        .id();
    commands.entity(parent).add_child(row);

    // Toggle button
    let toggle_button = commands
        .spawn((
            Node {
                width: Val::Px(40.),
                height: Val::Px(20.),
                border: UiRect::all(Val::Px(2.)),
                justify_content: if enabled {
                    JustifyContent::End
                } else {
                    JustifyContent::Start
                },
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(2.)),
                ..default()
            },
            BorderColor::all(Color::srgb(0.5, 0.5, 0.5)),
            BackgroundColor(if enabled {
                Color::srgb(0.3, 0.8, 0.3)
            } else {
                Color::srgb(0.8, 0.3, 0.3)
            }),
            Interaction::None,
            T::default(),
        ))
        .id();
    commands.entity(row).add_child(toggle_button);

    // Toggle knob
    let knob = commands
        .spawn((
            Node {
                width: Val::Px(14.),
                height: Val::Px(14.),
                ..default()
            },
            BackgroundColor(Color::WHITE),
        ))
        .id();
    commands.entity(toggle_button).add_child(knob);

    // Label
    let label = commands
        .spawn((
            TextColor(Color::srgb(0.9, 0.9, 0.9)),
            Text(label_text.into()),
        ))
        .id();
    commands.entity(row).add_child(label);
}

/// Helper to spawn close menu toggle
fn spawn_close_menu_toggle(commands: &mut Commands, parent: Entity) {
    let row = commands
        .spawn((Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.),
            ..default()
        },))
        .id();
    commands.entity(parent).add_child(row);

    // Close menu button
    let toggle_button = commands
        .spawn((
            Node {
                width: Val::Px(40.),
                height: Val::Px(20.),
                border: UiRect::all(Val::Px(2.)),
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(2.)),
                justify_content: JustifyContent::Start,
                ..default()
            },
            BorderColor::all(Color::srgb(0.5, 0.5, 0.5)),
            BackgroundColor(Color::srgb(0.8, 0.3, 0.3)),
            Interaction::None,
            CloseMenuToggle,
        ))
        .id();
    commands.entity(row).add_child(toggle_button);

    // Toggle knob
    let knob = commands
        .spawn((
            Node {
                width: Val::Px(14.),
                height: Val::Px(14.),
                ..default()
            },
            BackgroundColor(Color::WHITE),
        ))
        .id();
    commands.entity(toggle_button).add_child(knob);

    // Label
    let label = commands
        .spawn((
            TextColor(Color::srgb(0.9, 0.9, 0.9)),
            Text("Close Menu [G]".into()),
        ))
        .id();
    commands.entity(row).add_child(label);
}

/// Sync toggle switch state to marker entities via click detection
fn sync_toggle_to_marker<T: Component, M>(
    toggle_query: Query<&Interaction, (With<T>, Changed<Interaction>)>,
    marker_query: Query<Entity, With<M>>,
    mut commands: Commands,
) where
    M: Bundle + Default + Component,
{
    for interaction in toggle_query.iter() {
        if *interaction == Interaction::Pressed {
            let marker_exists = !marker_query.is_empty();

            if marker_exists {
                if let Ok(entity) = marker_query.single() {
                    commands.entity(entity).despawn();
                }
            } else {
                commands.spawn(M::default());
            }
        }
    }
}

/// Handle close menu toggle click
fn handle_close_menu_toggle(
    toggle_query: Query<&Interaction, (With<CloseMenuToggle>, Changed<Interaction>)>,
    ui_entity: Query<Entity, With<SettingsUiRoot>>,
    mut commands: Commands,
) {
    for interaction in toggle_query.iter() {
        if *interaction == Interaction::Pressed {
            for entity in ui_entity.iter() {
                commands.entity(entity).despawn();
            }
        }
    }
}

/// Update visual state of toggle buttons based on marker presence
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
fn update_toggle_button_states(
    fullscreen_query: Query<Entity, With<FullscreenEffectEnabled>>,
    fps_query: Query<Entity, With<FpsDisplay>>,
    camera_query: Query<Entity, With<CameraRotation>>,
    cube_query: Query<Entity, With<CubeRotation>>,
    hue_query: Query<Entity, With<HueAnimation>>,
    mut fullscreen_toggle: Query<(&mut Node, &mut BackgroundColor), With<FullscreenToggle>>,
    mut fps_toggle: Query<
        (&mut Node, &mut BackgroundColor),
        (With<FpsToggle>, Without<FullscreenToggle>),
    >,
    mut camera_toggle: Query<
        (&mut Node, &mut BackgroundColor),
        (
            With<CameraToggle>,
            Without<FpsToggle>,
            Without<FullscreenToggle>,
        ),
    >,
    mut cube_toggle: Query<
        (&mut Node, &mut BackgroundColor),
        (
            With<CubeToggle>,
            Without<FpsToggle>,
            Without<CameraToggle>,
            Without<FullscreenToggle>,
        ),
    >,
    mut hue_toggle: Query<
        (&mut Node, &mut BackgroundColor),
        (
            With<HueToggle>,
            Without<FpsToggle>,
            Without<CameraToggle>,
            Without<CubeToggle>,
            Without<FullscreenToggle>,
        ),
    >,
) {
    let fullscreen_enabled = !fullscreen_query.is_empty();
    let fps_enabled = !fps_query.is_empty();
    let camera_enabled = !camera_query.is_empty();
    let cube_enabled = !cube_query.is_empty();
    let hue_enabled = !hue_query.is_empty();

    for (mut node, mut bg) in fullscreen_toggle.iter_mut() {
        node.justify_content = if fullscreen_enabled {
            JustifyContent::End
        } else {
            JustifyContent::Start
        };
        bg.0 = if fullscreen_enabled {
            Color::srgb(0.3, 0.8, 0.3)
        } else {
            Color::srgb(0.8, 0.3, 0.3)
        };
    }

    for (mut node, mut bg) in fps_toggle.iter_mut() {
        node.justify_content = if fps_enabled {
            JustifyContent::End
        } else {
            JustifyContent::Start
        };
        bg.0 = if fps_enabled {
            Color::srgb(0.3, 0.8, 0.3)
        } else {
            Color::srgb(0.8, 0.3, 0.3)
        };
    }

    for (mut node, mut bg) in camera_toggle.iter_mut() {
        node.justify_content = if camera_enabled {
            JustifyContent::End
        } else {
            JustifyContent::Start
        };
        bg.0 = if camera_enabled {
            Color::srgb(0.3, 0.8, 0.3)
        } else {
            Color::srgb(0.8, 0.3, 0.3)
        };
    }

    for (mut node, mut bg) in cube_toggle.iter_mut() {
        node.justify_content = if cube_enabled {
            JustifyContent::End
        } else {
            JustifyContent::Start
        };
        bg.0 = if cube_enabled {
            Color::srgb(0.3, 0.8, 0.3)
        } else {
            Color::srgb(0.8, 0.3, 0.3)
        };
    }

    for (mut node, mut bg) in hue_toggle.iter_mut() {
        node.justify_content = if hue_enabled {
            JustifyContent::End
        } else {
            JustifyContent::Start
        };
        bg.0 = if hue_enabled {
            Color::srgb(0.3, 0.8, 0.3)
        } else {
            Color::srgb(0.8, 0.3, 0.3)
        };
    }
}

/// Observer: Handle color plane value changes and update ClearColor
fn handle_plane_value_change(change: On<ValueChange<Vec2>>, mut clear_color: ResMut<ClearColor>) {
    // ColorPlane::RedGreen uses x=red, y=green
    // We'll keep the existing blue component
    let srgba = clear_color.0.to_srgba();
    let new_color = Color::srgb(change.value.x, change.value.y, srgba.blue);
    trace!(
        "ColorPlane value changed to: {:?}, updating ClearColor to: {:?}",
        change.value, new_color
    );
    clear_color.0 = new_color;
}

/// Observer: Handle color swatch value changes and update ClearColor
fn handle_swatch_value_change(change: On<ValueChange<Color>>, mut clear_color: ResMut<ClearColor>) {
    info!("ColorSwatch value changed to: {:?}", change.value);
    clear_color.0 = change.value;
}

/// Sync widgets from ClearColor when it changes (but not during user interaction)
fn sync_widgets_from_clear_color(
    clear_color: Res<ClearColor>,
    mut swatches: Query<(&mut ColorSwatchValue, &Interaction), With<ColorSwatch>>,
    mut planes: Query<(&mut ColorPlaneValue, &Interaction), With<ColorPlane>>,
) {
    if !clear_color.is_changed() {
        return;
    }

    let srgba = clear_color.0.to_srgba();

    // Update swatch (only if not being interacted with)
    for (mut swatch_value, interaction) in swatches.iter_mut() {
        if *interaction == Interaction::None {
            swatch_value.0 = clear_color.0;
        }
    }

    // Update color plane - always update to keep visual indicator in sync
    for (mut plane_value, _interaction) in planes.iter_mut() {
        // ColorPlane::RedGreen: x=red, y=green, z=blue (fixed third component)
        plane_value.0 = Vec3::new(srgba.red, srgba.green, srgba.blue);
    }
}

/// Manage color plane visibility based on whether container exists
fn manage_color_plane_parenting(
    plane_res: Res<ColorPlaneEntity>,
    containers: Query<(), With<ColorPlaneContainer>>,
    mut plane: Query<&mut Node, With<PersistentColorPlane>>,
) {
    let plane_entity = plane_res.0;

    if let Ok(mut node) = plane.get_mut(plane_entity) {
        // Show plane when container exists, hide when it doesn't
        node.display = if !containers.is_empty() {
            Display::Flex
        } else {
            Display::None
        };
    }
}
