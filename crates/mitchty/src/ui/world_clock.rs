use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use jiff::{Timestamp, Unit, civil, tz::TimeZone};

#[cfg(not(target_arch = "wasm32"))]
use arboard;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures;

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

impl SortColumn {
    /// Parse from a CLI/URL slug, case-insensitive.
    pub fn from_slug(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "timezone" | "tz" => Some(Self::Timezone),
            "time" => Some(Self::Time),
            "date" => Some(Self::Date),
            "offset" => Some(Self::Offset),
            "delta" | "delta-local" | "delta_local" => Some(Self::DeltaLocal),
            _ => None,
        }
    }

    /// Serialize to the canonical slug used in CLI args and URL params.
    pub fn to_slug(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Timezone => Some("timezone"),
            Self::Time => Some("time"),
            Self::Date => Some("date"),
            Self::Offset => Some("offset"),
            Self::DeltaLocal => Some("delta-local"),
        }
    }
}

/// Sort direction ascending or descending or other?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

impl SortDir {
    /// Parse from a CLI/URL slug, case-insensitive.
    /// Returns `None` for unrecognised values so callers can fail properly.
    pub fn from_slug(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "asc" | "ascending" | "a" => Some(Self::Asc),
            "desc" | "descending" | "d" => Some(Self::Desc),
            _ => None,
        }
    }

    /// Serialize to the canonical slug.
    pub fn to_slug(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
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

/// Alarm entry, simply in UTC moment and the IANA timezone for display usage.
#[derive(Debug, Clone)]
pub struct AlarmEntry {
    /// The exact UTC time when the alarm fires.
    pub target_ts: Timestamp,
    /// The IANA timezone used for the alarm.
    pub label_tz: String,
    /// Optional human-readable label. When set the 3D countdown reads
    /// `"<label> in <countdown>"` instead of the bare countdown.
    pub label: Option<String>,
}

/// Transient state for the alarm picker popup.
#[derive(Debug, Clone)]
pub struct AlarmState {
    /// Currently selected IANA tz from the dropdown.
    pub tz: String,
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub second: u8,
    /// Screen position where the popup is anchored.
    pub spawn_pos: egui::Pos2,
    /// Optional label for the alarm.
    pub label: String,
    /// Set to true the first time the hour dropdown scrolls to the current
    /// hour so we don't redraw back to the current hour every time.
    pub hour_scrolled: bool,
    /// Same as ^^^ but for the minute dropdown instead.
    pub minute_scrolled: bool,
}

impl AlarmState {
    /// Build a default `AlarmState` anchored at `spawn_pos`, with the first
    /// available timezone pre-selected and the fields set to the current time
    /// in that specific timezone.
    fn new(now: Timestamp, tz: &str, spawn_pos: egui::Pos2) -> Self {
        if let Ok(jtz) = TimeZone::get(tz)
            && let Ok(zdt) = now.to_zoned(jtz).round(Unit::Second)
        {
            return Self {
                tz: tz.to_string(),
                year: zdt.year() as i32,
                month: zdt.month() as u8,
                day: zdt.day() as u8,
                hour: None,
                minute: None,
                second: 0,
                spawn_pos,
                label: String::new(),
                hour_scrolled: false,
                minute_scrolled: false,
            };
        }
        Self {
            tz: tz.to_string(),
            year: 2025,
            month: 1,
            day: 1,
            hour: None,
            minute: None,
            second: 0,
            spawn_pos,
            label: String::new(),
            hour_scrolled: false,
            minute_scrolled: false,
        }
    }

    /// Convert the current picker values to UTC `Timestamp`.
    fn to_timestamp(&self, now: Timestamp, tz: &str) -> Option<Timestamp> {
        // Resolve live time once so both components can fall back independently.
        let (live_hour, live_minute) = if let Ok(jtz) = TimeZone::get(tz)
            && let Ok(zdt) = now.to_zoned(jtz).round(Unit::Second)
        {
            (zdt.hour() as u8, zdt.minute() as u8)
        } else {
            (0, 0)
        };

        // Each component uses the user-selected value when Some, otherwise
        // falls back to live time. This means selecting only the
        // hour or minute works as expected.
        let hour = self.hour.unwrap_or(live_hour);
        let minute = self.minute.unwrap_or(live_minute);

        let dt = civil::DateTime::new(
            self.year as i16,
            self.month as i8,
            self.day as i8,
            hour as i8,
            minute as i8,
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

    /// Index into `time_history` of the currently displayed pinned time. Only
    /// meaningful when `pinned_time.is_some()`. Used for editing the optional
    /// alarm text.
    pub history_cursor: usize,

    /// Active edit popup state. None = no popup open.
    pub editing: Option<EditState>,

    /// All alarms the user has set, stored in insertion order for future use?
    pub alarms: Vec<AlarmEntry>,

    /// Active alarm picker popup state. None = closed.
    pub editing_alarm: Option<AlarmState>,

    /// When `Some`, the copy-link button was clicked recently.
    /// Stores the `Timestamp` of the click so we can show a brief "✔ Copied!"
    /// label for about 2 seconds and then revert back to the normal label.
    pub copy_feedback_until: Option<Timestamp>,

    /// Index of the alarm whose label is currently being edited inline.
    /// None = no inline edit in progress.
    pub editing_label: Option<usize>,

    /// Scratch buffer for the inline label text edit.
    pub label_edit_buf: String,
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

        // Default timezone list: local tz first, then random timezones I find useful.
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
            alarms: Vec::new(),
            editing_alarm: None,
            copy_feedback_until: None,
            editing_label: None,
            label_edit_buf: String::new(),
        }
    }
}

impl WorldClockState {
    /// Build a `WorldClockState` seeded from the values parsed out of CLI args
    /// or URL query parameters. Basically lets me "serialize" or "deserialize"
    /// the windows state and pass that back in via args or query params later.
    pub fn from_config(
        initial_timezones: &[String],
        initial_alarms: &[(Timestamp, String, Option<String>)],
        initial_sort_col: SortColumn,
        initial_sort_dir: SortDir,
        initial_pinned: Option<Timestamp>,
    ) -> Self {
        let mut state = Self::default();

        if !initial_timezones.is_empty() {
            let mut tzs: Vec<String> = initial_timezones.to_vec();
            tzs.dedup();
            state.timezones = tzs;
        }

        for (ts, tz, label) in initial_alarms {
            state.alarms.push(AlarmEntry {
                target_ts: *ts,
                label_tz: tz.clone(),
                label: label.clone(),
            });
        }

        state.sort_col = initial_sort_col;
        state.sort_dir = initial_sort_dir;

        if let Some(ts) = initial_pinned {
            state.time_history.push(ts);
            state.history_cursor = 0;
            state.pinned_time = Some(ts);
        }

        state
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
/// I'll fix this to be better later. Might make it a shader, who knows? Only
/// future sucker mitch!
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
        egui::Stroke::new(2.5_f32, hand_color),
    );

    // Minute
    let m_angle = (m as f32 + s as f32 / 60.0) / 60.0 * tau;
    painter.line_segment(
        [center, hand(0.78, m_angle)],
        egui::Stroke::new(1.5_f32, hand_color),
    );

    // Second
    let s_angle = s as f32 / 60.0 * tau;
    painter.line_segment(
        [center, hand(0.88, s_angle)],
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(255, 80, 80)),
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
                        "-".to_string()
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

/// Draw the alarm-setting popup window.
///
/// `available_tzs` is the list of IANA timezone names currently shown in the
/// world clock table; the user picks one of those as the relevent alarm
/// timezone.
///
/// Returns:
/// - `(Some((ts, tz, label)), false)` when the user clicks apply with a valid future time.
///   `label` is `Some(text)` when the user filled in the optional label field.
/// - `(None, true)` when the user clicks cancel.
/// - `(None, false)` while the popup is still open and people are dilly dallying about.
fn draw_alarm_popup(
    ctx: &egui::Context,
    alarm: &mut AlarmState,
    now: Timestamp,
    available_tzs: &[String],
) -> (Option<(Timestamp, String, Option<String>)>, bool) {
    let mut confirmed: Option<(Timestamp, String, Option<String>)> = None;
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

    egui::Window::new("Set Alarm")
        .collapsible(false)
        .resizable(false)
        .fixed_pos(alarm.spawn_pos)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Timezone:");
                egui::ComboBox::new("alarm_tz_pick", "")
                    .selected_text(alarm.tz.as_str())
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for tz_name in available_tzs {
                            if ui
                                .selectable_value(&mut alarm.tz, tz_name.clone(), tz_name.as_str())
                                .changed()
                                && let Ok(jtz) = TimeZone::get(&alarm.tz)
                                && let Ok(zdt) = now.to_zoned(jtz).round(Unit::Second)
                            {
                                alarm.year = zdt.year() as i32;
                                alarm.month = zdt.month() as u8;
                                alarm.day = zdt.day() as u8;
                                alarm.hour = None;
                                alarm.minute = None;
                                alarm.second = 0;
                                alarm.hour_scrolled = false;
                                alarm.minute_scrolled = false;
                            }
                        }
                    });
            });

            ui.add_space(4.0);

            // Date row.
            ui.horizontal(|ui| {
                ui.label("Year");
                ui.add(
                    egui::DragValue::new(&mut alarm.year)
                        .range(1970..=2200)
                        .speed(0.2),
                );
                let max_day = days_in_month(alarm.year, alarm.month);
                alarm.day = alarm.day.min(max_day);

                egui::ComboBox::new("alarm_pick_month", "Month")
                    .selected_text(MONTH_NAMES[(alarm.month - 1) as usize])
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for (idx, name) in MONTH_NAMES.iter().enumerate() {
                            let mo = (idx + 1) as u8;
                            if ui.selectable_value(&mut alarm.month, mo, *name).changed() {
                                let max = days_in_month(alarm.year, alarm.month);
                                alarm.day = alarm.day.min(max);
                            }
                        }
                    });

                let max_day = days_in_month(alarm.year, alarm.month);
                egui::ComboBox::new("alarm_pick_day", "Day")
                    .selected_text(format!("{:02}", alarm.day))
                    .width(55.0)
                    .show_ui(ui, |ui| {
                        for d in 1u8..=max_day {
                            ui.selectable_value(&mut alarm.day, d, format!("{:02}", d));
                        }
                    });
            });

            ui.add_space(4.0);

            // Auto-follow current time when hour/minute are None.
            // Scroll to the current value on the exact frame the popup first opened.
            let (current_hour_display, current_minute_display, cur_h, cur_m) = if let Ok(jtz) =
                TimeZone::get(&alarm.tz)
                && let Ok(zdt) = now.to_zoned(jtz).round(Unit::Second)
            {
                let h = zdt.hour() as u8;
                let m = zdt.minute() as u8;
                (format!("{:02}", h), format!("{:02}", m), h, m)
            } else {
                ("--".to_string(), "--".to_string(), 0u8, 0u8)
            };

            // TODO: keep?
            // ui.horizontal(|ui| {
            //     ui.label("Current:");
            //     ui.label(egui::RichText::new(format!("{}:{}", current_hour_display, current_minute_display)).strong());
            // });
            // ui.add_space(4.0);

            ui.horizontal(|ui| {
                let hour_display = alarm
                    .hour
                    .map(|h| format!("{:02}", h))
                    .unwrap_or_else(|| current_hour_display.clone());

                egui::ComboBox::new("alarm_pick_hour", "Hour")
                    .selected_text(hour_display)
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        for h in 0u8..=23 {
                            let resp =
                                ui.selectable_value(&mut alarm.hour, Some(h), format!("{:02}", h));
                            if !alarm.hour_scrolled && alarm.hour.is_none() && h == cur_h {
                                resp.scroll_to_me(Some(egui::Align::Center));
                                alarm.hour_scrolled = true;
                            }
                        }
                    });

                let minute_display = alarm
                    .minute
                    .map(|m| format!("{:02}", m))
                    .unwrap_or_else(|| current_minute_display.clone());

                egui::ComboBox::new("alarm_pick_minute", "Min")
                    .selected_text(minute_display)
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        for m in 0u8..=59 {
                            let resp = ui.selectable_value(
                                &mut alarm.minute,
                                Some(m),
                                format!("{:02}", m),
                            );
                            if !alarm.minute_scrolled && alarm.minute.is_none() && m == cur_m {
                                resp.scroll_to_me(Some(egui::Align::Center));
                                alarm.minute_scrolled = true;
                            }
                        }
                    });
            });
            alarm.second = 0;

            ui.add_space(4.0);

            // Optional label row.
            ui.horizontal(|ui| {
                ui.label("Label (optional):");
                ui.text_edit_singleline(&mut alarm.label);
            });

            ui.add_space(4.0);

            // Preview.
            let maybe_ts = alarm.to_timestamp(now, &alarm.tz);
            let is_future = maybe_ts.map(|ts| ts > now).unwrap_or(false);
            let preview_color = if is_future {
                egui::Color32::from_rgb(120, 220, 120)
            } else if maybe_ts.is_some() {
                egui::Color32::from_rgb(255, 60, 60)
            } else {
                egui::Color32::RED
            };
            let preview_txt = match maybe_ts {
                Some(ts) => {
                    if let Some((time, date, offset, ..)) = format_tz(ts, &alarm.tz) {
                        let past_note = if ts <= now {
                            "⚠ occurs in the past ⚠"
                        } else {
                            ""
                        };
                        format!("{date}  {time}  ({offset}){past_note}")
                    } else {
                        "-".to_string()
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
                if ui
                    .add_enabled(is_future, egui::Button::new("✔ Set Alarm"))
                    .clicked()
                    && let Some(ts) = alarm.to_timestamp(now, &alarm.tz)
                {
                    let label = alarm.label.trim().to_string();
                    confirmed = Some((
                        ts,
                        alarm.tz.clone(),
                        if label.is_empty() { None } else { Some(label) },
                    ));
                }
                if ui.button("✖ Cancel").clicked() {
                    cancelled = true;
                }
            });
        });

    (confirmed, cancelled)
}

