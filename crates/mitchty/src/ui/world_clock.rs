use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use jiff::{Timestamp, Unit, tz::TimeZone};

/// Which column the table is currently being sorted with if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    /// Insertion order by default.
    #[default]
    None,
    Timezone,
    Time,
    Date,
    Offset,
}

/// Sort direction ascending or descending or other?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

/// Marker component for if the window shows.
#[derive(Component)]
pub struct ShowWorldClock;

/// Persistent state for the World Clock egui window.
#[derive(Resource)]
pub struct WorldClockState {
    /// IANA timezone identifiers to display.
    pub timezones: Vec<String>,

    /// Text typed into the add timezone search field.
    pub search: String,

    /// Cached filtered list of tz names matching `search`.
    filtered: Vec<&'static str>,

    /// True when `search` was modified and `filtered` needs to be rebuilt for next frame
    search_dirty: bool,

    /// Which column is the active sort key None = insertion order as default in the enum
    pub sort_col: SortColumn,

    /// Ascending or descending for the active sort column.
    pub sort_dir: SortDir,
}

/// Resolve the IANA name of the system's local timezone.
///
/// Falls back to "UTC" if it can't be determined, aka wasm until I figure out
/// how to find the users timezone. Why is everything web such a pita.
fn local_tz_name() -> String {
    TimeZone::system()
        .iana_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "UTC".to_string())
}

impl Default for WorldClockState {
    fn default() -> Self {
        let local = local_tz_name();

        // Default start is local timezone then these defaults. Need to make it
        // so I can pass these in as args later
        let mut timezones = vec![
            local.clone(),
            "UTC".to_string(),
            "America/New_York".to_string(),
            "America/Los_Angeles".to_string(),
            "Europe/Paris".to_string(),
            "Asia/Tokyo".to_string(),
        ];

        // Deduplicate whilst preserving order, note the local timezone may already equal one of
        // the preset strings, in which case say Asia/Tokyo would be first not last in the list.
        timezones.dedup();

        Self {
            timezones,
            search: String::new(),
            filtered: Vec::new(),
            search_dirty: true,
            sort_col: SortColumn::default(),
            sort_dir: SortDir::default(),
        }
    }
}

impl WorldClockState {
    fn rebuild_filter(&mut self) {
        let needle = self.search.to_ascii_lowercase();
        self.filtered = jiff_tzdb::available()
            .filter(|name| needle.is_empty() || name.to_ascii_lowercase().contains(&needle))
            .take(50)
            .collect();
        self.search_dirty = false;
    }
}

/// Format a `Timestamp` in the given IANA timezone.
fn format_tz(ts: Timestamp, iana: &str) -> Option<(String, String, String, i32, u8, u8, u8)> {
    let tz = TimeZone::get(iana).ok()?;
    let zdt = ts.to_zoned(tz).round(Unit::Second).ok()?;

    let hour = zdt.hour() as u8;
    let minute = zdt.minute() as u8;
    let second = zdt.second() as u8;
    let time = zdt.strftime("%H:%M:%S").to_string();
    let date = zdt.strftime("%Y-%m-%d %a").to_string();

    // Raw utc offset string aka +0530 or -0400
    let raw = zdt.strftime("%z").to_string();

    // Parse into signed minutes for numeric comparisons.
    let offset_mins = raw
        .parse::<i32>()
        .ok()
        .map(|v| {
            let sign = if v < 0 { -1 } else { 1 };
            let abs = v.unsigned_abs() as i32;
            sign * ((abs / 100) * 60 + (abs % 100))
        })
        .unwrap_or(0);

    let offset = if raw.len() == 5 {
        format!("{}:{}", &raw[..3], &raw[3..])
    } else {
        raw
    };

    Some((time, date, offset, offset_mins, hour, minute, second))
}

/// Draw a minimal analog clock face using egui's `Painter` API for now.
///
/// At "night" the clock face is black with white H/M hands.
/// At "day" it is white with black H/M hands.
///
/// Daytime is 8am to 5pm
/// Night is the rest.
///
/// I'll fix this to be better later.
fn analog_clock(ui: &mut egui::Ui, h: u8, m: u8, s: u8, radius: f32, is_off_hours: bool) {
    let size = egui::vec2(radius * 2.0, radius * 2.0);
    let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());
    let center = response.rect.center();

    // Second hand is always red.
    let (face_color, hand_color) = if is_off_hours {
        (egui::Color32::BLACK, egui::Color32::WHITE)
    } else {
        (egui::Color32::WHITE, egui::Color32::BLACK)
    };

    painter.circle_filled(center, radius, face_color);

    let hand = |frac: f32, angle: f32| -> egui::Pos2 {
        let (sin, cos) = angle.sin_cos();
        center + egui::vec2(sin, -cos) * (radius * frac)
    };

    let tau = std::f32::consts::TAU;

    // Hour
    let h_angle = (h as f32 % 12.0 + m as f32 / 60.0) / 12.0 * tau;
    painter.line_segment(
        [center, hand(0.55, h_angle)],
        egui::Stroke::new(2.5, hand_color),
    );

    // Minute
    let m_angle = (m as f32 + s as f32 / 60.0) / 60.0 * tau;
    painter.line_segment(
        [center, hand(0.78, m_angle)],
        egui::Stroke::new(1.5, hand_color),
    );

    // Second
    let s_angle = s as f32 / 60.0 * tau;
    painter.line_segment(
        [center, hand(0.88, s_angle)],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 80, 80)),
    );
}

