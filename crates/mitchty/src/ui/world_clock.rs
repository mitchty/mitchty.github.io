use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use jiff::{Timestamp, Unit, civil, tz::TimeZone};

/// Which column the table is currently being sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    /// Insertion order by default.
    #[default]
    None,
    Timezone,
    Time,
    Date,
    Offset,
    /// Signed difference in minutes between this row's UTC offset and the local timezone's.
    DeltaLocal,
}

/// Sort direction ascending or descending or other?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

/// Marker component for if the window shows or not.
#[derive(Component)]
pub struct ShowWorldClock;

/// Transient edit state for the unified date+time picker popup.
#[derive(Debug, Clone)]
pub struct EditState {
    /// IANA tz context of the row that was clicked. Used for converting user
    /// picked value for internal calculations for the time changes.
    pub tz: String,
    /// Year component (drag-value spinner). Stored as i32 for DragValue compat.
    pub year: i32,
    /// Month 1–12.
    pub month: u8,
    /// Day 1–31. Note: it changes based on month to how many days are in the month picked.
    pub day: u8,
    /// Hour 0–23.
    pub hour: u8,
    /// Minute 0–59.
    pub minute: u8,
    /// Second 0–59.
    pub second: u8,
    /// Screen position where the popup should be anchored defaults to where user tapped.
    pub spawn_pos: egui::Pos2,
}

impl EditState {
    /// Build an EditState from a known timestamp in a given tz, originating
    /// the popup at `spawn_pos`.
    fn from_ts(ts: Timestamp, iana: &str, spawn_pos: egui::Pos2) -> Option<Self> {
        let tz = TimeZone::get(iana).ok()?;
        let zdt = ts.to_zoned(tz).round(Unit::Second).ok()?;
        Some(Self {
            tz: iana.to_string(),
            year: zdt.year() as i32,
            month: zdt.month() as u8,
            day: zdt.day() as u8,
            hour: zdt.hour() as u8,
            minute: 0,
            // Seconds are always pinned to 0, makes zero sense to change this.
            second: 0,
            spawn_pos,
        })
    }

    /// Convert the current spinner values back to a UTC Timestamp.
    fn to_timestamp(&self) -> Option<Timestamp> {
        let dt = civil::DateTime::new(
            self.year as i16,
            self.month as i8,
            self.day as i8,
            self.hour as i8,
            self.minute as i8,
            self.second as i8,
            0,
        )
        .ok()?;
        Some(dt.in_tz(&self.tz).ok()?.timestamp())
    }
}

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

    /// When Some the clock is frozen at this moment instead of showing live time.
    pub pinned_time: Option<Timestamp>,

    /// History of all pinned timestamps the user has applied, oldest first.
    pub time_history: Vec<Timestamp>,

    /// Index into `time_history` of the currently displayed pinned time.
    /// Only meaningful when `pinned_time.is_some()`.
    pub history_cursor: usize,

    /// Active edit popup state. None = no popup open.
    pub editing: Option<EditState>,
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

/// Returns the IANA name of the local timezone only when it was genuinely
/// resolved by the OS. Returns `None` when jiff fell back to UTC aka if on a
/// wasm target that can't get the local tz, so callers can distinguish "we know
/// the local tz" from "we guessed".
fn local_tz_known() -> Option<String> {
    TimeZone::system().iana_name().map(|s| s.to_string())
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
            pinned_time: None,
            time_history: Vec::new(),
            history_cursor: 0,
            editing: None,
        }
    }
}

impl WorldClockState {
    fn rebuild_filter(&mut self) {
        let needle = self.search.to_ascii_lowercase();
        self.filtered = jiff_tzdb::available()
            .filter(|name| needle.is_empty() || name.to_ascii_lowercase().contains(&needle))
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

    // Parse into signed minutes for backend numeric comparisons.
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
/// I'll fix this to be better later. Might make it a shader.
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
            // third click resets to original insertion order
            *dir = SortDir::Asc;
            SortColumn::None
        }
    } else {
        *dir = SortDir::Asc;
        clicked
    }
}