/// Format the countdown from `now` to `target` using humantime.
/// Returns `"Expired"` when `target <= now`. Probably not worth keeping around.
fn format_countdown(target: Timestamp, now: Timestamp) -> String {
    if target <= now {
        return "Expired".to_string();
    }
    let secs = (target.as_second() - now.as_second()).max(0) as u64;
    humantime::format_duration(std::time::Duration::from_secs(secs)).to_string()
}

/// Format how long ago `target` fired relative to `now`.
/// Returns a string like `"Expired 2h 5m 3s ago"`.
fn format_elapsed(target: Timestamp, now: Timestamp) -> String {
    let secs = (now.as_second() - target.as_second()).max(0) as u64;
    format!(
        "Expired {} ago",
        humantime::format_duration(std::time::Duration::from_secs(secs))
    )
}

/// Parse a single alarm entry string in the form `[LABEL:]TZ:EPOCH`.
///
/// For "backwards compat" this allows for `string:tz:epoch` and `tz:epoch` to
/// parse the same so I don't have to update any docs that might have links in
/// em.
///
/// Returns `(epoch_secs, tz, label)` on success, `None` on any parse failure.
pub(crate) fn parse_alarm_entry(entry: &str) -> Option<(i64, String, Option<String>)> {
    let (prefix, epoch_str) = entry.rsplit_once(':')?;
    let epoch: i64 = epoch_str.trim().parse().ok()?;
    // TODO: Future mitch refactor this is jank af
    let (label, tz) = if let Some((lbl, tz)) = prefix.rsplit_once(':') {
        (Some(lbl.trim().to_string()), tz.trim().to_string())
    } else {
        (None, prefix.trim().to_string())
    };
    Some((epoch, tz, label))
}

