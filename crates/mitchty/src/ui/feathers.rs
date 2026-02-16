use crate::fullscreen_effect::FullscreenEffectEnabled;
use crate::{CameraRotation, ColorState, CubeRotation, DragState, FpsDisplay, HueAnimation};
use bevy::{
    color::Hsla,
    feathers::controls::{
        ColorChannel, ColorPlane, ColorPlaneValue, ColorSlider, ColorSliderProps, ColorSwatch,
        ColorSwatchValue, SliderBaseColor, color_plane, color_slider, color_swatch,
    },
    input::touch::TouchPhase,
    prelude::*,
    text::TextColor,
    ui::{
        AlignItems, BackgroundColor, BorderColor, FlexDirection, Interaction, JustifyContent, Node,
        PositionType, UiRect, Val, widget::Text,
    },
    ui_widgets::{ValueChange, observe},
};

/// Marker component to indicate TV effect is enabled disabled for now, I should
/// port the wgsl shader to full screen.
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
                sync_color_sliders_from_clear_color,
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
#[allow(clippy::too_many_arguments)]
fn toggle_ui_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut touch_events: MessageReader<TouchInput>,
    ui_entity: Query<Entity, With<SettingsUiRoot>>,
    interaction_query: Query<&Interaction>,
    ui_root_interaction: Query<&Interaction, With<SettingsUiRoot>>,
    drag_state: Res<DragState>,
    color_state: Res<ColorState>,
    mut commands: Commands,
) {
    let keyboard_toggle = keyboard.just_pressed(KeyCode::KeyG);

    let ui_is_interacted = if let Ok(root_interaction) = ui_root_interaction.single() {
        *root_interaction != Interaction::None
    } else {
        interaction_query
            .iter()
            .any(|interaction| *interaction != Interaction::None)
    };

    // A click is only valid if the mouse was released and didn't move much about 5 pixels
    let mouse_toggle = mouse.just_released(MouseButton::Left)
        && !ui_is_interacted
        && drag_state.drag_distance < 5.0;

    let touch_toggle = touch_events
        .read()
        .any(|event| event.phase == TouchPhase::Ended)
        && !ui_is_interacted;

    let should_toggle = keyboard_toggle || mouse_toggle || touch_toggle;

    if should_toggle {
        let ui_exists = !ui_entity.is_empty();

        if ui_exists {
            // Close the menu
            for entity in ui_entity.iter() {
                commands.entity(entity).despawn();
            }
        } else {
            // Open the menu with current background color
            spawn_settings_ui(&mut commands, Color::from(color_state.color));
        }
    }
}

/// Spawn the complete settings UI hierarchy
fn spawn_settings_ui(commands: &mut Commands, current_color: Color) {
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
            Interaction::None,
            SettingsUiRoot,
        ))
        .id();

    let title = commands
        .spawn((
            TextColor(Color::srgb(0.9, 0.9, 0.9)),
            Text("Settings".into()),
        ))
        .id();
    commands.entity(root).add_child(title);

    let label = commands
        .spawn((
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
            Text("Background Color".into()),
        ))
        .id();
    commands.entity(root).add_child(label);

    let swatch = commands
        .spawn((color_swatch(()), observe(handle_swatch_value_change)))
        .id();

    commands.entity(swatch).insert(Node {
        width: Val::Percent(100.0),
        margin: UiRect::all(Val::Px(5.0)),
        ..default()
    });

    commands.entity(root).add_child(swatch);

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

    let hsla = Hsla::from(current_color);

    let hue_label = commands
        .spawn((TextColor(Color::srgb(0.8, 0.8, 0.8)), Text("Hue".into())))
        .id();
    commands.entity(root).add_child(hue_label);

    let hue_slider = commands
        .spawn((
            color_slider(
                ColorSliderProps {
                    value: hsla.hue,
                    channel: ColorChannel::HslHue,
                },
                (),
            ),
            observe(handle_hsl_hue_change),
        ))
        .id();

    commands
        .entity(hue_slider)
        .insert(SliderBaseColor(current_color));
    commands.entity(root).add_child(hue_slider);

    let saturation_label = commands
        .spawn((
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
            Text("Saturation".into()),
        ))
        .id();
    commands.entity(root).add_child(saturation_label);

    let saturation_slider = commands
        .spawn((
            color_slider(
                ColorSliderProps {
                    value: hsla.saturation,
                    channel: ColorChannel::HslSaturation,
                },
                (),
            ),
            observe(handle_hsl_saturation_change),
        ))
        .id();

    commands
        .entity(saturation_slider)
        .insert(SliderBaseColor(current_color));
    commands.entity(root).add_child(saturation_slider);

    let lightness_label = commands
        .spawn((
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
            Text("Lightness".into()),
        ))
        .id();
    commands.entity(root).add_child(lightness_label);

    let lightness_slider = commands
        .spawn((
            color_slider(
                ColorSliderProps {
                    value: hsla.lightness,
                    channel: ColorChannel::HslLightness,
                },
                (),
            ),
            observe(handle_hsl_lightness_change),
        ))
        .id();

    commands
        .entity(lightness_slider)
        .insert(SliderBaseColor(current_color));
    commands.entity(root).add_child(lightness_slider);

    spawn_fullscreen_toggle_row(commands, root, true);
    spawn_fps_toggle_row(commands, root, true);
    spawn_camera_toggle_row(commands, root, true);
    spawn_cube_toggle_row(commands, root, true);
    spawn_hue_toggle_row(commands, root, true);
    spawn_close_menu_toggle(commands, root);
}