/// Draw date and time picker popup.
fn draw_picker_popup(ctx: &egui::Context, edit: &mut EditState) -> (Option<Timestamp>, bool) {
    let mut confirmed_ts: Option<Timestamp> = None;
    let mut cancelled = false;

    const MONTH_NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];

    egui::Window::new("Set Date & Time")
        .collapsible(false)
        .resizable(false)
        .fixed_pos(edit.spawn_pos)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(format!("Timezone: {}", edit.tz))
                    .small()
                    .color(egui::Color32::GRAY),
            );
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label("Year");
                ui.add(
                    egui::DragValue::new(&mut edit.year)
                        .range(1970..=2200)
                        .speed(0.2),
                );
                // Clamp day whenever year changes so that if say 31 was chosen and feb is picked 28 shows.
                let max_day = days_in_month(edit.year, edit.month);
                edit.day = edit.day.min(max_day);

                egui::ComboBox::new("wc_pick_month", "Month")
                    .selected_text(MONTH_NAMES[(edit.month - 1) as usize])
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for (idx, name) in MONTH_NAMES.iter().enumerate() {
                            let mo = (idx + 1) as u8;
                            if ui.selectable_value(&mut edit.month, mo, *name).changed() {
                                let max = days_in_month(edit.year, edit.month);
                                edit.day = edit.day.min(max);
                            }
                        }
                    });

                let max_day = days_in_month(edit.year, edit.month);
                egui::ComboBox::new("wc_pick_day", "Day")
                    .selected_text(format!("{:02}", edit.day))
                    .width(55.0)
                    .show_ui(ui, |ui| {
                        for d in 1u8..=max_day {
                            ui.selectable_value(&mut edit.day, d, format!("{:02}", d));
                        }
                    });
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                egui::ComboBox::new("wc_pick_hour", "Hour")
                    .selected_text(format!("{:02}", edit.hour))
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        for h in 0u8..=23 {
                            ui.selectable_value(&mut edit.hour, h, format!("{:02}", h));
                        }
                    });
                egui::ComboBox::new("wc_pick_minute", "Min")
                    .selected_text(format!("{:02}", edit.minute))
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        for m in 0u8..=59 {
                            ui.selectable_value(&mut edit.minute, m, format!("{:02}", m));
                        }
                    });
            });
            // Seconds are always 0, changing them is silly.
            edit.second = 0;

            ui.add_space(4.0);

            let preview_color = if edit.to_timestamp().is_some() {
                egui::Color32::from_rgb(120, 220, 120)
            } else {
                egui::Color32::RED
            };
            let preview_txt = match edit.to_timestamp() {
                Some(ts) => {
                    if let Some((time, date, offset, ..)) = format_tz(ts, &edit.tz) {
                        format!("{date}  {time}  ({offset})")
                    } else {
                        "—".to_string()
                    }
                }
                None => "Invalid date/time".to_string(),
            };
            ui.label(
                egui::RichText::new(preview_txt)
                    .small()
                    .color(preview_color),
            );

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                let can_apply = edit.to_timestamp().is_some();
                if ui
                    .add_enabled(can_apply, egui::Button::new("✔ Apply"))
                    .clicked()
                {
                    confirmed_ts = edit.to_timestamp();
                }
                if ui.button("✖ Cancel").clicked() {
                    cancelled = true;
                }
            });
        });

    (confirmed_ts, cancelled)
}