/// Cycle the sort state when a column header is clicked.
fn cycle_sort(cur_col: SortColumn, clicked: SortColumn, dir: &mut SortDir) -> SortColumn {
    if cur_col == clicked {
        if *dir == SortDir::Asc {
            *dir = SortDir::Desc;
            clicked
        } else {
            // third click resets to insertion order
            *dir = SortDir::Asc;
            SortColumn::None
        }
    } else {
        *dir = SortDir::Asc;
        clicked
    }
}

pub fn world_clock_window(
    mut contexts: EguiContexts,
    show_query: Query<Entity, With<ShowWorldClock>>,
    mut state: ResMut<WorldClockState>,
    mut commands: Commands,
) -> Result {
    if show_query.is_empty() {
        return Ok(());
    }

    let now = Timestamp::now();
    let local = local_tz_name();

    // Build a flat vec of row data keyed by original index so removals stay
    // correct even after sorting.
    struct RowData {
        orig_idx: usize,
        name: String,
        time: String,
        date: String,
        offset: String,
        offset_mins: i32,
        hour: u8,
        minute: u8,
        second: u8,
        valid: bool,
    }

    let mut rows: Vec<RowData> = state
        .timezones
        .iter()
        .enumerate()
        .map(|(orig_idx, tz_name)| match format_tz(now, tz_name) {
            Some((time, date, offset, offset_mins, hour, minute, second)) => RowData {
                orig_idx,
                name: tz_name.clone(),
                time,
                date,
                offset,
                offset_mins,
                hour,
                minute,
                second,
                valid: true,
            },
            None => RowData {
                orig_idx,
                name: tz_name.clone(),
                time: String::new(),
                date: String::new(),
                offset: String::new(),
                offset_mins: i32::MAX,
                hour: 0,
                minute: 0,
                second: 0,
                valid: false,
            },
        })
        .collect();

    // Apply sort if a sort is present.
    let sort_col = state.sort_col;
    let sort_dir = state.sort_dir;
    if sort_col != SortColumn::None {
        rows.sort_by(|a, b| {
            let ord = match sort_col {
                SortColumn::None => std::cmp::Ordering::Equal,
                SortColumn::Timezone => a.name.cmp(&b.name),
                SortColumn::Time => a.time.cmp(&b.time),
                SortColumn::Date => a.date.cmp(&b.date),
                SortColumn::Offset => a.offset_mins.cmp(&b.offset_mins),
            };
            if sort_dir == SortDir::Desc {
                ord.reverse()
            } else {
                ord
            }
        });
    }

    let header_btn = |ui: &mut egui::Ui,
                      label: &str,
                      col: SortColumn,
                      cur_col: SortColumn,
                      cur_dir: SortDir|
     -> bool {
        let indicator = if cur_col == col {
            if cur_dir == SortDir::Asc {
                " ▲"
            } else {
                " ▼"
            }
        } else {
            " ⇅"
        };
        let text = egui::RichText::new(format!("{label}{indicator}"))
            .strong()
            .color(if cur_col == col {
                egui::Color32::from_rgb(220, 220, 255)
            } else {
                egui::Color32::from_rgb(150, 150, 210)
            });
        ui.button(text).clicked()
    };

    const CLOCK_RADIUS: f32 = 30.0;

    let mut open = true;
    egui::Window::new("World Clock")
        .open(&mut open)
        .auto_sized()
        .show(contexts.ctx_mut()?, |ui| {
            egui::Grid::new("wc_grid")
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label(""); // analog clock column is first and has no real header
                    if header_btn(ui, "Timezone", SortColumn::Timezone, sort_col, sort_dir) {
                        state.sort_col =
                            cycle_sort(state.sort_col, SortColumn::Timezone, &mut state.sort_dir);
                    }
                    if header_btn(ui, "Time", SortColumn::Time, sort_col, sort_dir) {
                        state.sort_col =
                            cycle_sort(state.sort_col, SortColumn::Time, &mut state.sort_dir);
                    }
                    if header_btn(ui, "Date", SortColumn::Date, sort_col, sort_dir) {
                        state.sort_col =
                            cycle_sort(state.sort_col, SortColumn::Date, &mut state.sort_dir);
                    }
                    if header_btn(ui, "Offset", SortColumn::Offset, sort_col, sort_dir) {
                        state.sort_col =
                            cycle_sort(state.sort_col, SortColumn::Offset, &mut state.sort_dir);
                    }
                    ui.label(""); // ✖ column
                    ui.end_row();

                    let mut to_remove: Option<usize> = None;

                    for row in &rows {
                        let is_local = row.name == local;

                        // Clock cell
                        let is_off_hours = row.hour < 8 || row.hour >= 17;
                        if row.valid {
                            analog_clock(
                                ui,
                                row.hour,
                                row.minute,
                                row.second,
                                CLOCK_RADIUS,
                                is_off_hours,
                            );
                        } else {
                            ui.allocate_exact_size(
                                egui::vec2(CLOCK_RADIUS * 2.0, CLOCK_RADIUS * 2.0),
                                egui::Sense::hover(),
                            );
                        }

                        // Timezone name in timezone format
                        ui.label(if is_local {
                            egui::RichText::new(format!("★ {}", row.name))
                                .strong()
                                .color(egui::Color32::WHITE)
                        } else {
                            egui::RichText::new(row.name.as_str())
                        });

                        if row.valid {
                            let is_off_hours = row.hour < 8 || row.hour >= 17;
                            let (normal_color, local_color) = if is_off_hours {
                                (
                                    egui::Color32::from_rgb(100, 160, 255),
                                    egui::Color32::from_rgb(140, 200, 255),
                                )
                            } else {
                                (
                                    egui::Color32::from_rgb(120, 220, 120),
                                    egui::Color32::from_rgb(160, 255, 160),
                                )
                            };

                            ui.label(if is_local {
                                egui::RichText::new(&row.time)
                                    .monospace()
                                    .strong()
                                    .color(local_color)
                            } else {
                                egui::RichText::new(&row.time)
                                    .monospace()
                                    .color(normal_color)
                            });

                            ui.label(if is_local {
                                egui::RichText::new(&row.date)
                                    .strong()
                                    .color(egui::Color32::WHITE)
                            } else {
                                egui::RichText::new(&row.date)
                            });

                            ui.label(if is_local {
                                egui::RichText::new(&row.offset)
                                    .strong()
                                    .color(egui::Color32::from_rgb(255, 230, 80))
                            } else {
                                egui::RichText::new(&row.offset)
                                    .color(egui::Color32::from_rgb(200, 200, 100))
                            });
                        } else {
                            ui.label(egui::RichText::new("invalid").color(egui::Color32::RED));
                            ui.label("-");
                            ui.label("-");
                        }

                        if ui
                            .small_button(egui::RichText::new("✖").color(egui::Color32::RED))
                            .on_hover_text("Remove")
                            .clicked()
                        {
                            to_remove = Some(row.orig_idx);
                        }

                        ui.end_row();
                    }

                    if let Some(idx) = to_remove {
                        state.timezones.remove(idx);
                    }
                });

            ui.add_space(6.0);

            ui.label(egui::RichText::new("Add timezone:").strong());
            let search_width = 240.0_f32;
            let text_height = ui.spacing().interact_size.y;
            let resp = ui.add_sized(
                [search_width, text_height],
                egui::TextEdit::singleline(&mut state.search),
            );
            if resp.changed() {
                state.search_dirty = true;
            }

            if state.search_dirty {
                state.rebuild_filter();
            }

            if !state.filtered.is_empty() {
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .min_scrolled_width(search_width)
                    .id_salt("wc_tz_picker")
                    .show(ui, |ui| {
                        ui.set_max_width(search_width);
                        let mut add_tz: Option<String> = None;
                        for &tz_name in &state.filtered {
                            let already_added = state.timezones.iter().any(|t| t == tz_name);
                            ui.horizontal(|ui| {
                                if already_added {
                                    ui.label(
                                        egui::RichText::new(tz_name)
                                            .color(egui::Color32::GRAY)
                                            .italics(),
                                    );
                                } else if ui.selectable_label(false, tz_name).clicked() {
                                    add_tz = Some(tz_name.to_string());
                                }
                            });
                        }
                        if let Some(tz) = add_tz {
                            state.timezones.push(tz);
                            state.search.clear();
                            state.search_dirty = true;
                        }
                    });
            }
        });

    if !open && let Ok(entity) = show_query.single() {
        commands.entity(entity).despawn();
    }

    Ok(())
}