/// Serialize a single alarm back into ^^^ what that expects to parse
///
/// Produces `LABEL:TZ:EPOCH` when a label is present, `TZ:EPOCH` otherwise for backwards compat.
pub(crate) fn format_alarm_entry(label_tz: &str, epoch: i64, label: Option<&str>) -> String {
    match label {
        Some(lbl) => format!("{}:{}:{}", lbl, label_tz, epoch),
        None => format!("{}:{}", label_tz, epoch),
    }
}

/// Build the shareable string that reconstructs the current world clock layout.
///
/// - **Native**: returns a CLI arg string you could use to get to this setup ex:
///   `command --app=world-clock --tz=UTC,America/New_York --sort=offset --sort-dir=asc --pinned=1720000000`
/// - **WASM**: returns a full URL using the current `window.location` origin +
///   pathname, e.g. `https://example.com/?app=world-clock&tz=UTC%2CAmerica%2FNew_York&sort=offset`
///
/// Alarms are serialized as `[LABEL:]TZ:EPOCH_SECS`. Sort is only included when a
/// non-None column is active. Pinned time is only included when the clock is
/// frozen aka you were actively looking at a historical time.
fn build_share_string(
    timezones: &[String],
    alarms: &[AlarmEntry],
    sort_col: SortColumn,
    sort_dir: SortDir,
    pinned_time: Option<Timestamp>,
) -> String {
    // Percent-encode only the characters that would otherwise break URL query parsing.
    // For IANA names the only special character in practice is '/', which becomes '%2F'.
    #[cfg(target_arch = "wasm32")]
    fn url_encode(s: &str) -> String {
        s.replace('/', "%2F").replace(' ', "%20")
    }

    let tz_joined = timezones.join(",");

    let alarm_parts: Vec<String> = alarms
        .iter()
        .map(|a| format_alarm_entry(&a.label_tz, a.target_ts.as_second(), a.label.as_deref()))
        .collect();

    #[cfg(not(target_arch = "wasm32"))]
    {
        // Resolve the executable name: try argv[0] first, if we were on $PATH
        // use that and the exe name, otherwise just use "mitchty" and hope for
        // the best I guess.
        let exe = std::env::args()
            .next()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .map(|p| p.display().to_string())
            })
            .unwrap_or_else(|| "mitchty".to_string());

        let mut parts = vec![exe, "--app=world-clock".to_string()];
        if !tz_joined.is_empty() {
            parts.push(format!("--tz={}", tz_joined));
        }
        for a in &alarm_parts {
            parts.push(format!("--alarm={}", a));
        }
        if let Some(col_slug) = sort_col.to_slug() {
            parts.push(format!("--sort={}", col_slug));
            // Only emit sort-dir when a sort column is active; asc is the default so
            // we always include it to be sure copy/paste is right
            parts.push(format!("--sort-dir={}", sort_dir.to_slug()));
        }
        if let Some(ts) = pinned_time {
            parts.push(format!("--pinned={}", ts.as_second()));
        }
        parts.join(" ")
    }

    #[cfg(target_arch = "wasm32")]
    {
        // WASM URL form using the browser's current origin + pathname.
        let base = web_sys::window()
            .and_then(|w| {
                let loc = w.location();
                let origin = loc.origin().ok()?;
                let pathname = loc.pathname().ok()?;
                Some(format!("{}{}", origin, pathname))
            })
            .unwrap_or_else(|| "/".to_string());

        let mut params = vec!["app=world-clock".to_string()];

        if !tz_joined.is_empty() {
            // Encode each tz name individually then rejoin with %2C which is encoded comma.
            let encoded_tzs: Vec<String> = timezones.iter().map(|t| url_encode(t)).collect();
            params.push(format!("tz={}", encoded_tzs.join("%2C")));
        }

        for a in &alarm_parts {
            // Split on the LAST colon: to mimic the clap arg parsing for
            // string:tz:epoch. Each colon in the prefix gets encoded as %3A and
            // slashes in tz names get encoded by url_encode as before.
            if let Some((prefix, epoch)) = a.rsplit_once(':') {
                let encoded_prefix = prefix
                    .split(':')
                    .map(url_encode)
                    .collect::<Vec<_>>()
                    .join("%3A");
                params.push(format!("alarm={}%3A{}", encoded_prefix, epoch));
            }
        }

        if let Some(col_slug) = sort_col.to_slug() {
            params.push(format!("sort={}", col_slug));
            params.push(format!("sort-dir={}", sort_dir.to_slug()));
        }

        if let Some(ts) = pinned_time {
            params.push(format!("pinned={}", ts.as_second()));
        }

        format!("{}?{}", base, params.join("&"))
    }
}