/// Returns the number of days in a given month/year, accounting for leap years.
fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
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

    let live_now = Timestamp::now();
    let now = state.pinned_time.unwrap_or(live_now);
    let is_pinned = state.pinned_time.is_some();
    let local = local_tz_name();

    // Build a flat vec of row data keyed by original index so removals stay
    // correct even after sorting occurs.
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

    // Determine the local offset in minutes for the delta Local column.
    // Only Some when the OS reported a real IANA name AND that timezone is
    // actually present in the displayed list.
    let local_offset_mins: Option<i32> = local_tz_known().and_then(|known_local| {
        if state.timezones.contains(&known_local) {
            rows.iter()
                .find(|r| r.name == known_local && r.valid)
                .map(|r| r.offset_mins)
        } else {
            None
        }
    });

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
                SortColumn::DeltaLocal => {
                    // Sort by signed diff in minutes. Rows without a valid
                    // offset or when local_offset_mins is unavailable sink to
                    // the bottom.
                    let delta = |r: &RowData| -> i32 {
                        match (r.valid, local_offset_mins) {
                            (true, Some(local)) => r.offset_mins - local,
                            _ => i32::MAX,
                        }
                    };
                    delta(a).cmp(&delta(b))
                }
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

    let mut open_edit: Option<(String, Timestamp, egui::Pos2)> = None;

    let mut go_live = false;
    let mut go_back = false;
    let mut go_forward = false;
    let mut clear_history = false;

    let has_history = !state.time_history.is_empty();
    let cursor = state.history_cursor;
    let can_go_back = (cursor > 0 || !is_pinned) && has_history;
    let can_go_forward = is_pinned && has_history && cursor + 1 < state.time_history.len();

    let mut open = true;

    let ctx = contexts.ctx_mut()?;

    egui::Window::new("World Clock")
        .open(&mut open)
        .auto_sized()
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if is_pinned || has_history {
                    // Status label.
                    if is_pinned {
                        ui.label(
                            egui::RichText::new("🔒 Pinned time not live")
                                .color(egui::Color32::from_rgb(255, 200, 60))
                                .strong(),
                        );
                    } else {
                        // We have history but currently are in live mode.
                        ui.label(
                            egui::RichText::new("🕐 Live")
                                .color(egui::Color32::from_rgb(100, 220, 100))
                                .strong(),
                        );
                    }

                    // <- moves backwards through time pick history.
                    if ui
                        .add_enabled(can_go_back, egui::Button::new("◀"))
                        .on_hover_text("Go to previous pinned time")
                        .clicked()
                    {
                        go_back = true;
                    }

                    // History time picks position indicator, aka N / M "2 / 4".
                    if has_history {
                        let shown = if is_pinned { cursor + 1 } else { 0 };
                        ui.label(
                            egui::RichText::new(format!(
                                "{} / {}",
                                shown,
                                state.time_history.len()
                            ))
                            .weak(),
                        );
                    }

                    // -> moves forward through history picks
                    if ui
                        .add_enabled(can_go_forward, egui::Button::new("▶"))
                        .on_hover_text("Go to next pinned time")
                        .clicked()
                    {
                        go_forward = true;
                    }

                    // Go back to Live time.
                    if is_pinned
                        && ui
                            .button(
                                egui::RichText::new("↩ Go Live")
                                    .color(egui::Color32::from_rgb(100, 220, 100)),
                            )
                            .on_hover_text("Return to live clock")
                            .clicked()
                    {
                        go_live = true;
                    }

                    // Clear all history.
                    if ui
                        .button(
                            egui::RichText::new("🗑 Clear")
                                .color(egui::Color32::from_rgb(220, 100, 100)),
                        )
                        .on_hover_text("Clear all pinned time history")
                        .clicked()
                    {
                        clear_history = true;
                    }
                } else {
                    // No history, just reserve the space to prevent egui window resizing.
                    ui.label(egui::RichText::new(" ").strong());
                }
            });
            ui.add_space(4.0);

            egui::Grid::new("wc_grid")
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label(""); // analog clock column is first and has no real header to speak of
                    if header_btn(ui, "Timezone", SortColumn::Timezone, sort_col, sort_dir) {
                        state.sort_col =
                            cycle_sort(state.sort_col, SortColumn::Timezone, &mut state.sort_dir);
                    }
                    if header_btn(ui, "Date", SortColumn::Date, sort_col, sort_dir) {
                        state.sort_col =
                            cycle_sort(state.sort_col, SortColumn::Date, &mut state.sort_dir);
                    }
                    if header_btn(ui, "Time", SortColumn::Time, sort_col, sort_dir) {
                        state.sort_col =
                            cycle_sort(state.sort_col, SortColumn::Time, &mut state.sort_dir);
                    }
                    if header_btn(ui, "Offset", SortColumn::Offset, sort_col, sort_dir) {
                        state.sort_col =
                            cycle_sort(state.sort_col, SortColumn::Offset, &mut state.sort_dir);
                    }
                    if local_offset_mins.is_some()
                        && header_btn(ui, "Δ Local", SortColumn::DeltaLocal, sort_col, sort_dir)
                    {
                        state.sort_col =
                            cycle_sort(state.sort_col, SortColumn::DeltaLocal, &mut state.sort_dir);
                    }
                    ui.label("");
                    ui.end_row();

                    let mut to_remove: Option<usize> = None;

                    for row in &rows {
                        let is_local = row.name == local;

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

                            let date_text = if is_local {
                                egui::RichText::new(&row.date)
                                    .strong()
                                    .color(egui::Color32::WHITE)
                            } else {
                                egui::RichText::new(&row.date)
                            };
                            let date_btn = egui::Button::new(date_text).frame(false);
                            let date_resp =
                                ui.add(date_btn).on_hover_text("Click to set date & time");
                            if date_resp.clicked() {
                                let pos = date_resp.rect.left_bottom();
                                open_edit = Some((row.name.clone(), now, pos));
                            }

                            let time_text = if is_local {
                                egui::RichText::new(&row.time)
                                    .monospace()
                                    .strong()
                                    .color(local_color)
                            } else {
                                egui::RichText::new(&row.time)
                                    .monospace()
                                    .color(normal_color)
                            };
                            let time_btn = egui::Button::new(time_text).frame(false);
                            let time_resp =
                                ui.add(time_btn).on_hover_text("Click to set date & time");
                            if time_resp.clicked() {
                                let pos = time_resp.rect.left_bottom();
                                open_edit = Some((row.name.clone(), now, pos));
                            }

                            {
                                let offset_color = if row.offset_mins > 0 {
                                    egui::Color32::from_rgb(120, 220, 120)
                                } else if row.offset_mins < 0 {
                                    egui::Color32::from_rgb(100, 160, 255)
                                } else {
                                    egui::Color32::GRAY
                                };
                                let text = egui::RichText::new(&row.offset)
                                    .monospace()
                                    .color(offset_color);
                                ui.label(if is_local { text.strong() } else { text });
                            }

                            if let Some(local_mins) = local_offset_mins {
                                if is_local {
                                    // This is the reference row — no diff to show.
                                    ui.label(
                                        egui::RichText::new("+00:00").color(egui::Color32::GRAY),
                                    );
                                } else {
                                    let diff = row.offset_mins - local_mins;
                                    let abs = diff.unsigned_abs() as i32;
                                    let h = abs / 60;
                                    let m = abs % 60;
                                    let (text, color) = if diff == 0 {
                                        ("same".to_string(), egui::Color32::GRAY)
                                    } else if diff > 0 {
                                        (
                                            format!("+{}:{:02}", h, m),
                                            egui::Color32::from_rgb(120, 220, 120),
                                        )
                                    } else {
                                        (
                                            format!("-{}:{:02}", h, m),
                                            egui::Color32::from_rgb(100, 160, 255),
                                        )
                                    };
                                    ui.label(egui::RichText::new(text).monospace().color(color));
                                }
                            }
                        } else {
                            ui.label(egui::RichText::new("invalid").color(egui::Color32::RED));
                            ui.label("-");
                            ui.label("-");
                            if local_offset_mins.is_some() {
                                ui.label("");
                            }
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
                    .max_height(300.0)
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

    if clear_history {
        state.time_history.clear();
        state.history_cursor = 0;
        state.pinned_time = None;
        state.editing = None;
    } else if go_live {
        state.pinned_time = None;
    } else if go_back {
        if !state.time_history.is_empty() {
            if is_pinned && cursor > 0 {
                state.history_cursor = cursor - 1;
            } else if !is_pinned {
                state.history_cursor = state.time_history.len() - 1;
            }
            state.pinned_time = Some(state.time_history[state.history_cursor]);
        }
    } else if go_forward && is_pinned && cursor + 1 < state.time_history.len() {
        state.history_cursor = cursor + 1;
        state.pinned_time = Some(state.time_history[state.history_cursor]);
    }

    if let Some((tz_name, ts, pos)) = open_edit {
        let already_open = state
            .editing
            .as_ref()
            .map(|e| e.tz == tz_name)
            .unwrap_or(false);
        if already_open {
            state.editing = None;
        } else {
            state.editing = EditState::from_ts(ts, &tz_name, pos);
        }
    }

    let mut close_edit = false;
    if let Some(edit) = state.editing.as_mut() {
        let (confirmed_ts, cancelled) = draw_picker_popup(ctx, edit);
        if let Some(ts) = confirmed_ts {
            let new_cursor = if state.pinned_time.is_some() {
                state.history_cursor + 1
            } else {
                state.time_history.len()
            };
            state.time_history.truncate(new_cursor);
            state.time_history.push(ts);
            state.history_cursor = state.time_history.len() - 1;
            state.pinned_time = Some(ts);
            close_edit = true;
        } else if cancelled {
            close_edit = true;
        }
    }
    if close_edit {
        state.editing = None;
    }

    if !open && let Ok(entity) = show_query.single() {
        commands.entity(entity).despawn();
    }

    Ok(())
}
