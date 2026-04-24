use bevy::ecs::message::MessageReader;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use std::collections::VecDeque;

use crate::plugins::{PluginEnabled, PluginRegistry, run_if_enabled};

/// Marker component for the terminal's own UI text node, for now only one.
#[derive(Component)]
pub struct TerminalText;

/// Terminal backend, initial functions only
// TODO: alacritty_terminal in future? Can I use that with wasm?
pub trait TerminalBackend: Send + Sync + 'static {
    fn write(&mut self, input: &[u8]);
    fn read(&mut self) -> Vec<u8>;
    // TODO: future...
    #[allow(dead_code)]
    fn resize(&mut self, _cols: u16, _rows: u16) {}
}

/// Fake backend for just testing purposes not complete at all nor it it
/// intended to be.
#[derive(Default)]
pub struct EchoBackend {
    buffer: VecDeque<u8>,
}

impl TerminalBackend for EchoBackend {
    fn write(&mut self, input: &[u8]) {
        for b in input {
            self.buffer.push_back(*b);
        }
    }

    fn read(&mut self) -> Vec<u8> {
        self.buffer.drain(..).collect()
    }
}

#[derive(Clone)]
pub struct Cell {
    pub ch: char,
}

#[derive(Resource)]
pub struct TerminalState {
    pub cells: Vec<Vec<Cell>>, // rows x cols
    pub cols: usize,
    pub rows: usize,
    /// Current cursor column (0-based).
    pub cursor_col: usize,
    /// Current cursor row (0-based).
    pub cursor_row: usize,
}

impl TerminalState {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![vec![Cell { ch: ' ' }; cols]; rows],
            cursor_col: 0,
            cursor_row: 0,
        }
    }

    /// Scroll the entire grid up by one row, clearing the bottom row.
    fn scroll_up(&mut self) {
        self.cells.rotate_left(1);
        let last = self.rows - 1;
        for cell in &mut self.cells[last] {
            cell.ch = ' ';
        }
    }

    /// Advance the cursor to the start of the next line, scrolling if needed.
    fn newline(&mut self) {
        self.cursor_col = 0;
        self.cursor_row += 1;
        if self.cursor_row >= self.rows {
            self.scroll_up();
            self.cursor_row = self.rows - 1;
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match b {
                b'\n' | b'\r' => {
                    self.newline();
                }
                0x7f | 0x08 => {
                    // backspace: erase previous character
                    if self.cursor_col > 0 {
                        self.cursor_col -= 1;
                        self.cells[self.cursor_row][self.cursor_col].ch = ' ';
                    }
                }
                _ => {
                    let ch = b as char;
                    self.cells[self.cursor_row][self.cursor_col] = Cell { ch };
                    self.cursor_col += 1;
                    if self.cursor_col >= self.cols {
                        self.newline();
                    }
                }
            }
        }
    }
}

#[derive(Resource)]
pub struct TerminalEngine {
    pub backend: Box<dyn TerminalBackend>,
}

impl TerminalEngine {
    pub fn new(backend: Box<dyn TerminalBackend>) -> Self {
        Self { backend }
    }
}

/// `SystemSet` that gates all per-frame systems owned by [`TerminalPlugin`].
///
/// Controlled by `PluginEnabled::<TerminalPlugin>`. Enabled by default for
/// debugging — toggle with the backtick key at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub struct TerminalSystems;

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PluginEnabled::<TerminalPlugin>::default())
            .configure_sets(
                Update,
                TerminalSystems.run_if(run_if_enabled::<TerminalPlugin>()),
            )
            .insert_resource(TerminalEngine::new(Box::new(EchoBackend::default())))
            .insert_resource(TerminalState::new(80, 24));

        app.world_mut()
            .resource_mut::<TerminalState>()
            .push_bytes(b"terminal stub ready\n` to toggle\n\n");

        app.add_systems(Startup, spawn_terminal_ui)
            // Backtick toggles the terminal open/closed unconditionally for
            // now, future me can figure out how I can pass this in as data to
            // ssh/real ptys.
            .add_systems(Update, terminal_toggle_system)
            .add_systems(Update, terminal_visibility_system)
            .add_systems(
                Update,
                (
                    terminal_input_system,
                    terminal_backend_system,
                    terminal_render_system,
                )
                    .chain()
                    .in_set(TerminalSystems),
            );

        if let Some(mut registry) = app.world_mut().get_resource_mut::<PluginRegistry>() {
            registry.register::<TerminalPlugin>("Terminal", false);
        }
    }
}