fn spawn_fullscreen_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<FullscreenToggle>(
        commands,
        parent,
        "Fullscreen Effect [E]",
        enabled,
    );
}

fn spawn_fps_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<FpsToggle>(commands, parent, "FPS Display [F]", enabled);
}

fn spawn_camera_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<CameraToggle>(commands, parent, "Camera Rotation [R]", enabled);
}

fn spawn_cube_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<CubeToggle>(commands, parent, "Cube Rotation [C]", enabled);
}

fn spawn_hue_toggle_row(commands: &mut Commands, parent: Entity, enabled: bool) {
    spawn_toggle_row_generic::<HueToggle>(commands, parent, "Hue Animation [H]", enabled);
}

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

/// Observer: Handle color plane value changes and update ColorState
fn handle_plane_value_change(change: On<ValueChange<Vec2>>, mut color_state: ResMut<ColorState>) {
    // ColorPlane::RedGreen uses x=red, y=green
    // We'll keep the existing blue component until I learn how this works.
    let new_color = bevy::color::Srgba::rgb(change.value.x, change.value.y, color_state.color.blue);
    trace!("colorplane: {:?}, state: {:?}", change.value, new_color);
    color_state.color = new_color;
}

/// Observer: Handle color swatch value changes and update ColorState
fn handle_swatch_value_change(change: On<ValueChange<Color>>, mut color_state: ResMut<ColorState>) {
    trace!("colorswatch: {:?}", change.value);
    color_state.color = change.value.to_srgba();
}

/// Observer: Handle HSL Hue slider changes
fn handle_hsl_hue_change(change: On<ValueChange<f32>>, mut color_state: ResMut<ColorState>) {
    let mut hsla = Hsla::from(Color::from(color_state.color));
    hsla.hue = change.value;
    color_state.color = Color::from(hsla).to_srgba();
    trace!("hsl hue changed to: {}", change.value);
}

/// Observer: Handle HSL Saturation slider changes
fn handle_hsl_saturation_change(change: On<ValueChange<f32>>, mut color_state: ResMut<ColorState>) {
    let mut hsla = Hsla::from(Color::from(color_state.color));
    hsla.saturation = change.value;
    color_state.color = Color::from(hsla).to_srgba();
    trace!("hsl saturation changed to: {}", change.value);
}

/// Observer: Handle HSL Lightness slider changes
fn handle_hsl_lightness_change(change: On<ValueChange<f32>>, mut color_state: ResMut<ColorState>) {
    let mut hsla = Hsla::from(Color::from(color_state.color));
    hsla.lightness = change.value;
    color_state.color = Color::from(hsla).to_srgba();
    trace!("hsl lightness changed to: {}", change.value);
}

/// Sync widgets from ColorState when it changes
fn sync_widgets_from_clear_color(
    color_state: Res<ColorState>,
    mut swatches: Query<(&mut ColorSwatchValue, &Interaction), With<ColorSwatch>>,
    mut planes: Query<(&mut ColorPlaneValue, &Interaction), With<ColorPlane>>,
) {
    if !color_state.is_changed() {
        return;
    }

    for (mut swatch_value, interaction) in swatches.iter_mut() {
        if *interaction == Interaction::None {
            swatch_value.0 = Color::from(color_state.color);
        }
    }

    for (mut plane_value, _interaction) in planes.iter_mut() {
        // ColorPlane::RedGreen: x=red, y=green, z=blue (fixed third component)
        plane_value.0 = Vec3::new(
            color_state.color.red,
            color_state.color.green,
            color_state.color.blue,
        );
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

/// Sync color sliders from the color state
fn sync_color_sliders_from_clear_color(
    color_state: Res<ColorState>,
    mut sliders: Query<(
        Entity,
        &ColorSlider,
        &mut SliderBaseColor,
        Option<&Interaction>,
    )>,
    mut commands: Commands,
) {
    if !color_state.is_changed() {
        return;
    }

    let hsla = Hsla::from(Color::from(color_state.color));

    for (entity, slider, mut base_color, interaction) in sliders.iter_mut() {
        base_color.0 = Color::from(color_state.color);

        let is_being_interacted = interaction.is_some_and(|i| *i != Interaction::None);
        if !is_being_interacted {
            match slider.channel {
                ColorChannel::HslHue => {
                    commands
                        .entity(entity)
                        .insert(bevy::ui_widgets::SliderValue(hsla.hue));
                }
                ColorChannel::HslSaturation => {
                    commands
                        .entity(entity)
                        .insert(bevy::ui_widgets::SliderValue(hsla.saturation));
                }
                ColorChannel::HslLightness => {
                    commands
                        .entity(entity)
                        .insert(bevy::ui_widgets::SliderValue(hsla.lightness));
                }
                _ => {}
            }
        }
    }
}