/// Write `text` to the system clipboard.
///
/// - Native: uses `arboard` for direct OS clipboard access.
/// - WASM: calls the async Clipboard API via `wasm_bindgen_futures::spawn_local`
///   fire-and-forget; the browser may show a permission prompt the first time
///   thats a user problem not mine.
fn copy_to_clipboard(_ctx: &egui::Context, text: String) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                if let Err(e) = cb.set_text(text.as_str()) {
                    bevy::log::warn!("clipboard write failed: {}", e);
                }
            }
            Err(e) => bevy::log::warn!("could not open clipboard: {}", e),
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        // egui's ctx.copy_text won't reach the browser clipboard from inside a
        // WebGL canvas on most browsers, so we go direct via the Web Clipboard API.
        if let Some(window) = web_sys::window() {
            let clipboard = window.navigator().clipboard();
            let promise = clipboard.write_text(&text);
            wasm_bindgen_futures::spawn_local(async move {
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
            });
        }
    }
}

// TODO: This is getting out of hand, need to start refactoring.
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

    let ctx = contexts.ctx_mut()?;

    // `visuals.dark_mode` is the single source of truth egui sets after
    // resolving the system preference, so this is correct for all three states
    // of ThemePreference Light/Dark/System
    //
    // This is a hack and its getting late but this logic is jank so I need to
    // probably figure out a better theming approach in general.
    let is_light_mode = !ctx.global_style().visuals.dark_mode;

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
        let (active_color, inactive_color) = if is_light_mode {
            // Dark indigo active, mid-grey inactive both readable on white background
            // TODO: Future me should make constants too many magic numbers but
            // I'm tired.
            (
                egui::Color32::from_rgb(60, 60, 180),
                egui::Color32::from_rgb(90, 90, 90),
            )
        } else {
            // Light-lavender palette works fine on dark background themes.
            (
                egui::Color32::from_rgb(220, 220, 255),
                egui::Color32::from_rgb(150, 150, 210),
            )
        };
        let text = egui::RichText::new(format!("{label}{indicator}"))
            .strong()
            .color(if cur_col == col {
                active_color
            } else {
                inactive_color
            });
        ui.button(text).clicked()
    };

    const CLOCK_RADIUS: f32 = 30.0;

    let mut open_edit: Option<(String, Timestamp, egui::Pos2)> = None;
    let mut open_alarm_edit = false;
    let mut alarm_edit_pos = egui::Pos2::ZERO;
    let mut remove_alarm: Option<usize> = None;

    let mut start_label_edit: Option<(usize, String)> = None;
    let mut commit_label_edit: Option<(usize, String)> = None;
    let mut cancel_label_edit = false;

    let mut go_live = false;
    let mut go_back = false;
    let mut go_forward = false;
    let mut clear_history = false;

    let has_history = !state.time_history.is_empty();
    let cursor = state.history_cursor;
    let can_go_back = (cursor > 0 || !is_pinned) && has_history;
    let can_go_forward = is_pinned && has_history && cursor + 1 < state.time_history.len();

    let mut open = true;

    egui::Window::new("World Clock")
        .open(&mut open)
        .auto_sized()
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let theme_text_color = if is_light_mode {
                    egui::Color32::BLACK
                } else {
                    egui::Color32::WHITE
                };

                // Copy-link/command button is first for no raisin.
                {
                    let now_ts = Timestamp::now();
                    let showing_feedback = state
                        .copy_feedback_until
                        .map(|until| now_ts < until)
                        .unwrap_or(false);

                    #[cfg(not(target_arch = "wasm32"))]
                    let copy_text = "📋 Copy Command";
                    #[cfg(target_arch = "wasm32")]
                    let copy_text = "📋 Copy Link";

                    // During feedback the button background goes green so the
                    // success is obvious regardless of theme; text still follows
                    // the theme color so it reads on both light and dark green.
                    let copy_btn = if showing_feedback {
                        egui::Button::new(egui::RichText::new("✔ Copied!").color(theme_text_color))
                            .fill(if is_light_mode {
                                egui::Color32::from_rgb(140, 210, 140)
                            } else {
                                egui::Color32::from_rgb(60, 160, 60)
                            })
                    } else {
                        egui::Button::new(egui::RichText::new(copy_text).color(theme_text_color))
                    };

                    #[cfg(not(target_arch = "wasm32"))]
                    let hover = "Copy a runnable command that reopens this layout";
                    #[cfg(target_arch = "wasm32")]
                    let hover = "Copy a shareable URL for this layout";

                    let copy_resp = ui.add(copy_btn).on_hover_text(hover);

                    if copy_resp.clicked() {
                        let link = build_share_string(
                            &state.timezones,
                            &state.alarms,
                            state.sort_col,
                            state.sort_dir,
                            state.pinned_time,
                        );
                        copy_to_clipboard(ctx, link);
                        // Show copied blurb for 2 seconds, then revert.
                        state.copy_feedback_until =
                            Some(Timestamp::from_second(now_ts.as_second() + 2).unwrap_or(now_ts));
                    }
                }

                ui.separator();

                // Alarm button next.
                let alarm_btn = ui
                    .button(egui::RichText::new("🔔 Set Alarm").color(theme_text_color))
                    .on_hover_text("Set a countdown alarm");
                if alarm_btn.clicked() {
                    open_alarm_edit = true;
                    alarm_edit_pos = alarm_btn.rect.left_bottom();
                }

                ui.separator();

                // History and status are at the right.
                if is_pinned || has_history {
                    // Status label.
                    if is_pinned {
                        ui.label(
                            egui::RichText::new("🔒 Pinned time not live")
                                .color(theme_text_color)
                                .strong(),
                        );
                    } else {
                        // We have history but currently are in live mode.
                        ui.label(
                            egui::RichText::new("🕐 Live")
                                .color(theme_text_color)
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
                            .button(egui::RichText::new("↩ Go Live").color(theme_text_color))
                            .on_hover_text("Return to live clock")
                            .clicked()
                    {
                        go_live = true;
                    }

                    // Clear all history.
                    if ui
                        .button(egui::RichText::new("🗑 Clear").color(theme_text_color))
                        .on_hover_text("Clear all pinned time history")
                        .clicked()
                    {
                        clear_history = true;
                    }
                } else {
                    // No history: keep the space empty on the right to avoid layout shifts.
                    ui.label(egui::RichText::new(" "));
                }
            });
            ui.add_space(4.0);

            // Outer 2-column layout using egui::Grid to ensure there is a rough
            // 2 column overall layout in egui
            egui::Grid::new("wc_outer_layout")
                .num_columns(2)
                .spacing([16.0, 0.0])
                .show(ui, |ui| {
                    egui::Grid::new("wc_grid")
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            ui.label(""); // analog clock column is first and has no real header to speak of
                            if header_btn(ui, "Timezone", SortColumn::Timezone, sort_col, sort_dir)
                            {
                                state.sort_col = cycle_sort(
                                    state.sort_col,
                                    SortColumn::Timezone,
                                    &mut state.sort_dir,
                                );
                            }
                            if header_btn(ui, "Date", SortColumn::Date, sort_col, sort_dir) {
                                state.sort_col = cycle_sort(
                                    state.sort_col,
                                    SortColumn::Date,
                                    &mut state.sort_dir,
                                );
                            }
                            if header_btn(ui, "Time", SortColumn::Time, sort_col, sort_dir) {
                                state.sort_col = cycle_sort(
                                    state.sort_col,
                                    SortColumn::Time,
                                    &mut state.sort_dir,
                                );
                            }
                            if header_btn(ui, "Offset", SortColumn::Offset, sort_col, sort_dir) {
                                state.sort_col = cycle_sort(
                                    state.sort_col,
                                    SortColumn::Offset,
                                    &mut state.sort_dir,
                                );
                            }
                            if local_offset_mins.is_some()
                                && header_btn(
                                    ui,
                                    "Δ Local",
                                    SortColumn::DeltaLocal,
                                    sort_col,
                                    sort_dir,
                                )
                            {
                                state.sort_col = cycle_sort(
                                    state.sort_col,
                                    SortColumn::DeltaLocal,
                                    &mut state.sort_dir,
                                );
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

                                // Local timezone: gold star + bold text (black
                                // in light, white in dark). Two labels inside a
                                // horizontal span so we can color the star
                                // independently from the name text.
                                if is_local {
                                    let text_color = if is_light_mode {
                                        egui::Color32::BLACK
                                    } else {
                                        egui::Color32::WHITE
                                    };
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("★")
                                                .strong()
                                                .color(egui::Color32::from_rgb(255, 210, 0)),
                                        );
                                        ui.label(
                                            egui::RichText::new(row.name.as_str())
                                                .strong()
                                                .color(text_color),
                                        );
                                    });
                                } else {
                                    ui.label(egui::RichText::new(row.name.as_str()));
                                }

                                if row.valid {
                                    let is_off_hours = row.hour < 8 || row.hour >= 17;
                                    let (normal_color, local_color) = if is_light_mode {
                                        (egui::Color32::from_rgb(60, 60, 60), egui::Color32::BLACK)
                                    } else if is_off_hours {
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
                                        let text_color = if is_light_mode {
                                            egui::Color32::BLACK
                                        } else {
                                            egui::Color32::WHITE
                                        };
                                        egui::RichText::new(&row.date).strong().color(text_color)
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
                                            // This is the reference row - no diff to show.
                                            ui.label(
                                                egui::RichText::new("+00:00")
                                                    .color(egui::Color32::GRAY),
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
                                            ui.label(
                                                egui::RichText::new(text).monospace().color(color),
                                            );
                                        }
                                    }
                                } else {
                                    ui.label(
                                        egui::RichText::new("invalid").color(egui::Color32::RED),
                                    );
                                    ui.label("-");
                                    ui.label("-");
                                    if local_offset_mins.is_some() {
                                        ui.label("");
                                    }
                                }

                                if ui
                                    .small_button(
                                        egui::RichText::new("✖").color(egui::Color32::RED),
                                    )
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

                    ui.vertical(|ui| {
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
                            // Cap scroll area to the same height as the left grid so
                            // neither column can inflate the horizontal row size.
                            let row_height = ui.text_style_height(&egui::TextStyle::Body)
                                + ui.spacing().item_spacing.y;
                            let content_height = state.filtered.len() as f32 * row_height;
                            // Cap to a sensible max so the list doesn't dwarf the timezone table in egui.
                            let scroll_height = content_height.clamp(40.0, 260.0);
                            egui::ScrollArea::vertical()
                                .max_height(scroll_height)
                                .min_scrolled_width(search_width)
                                .id_salt("wc_tz_picker")
                                .show(ui, |ui| {
                                    ui.set_max_width(search_width);
                                    let mut add_tz: Option<String> = None;
                                    for &tz_name in &state.filtered {
                                        let already_added =
                                            state.timezones.iter().any(|t| t == tz_name);
                                        ui.horizontal(|ui| {
                                            if already_added {
                                                ui.label(
                                                    egui::RichText::new(tz_name)
                                                        .color(egui::Color32::GRAY)
                                                        .italics(),
                                                );
                                            } else if ui.selectable_label(false, tz_name).clicked()
                                            {
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
                    ui.end_row();
                });

            // Alarms list
            // Active alarms list by time to alert and then expired alarms most
            // recently expired first.
            if !state.alarms.is_empty() {
                ui.add_space(8.0);
                ui.add_space(2.0);
                ui.label(egui::RichText::new("🔔 Alarms").strong());
                ui.add_space(2.0);

                // Build a sorted index list.
                // Active: ascending seconds remaining.
                // Expired: ascending seconds since expiry aka most recent first.
                // Expired entries always come after active alarms.
                let mut sorted_indices: Vec<usize> = (0..state.alarms.len()).collect();
                sorted_indices.sort_by_key(|&i| {
                    let diff = state.alarms[i].target_ts.as_second() - live_now.as_second();
                    if diff > 0 {
                        (0i64, diff)
                    } else {
                        (1i64, -diff)
                    }
                });

                egui::Grid::new("wc_alarms_grid")
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        for &idx in &sorted_indices {
                            // Copy out what we need before any mutable borrow of state happens.
                            let (target_ts, label_tz, alarm_label) = {
                                let e = &state.alarms[idx];
                                (e.target_ts, e.label_tz.clone(), e.label.clone())
                            };
                            let alarm_entry_target_ts = target_ts;
                            let alarm_entry_label_tz = label_tz;
                            let alarm_entry_label = alarm_label;
                            let is_expired = alarm_entry_target_ts <= live_now;
                            let status = if is_expired {
                                format_elapsed(alarm_entry_target_ts, live_now)
                            } else {
                                format_countdown(alarm_entry_target_ts, live_now)
                            };
                            let color = if is_expired {
                                egui::Color32::GRAY
                            } else if is_light_mode {
                                egui::Color32::BLACK
                            } else {
                                egui::Color32::WHITE
                            };
                            // Timezone + optional label, clickable so I can edit the alarm text.
                            if state.editing_label == Some(idx) {
                                // Inline edit field.
                                let edit_resp = ui.text_edit_singleline(&mut state.label_edit_buf);
                                // Auto-focus when first opened.
                                edit_resp.request_focus();
                                let pressed_enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                                let pressed_escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                if pressed_enter || edit_resp.lost_focus() && !pressed_escape {
                                    commit_label_edit = Some((idx, state.label_edit_buf.clone()));
                                } else if pressed_escape {
                                    cancel_label_edit = true;
                                }
                            } else {
                                let tz_label = if let Some(lbl) = &alarm_entry_label {
                                    format!("{} ({})", lbl, alarm_entry_label_tz)
                                } else {
                                    alarm_entry_label_tz.clone()
                                };
                                let resp = ui
                                    .selectable_label(
                                        false,
                                        egui::RichText::new(&tz_label).color(color),
                                    )
                                    .on_hover_text("Click to edit label");
                                if resp.clicked() {
                                    start_label_edit =
                                        Some((idx, alarm_entry_label.clone().unwrap_or_default()));
                                }
                            }
                            if let Some((time, date, ..)) =
                                format_tz(alarm_entry_target_ts, &alarm_entry_label_tz)
                            {
                                ui.label(
                                    egui::RichText::new(format!("{date}  {time}"))
                                        .monospace()
                                        .color(color),
                                );
                            } else {
                                ui.label("");
                            }
                            ui.label(egui::RichText::new(":").color(egui::Color32::GRAY));
                            ui.label(egui::RichText::new(&status).monospace().color(color));
                            if ui
                                .small_button(egui::RichText::new("✖").color(egui::Color32::RED))
                                .on_hover_text("Remove alarm")
                                .clicked()
                            {
                                remove_alarm = Some(idx);
                            }
                            ui.end_row();
                        }
                    });
            }
        });

    // Remove an alarm if user requested.
    if let Some(idx) = remove_alarm
        && idx < state.alarms.len()
    {
        state.alarms.remove(idx);
    }

    // Open inline label so a user can edit the clicked alarm optional text.
    if let Some((idx, current)) = start_label_edit {
        state.editing_label = Some(idx);
        state.label_edit_buf = current;
    }

    // Commit inline label edits.
    if let Some((idx, new_label)) = commit_label_edit
        && idx < state.alarms.len()
    {
        let trimmed = new_label.trim().to_string();
        state.alarms[idx].label = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };
        state.editing_label = None;
        state.label_edit_buf = String::new();
    }

    // Cancel inline label edit.
    if cancel_label_edit {
        state.editing_label = None;
        state.label_edit_buf = String::new();
    }

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

    // Open the alarm picker when the button was clicked.
    if open_alarm_edit {
        if state.editing_alarm.is_some() {
            state.editing_alarm = None;
        } else {
            let tz = state
                .timezones
                .first()
                .cloned()
                .unwrap_or_else(|| "UTC".to_string());
            state.editing_alarm = Some(AlarmState::new(live_now, &tz, alarm_edit_pos));
        }
    }

    // Render and handle the alarm picker popup.
    let tzs_snapshot = state.timezones.clone();
    let mut close_alarm_edit = false;
    if let Some(alarm_edit) = state.editing_alarm.as_mut() {
        let (confirmed, cancelled) = draw_alarm_popup(ctx, alarm_edit, live_now, &tzs_snapshot);
        if let Some((ts, tz, label)) = confirmed {
            state.alarms.push(AlarmEntry {
                target_ts: ts,
                label_tz: tz,
                label,
            });
            close_alarm_edit = true;
        } else if cancelled {
            close_alarm_edit = true;
        }
    }
    if close_alarm_edit {
        state.editing_alarm = None;
    }

    if !open && let Ok(entity) = show_query.single() {
        commands.entity(entity).despawn();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{format_alarm_entry, parse_alarm_entry};

    #[test]
    fn parse_two_form_plain_tz() {
        // Ye olde tz:epoch form
        let (epoch, tz, label) =
            parse_alarm_entry("UTC:1893456000").expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1893456000);
        assert_eq!(tz, "UTC");
        assert_eq!(label, None);
    }

    #[test]
    fn parse_two_form_iana_slash_tz() {
        // IANA tz names contain '/'s which we don't want to parse as a uri indicator
        let (epoch, tz, label) =
            parse_alarm_entry("America/New_York:1893456000").expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1893456000);
        assert_eq!(tz, "America/New_York");
        assert_eq!(label, None);
    }

    #[test]
    fn parse_three_form_with_label() {
        // new hotness 3 form with a label, such features!
        let (epoch, tz, label) = parse_alarm_entry("Birthday:America/New_York:1893456000")
            .expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1893456000);
        assert_eq!(tz, "America/New_York");
        assert_eq!(label, Some("Birthday".to_string()));
    }

    #[test]
    fn parse_three_form_label_with_space() {
        // Labels may contain spaces too cause yeah
        let (epoch, tz, label) = parse_alarm_entry("My Birthday:America/Chicago:1775012160")
            .expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1775012160);
        assert_eq!(tz, "America/Chicago");
        assert_eq!(label, Some("My Birthday".to_string()));
    }

    #[test]
    fn parse_three_form_iana_tz_path() {
        // Some IANA names have two slashes e.g. America/Indiana/Indianapolis
        let (epoch, tz, label) =
            parse_alarm_entry("Meeting:America/Indiana/Indianapolis:1893456000")
                .expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1893456000);
        assert_eq!(tz, "America/Indiana/Indianapolis");
        assert_eq!(label, Some("Meeting".to_string()));
    }

    #[test]
    fn parse_trims_whitespace() {
        //  Tolerate surrounding whitespace from comma-split entries just in
        //  case someone starts building these manually like a weirdo.
        let (epoch, tz, label) =
            parse_alarm_entry("  UTC : 1893456000 ").expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1893456000);
        // The tz/label trimming is on the prefix/suffix, not mid-name
        assert_eq!(tz, "UTC");
        assert_eq!(label, None);
    }

    #[test]
    fn parse_bad_epoch_returns_none() {
        assert!(parse_alarm_entry("America/New_York:notanumber").is_none());
    }

    #[test]
    fn parse_missing_colon_returns_none() {
        // No colon at all means we cannot split epoch from tz
        assert!(parse_alarm_entry("1893456000").is_none());
    }

    #[test]
    fn parse_empty_string_returns_none() {
        assert!(parse_alarm_entry("").is_none());
    }

    #[test]
    fn parse_epoch_only_colon_prefix_empty() {
        // I think in this instance maybe we just default to UTC here?
        // TODO: future mitch brain on it longer
        // ":1893456000" prefix is empty string, tz would be "", which is
        // technically Some("") but we still return it (callers validate the tz)
        let (epoch, tz, label) =
            parse_alarm_entry(":1893456000").expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1893456000);
        assert_eq!(tz, "");
        assert_eq!(label, None);
    }

    #[test]
    fn format_no_label_two_form() {
        assert_eq!(
            format_alarm_entry("UTC", 1893456000, None),
            "UTC:1893456000"
        );
    }

    #[test]
    fn format_no_label_iana_slash() {
        assert_eq!(
            format_alarm_entry("America/New_York", 1893456000, None),
            "America/New_York:1893456000"
        );
    }

    #[test]
    fn format_with_label_three_form() {
        assert_eq!(
            format_alarm_entry("America/New_York", 1893456000, Some("Birthday")),
            "Birthday:America/New_York:1893456000"
        );
    }

    #[test]
    fn format_with_label_with_space() {
        assert_eq!(
            format_alarm_entry("America/Chicago", 1775012160, Some("My Birthday")),
            "My Birthday:America/Chicago:1775012160"
        );
    }

    // God I hope none of this ever fails but in case it does make sure it round
    // trips fine and hopefully I got enough tests to make this useful in future.
    #[test]
    fn round_trip_no_label() {
        let serialized = format_alarm_entry("Asia/Tokyo", 1893456000, None);
        let (epoch, tz, label) = parse_alarm_entry(&serialized).expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1893456000);
        assert_eq!(tz, "Asia/Tokyo");
        assert_eq!(label, None);
    }

    #[test]
    fn round_trip_with_label() {
        let serialized = format_alarm_entry("Europe/Paris", 1893456000, Some("New Year"));
        let (epoch, tz, label) = parse_alarm_entry(&serialized).expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1893456000);
        assert_eq!(tz, "Europe/Paris");
        assert_eq!(label, Some("New Year".to_string()));
    }

    #[test]
    fn round_trip_iana_tz_with_label() {
        let serialized =
            format_alarm_entry("America/Indiana/Indianapolis", 1893456000, Some("Sprint"));
        let (epoch, tz, label) = parse_alarm_entry(&serialized).expect("parse_alarm_entry failed");
        assert_eq!(epoch, 1893456000);
        assert_eq!(tz, "America/Indiana/Indianapolis");
        assert_eq!(label, Some("Sprint".to_string()));
    }
}