/// Spawn the terminal overlay UI node, hidden by default.
///
/// Uses a fixed-width font so the cell grid renders correctly.
/// Visibility is driven by [`terminal_visibility_system`] rather than
/// the `TerminalSystems` gate so the node exists in the world at all times.
fn spawn_terminal_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
    use crate::assets::asset_path;

    let font = asset_server.load(asset_path("fonts/FiraMono-Medium.ttf"));

    commands.spawn((
        Text::new(""),
        TextFont {
            font,
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgb(0.0, 1.0, 0.0)),
        TextLayout::new_with_linebreak(bevy::text::LineBreak::NoWrap),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            // 70% wide centered as well
            width: Val::Percent(70.0),
            height: Val::Percent(70.0),
            left: Val::Percent(15.0),
            top: Val::Percent(15.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        Visibility::Hidden,
        TerminalText,
    ));
}

/// Toggles the terminal open/closed with the backtick key.
fn terminal_toggle_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut enabled: ResMut<PluginEnabled<TerminalPlugin>>,
    mut registry: ResMut<PluginRegistry>,
) {
    if keys.just_pressed(KeyCode::Backquote) {
        enabled.toggle();
        let now = enabled.is_enabled();
        if let Some(entry) = registry.entries.iter_mut().find(|e| e.name == "Terminal") {
            entry.enabled = now;
        }
    }
}

/// Show or hide the terminal overlay in sync with `PluginEnabled<TerminalPlugin>` SystemSet.
///
/// Runs every frame outside of `TerminalSystems` so toggling the registry
/// entry takes effect immediately even when the plugin was just disabled.
fn terminal_visibility_system(
    enabled: Option<Res<PluginEnabled<TerminalPlugin>>>,
    mut query: Query<&mut Visibility, With<TerminalText>>,
) {
    let want_visible = enabled.map(|r| r.enabled).unwrap_or(false);
    let target = if want_visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut vis in &mut query {
        if *vis != target {
            *vis = target;
        }
    }
}

/// Converts Bevy input into raw bytes.
fn terminal_input_system(
    mut evr_kbd: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut engine: ResMut<TerminalEngine>,
) {
    // text input skip the backtick so it doesn't echo when toggling the terminal
    // TODO: Future me needs to figure out how I deal with hiding the terminal
    // text ui after its added.
    for ev in evr_kbd.read() {
        if ev.key_code == KeyCode::Backquote {
            continue;
        }
        if let Some(text) = &ev.text {
            engine.backend.write(text.as_bytes());
        }
    }

    if keys.just_pressed(KeyCode::Enter) {
        engine.backend.write(b"\n");
    }

    if keys.just_pressed(KeyCode::Backspace) {
        engine.backend.write(&[0x7f]);
    }

    // arrows to ANSI escape sequences, need a real terminal emulator
    if keys.just_pressed(KeyCode::ArrowUp) {
        engine.backend.write(b"\x1b[A");
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        engine.backend.write(b"\x1b[B");
    }
}

/// Pull backend output and feed into the "terminal".
fn terminal_backend_system(mut engine: ResMut<TerminalEngine>, mut state: ResMut<TerminalState>) {
    let output = engine.backend.read();
    if !output.is_empty() {
        state.push_bytes(&output);
    }
}

/// VERY basic renderer for just a single Text entity
fn terminal_render_system(
    state: Res<TerminalState>,
    mut query: Query<&mut Text, With<TerminalText>>,
) {
    if !state.is_changed() {
        return;
    }

    let mut text = String::new();

    for row in &state.cells {
        for cell in row {
            text.push(cell.ch);
        }
        text.push('\n');
    }

    for mut t in &mut query {
        **t = text.clone();
    }
}
