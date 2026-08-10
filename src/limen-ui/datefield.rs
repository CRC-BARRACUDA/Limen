//! The date field, its calendar and its clock.

use crate::*;

/// A date field: type it, or pick it off a calendar.
///
/// Both, rather than either. Typing is faster once you know the format and is
/// the only way to clear the field; the calendar is how you answer "the Tuesday
/// before last" without counting. Returns `true` when the value changed.
///
/// The field and the button are laid out by hand, with the button's width taken
/// off the top. A text field asks for every point of width there is, so putting
/// one beside anything else and hoping is how a container ends up wider than it
/// declared itself to be.
/// A date, typed or picked off a calendar — and, where the field carries one, a
/// time.
///
/// Three parts on purpose: the field itself, a calendar, and a clock. A date and
/// a time are different questions, so they get their own buttons and their own
/// windows; this decides only how much room each takes and hands the same value
/// to both.
pub fn date_field(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut String,
    placeholder: &str,
    with_time: bool,
) -> bool {
    let mut changed = false;
    let btn_w = 34.0;
    let gap = ui.spacing().item_spacing.x;
    let full = ui.available_width();
    let buttons = if with_time { 2.0 } else { 1.0 };

    let (cal_btn, clock_btn) = ui
        .horizontal(|ui| {
            let field_w = (full - buttons * (btn_w + gap)).max(80.0);
            let mut text = value.clone();
            let edit = ui.add_sized(
                egui::vec2(field_w, 0.0),
                egui::TextEdit::singleline(&mut text)
                    .hint_text(placeholder)
                    .id_source(id),
            );
            if edit.changed() {
                *value = text;
                changed = true;
            }
            let cal = icon_toggle(ui, btn_w, paint_calendar_icon);
            let clock = with_time.then(|| icon_toggle(ui, btn_w, paint_clock_icon));
            (cal, clock)
        })
        .inner;

    changed |= calendar_popup(ui, id, &cal_btn, value, with_time);
    if let Some(clock) = clock_btn {
        changed |= time_popup(ui, id, &clock, value);
    }
    changed
}

/// One of the small square buttons that opens a field's window: the frame and
/// the hover, with paint drawing whatever it stands for.
pub fn icon_toggle(
    ui: &mut egui::Ui,
    size: f32,
    paint: fn(&egui::Painter, egui::Rect, egui::Color32),
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, 28.0), egui::Sense::click());
    let a = interact(ui, &resp);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::Rounding::ZERO,
        lerp_color(color::BG_ELEVATED, color::BG_WIDGET, a.hover * 0.5),
    );
    painter.rect_stroke(
        rect,
        egui::Rounding::ZERO,
        egui::Stroke::new(1.5_f32, lerp_color(color::BORDER, color::ACCENT, a.hover)),
    );
    paint(
        painter,
        rect,
        lerp_color(color::TEXT_MUTED, color::ACCENT, a.hover),
    );
    resp
}

/// A little calendar: a head rail, two hangers, and a grid of days.
pub fn paint_calendar_icon(painter: &egui::Painter, rect: egui::Rect, ink: egui::Color32) {
    let stroke = egui::Stroke::new(1.2_f32, ink);
    let g = egui::Rect::from_center_size(rect.center(), egui::vec2(15.0, 15.0));
    painter.rect_stroke(g, egui::Rounding::ZERO, stroke);
    painter.line_segment(
        [
            egui::pos2(g.left(), g.top() + 4.0),
            egui::pos2(g.right(), g.top() + 4.0),
        ],
        stroke,
    );
    for i in 0..2 {
        let x = g.left() + 4.0 + i as f32 * 7.0;
        painter.line_segment(
            [egui::pos2(x, g.top() - 2.0), egui::pos2(x, g.top() + 1.5)],
            stroke,
        );
    }
    for r in 0..2 {
        for c in 0..3 {
            let p = egui::pos2(
                g.left() + 3.5 + c as f32 * 4.0,
                g.top() + 8.0 + r as f32 * 4.0,
            );
            painter.circle_filled(p, 0.9, ink);
        }
    }
}

/// A clock, hands at ten past ten — which is how one is drawn when it stands for
/// the idea of a clock rather than telling the time.
pub fn paint_clock_icon(painter: &egui::Painter, rect: egui::Rect, ink: egui::Color32) {
    let stroke = egui::Stroke::new(1.2_f32, ink);
    let c = rect.center();
    painter.circle_stroke(c, 7.0, stroke);
    painter.line_segment([c, c + egui::vec2(0.0, -4.5)], stroke);
    painter.line_segment([c, c + egui::vec2(3.2, 1.6)], stroke);
}

/// The shell both of a field's windows sit in.
///
/// It owns the window rather than using egui's popup helper, because a popup's
/// contents stop being called the moment it closes and there is then nowhere
/// left to draw a closing animation. It also makes the two exclusive: one key
/// holds which is open, so opening the clock closes the calendar without either
/// of them knowing the other exists.
///
/// The closure is handed the eased opening factor, and a flag it can set to mean
/// "done here".
pub fn field_popup(
    ui: &mut egui::Ui,
    id: &str,
    key: &str,
    anchor: &egui::Response,
    add: impl FnOnce(&mut egui::Ui, f32, &mut bool),
) {
    let state = egui::Id::new(("limen_field_popup", id));
    let mut showing = ui.data(|d| d.get_temp::<String>(state)).unwrap_or_default();
    if anchor.clicked() {
        showing = if showing == key {
            String::new()
        } else {
            key.to_string()
        };
        ui.data_mut(|d| d.insert_temp(state, showing.clone()));
    }
    let t = smoothstep(anim_bool(ui, anchor.id.with("open"), showing == key, 0.16));
    if t <= 0.002 {
        return;
    }

    let mut close = false;
    let shown = egui::Area::new(ui.make_persistent_id(("limen_field_area", id, key)))
        .order(FIELD_POPUP_ORDER)
        // Below the field it belongs to, and kept on screen near an edge.
        .fixed_pos(anchor.rect.left_bottom() + egui::vec2(-6.0, 4.0))
        .constrain(true)
        .show(ui.ctx(), |ui| {
            // It fades as it arrives, so opening reads as the window dropping
            // out of the field rather than blinking into existence beside it.
            ui.set_opacity(t);
            egui::Frame::none()
                .fill(color::BG_ELEVATED)
                .stroke(egui::Stroke::new(1.0_f32, color::BORDER))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| add(ui, t, &mut close));
        });
    // The same easing the pop-up windows use, so these open like everything else
    // in the application does.
    pop_layer(ui.ctx(), shown.response.layer_id, shown.response.rect, t);

    // Dismissed by Escape, or by a press anywhere that is neither the window nor
    // the button that opened it.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if ui.input(|i| i.pointer.any_pressed())
        && let Some(p) = ui.ctx().pointer_interact_pos()
        && !shown.response.rect.contains(p)
        && !anchor.rect.contains(p)
    {
        close = true;
    }
    if close {
        ui.data_mut(|d| d.insert_temp(state, String::new()));
    }
}

/// The calendar: a month of days, the months of a year, or a page of years.
pub fn calendar_popup(
    ui: &mut egui::Ui,
    id: &str,
    anchor: &egui::Response,
    value: &mut String,
    with_time: bool,
) -> bool {
    let mut changed = false;
    let view_key = egui::Id::new(("limen_date_view", id));
    field_popup(ui, id, "calendar", anchor, |ui, open_t, close| {
        ui.set_width(GRID_W);
        let now = date::today();
        // Where to open, and what is actually selected. They are not the
        // same: an empty field opens on today but has nothing chosen, and
        // marking today as the selection there would claim a date the user
        // never gave.
        let selected = date::parse(value);
        let current = selected.unwrap_or(now);
        let (mut y, mut m, mut mode) = ui.data_mut(|d| {
            *d.get_temp_mut_or_insert_with(view_key, || (current.year, current.month, LEVEL_DAYS))
        });
        // What it was showing when this frame began. Movement is decided by
        // comparing against this, within the frame — comparing against a
        // value stored last frame means one disagreement leaves the clock
        // being reset on every frame, and an animation that never advances.
        let opened_on = (mode, y, m);
        // ‹ and › on the outside, the month and the year between them.
        let mut step = 0i64;
        // Measured, so a cell is as wide as the word inside it — "Листопад"
        // and "May" are not the same width in any font.
        let width_of = |ui: &egui::Ui, text: &str| {
            let font = egui::TextStyle::Button.resolve(ui.style());
            ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font, color::TEXT).size().x) + 18.0
        };
        // ‹ hard against the left edge, › hard against the right, the month
        // and the year centred between them. Laid out from one allocated
        // rect rather than by nesting: a nested region is free to shrink to
        // its content, which leaves the arrows huddled around the middle
        // instead of out on the sides where they were asked to be.
        let row_h = 24.0;
        let arrow = egui::vec2(26.0, row_h);
        let (bar, _) = ui.allocate_exact_size(egui::vec2(GRID_W, row_h), egui::Sense::hover());
        let at = |ui: &mut egui::Ui,
                  r: egui::Rect,
                  key: &str,
                  text: &str,
                  chosen: bool|
         -> egui::Response {
            let mut cui = ui.child_ui_with_id_source(
                r,
                egui::Layout::left_to_right(egui::Align::Center),
                view_key.with(key),
                None,
            );
            calendar_cell(&mut cui, r.size(), text, chosen, false, open_t, 1.0)
        };
        // The arrows first, because what they do decides how everything
        // below them moves.
        if at(
            ui,
            egui::Rect::from_min_size(bar.left_top(), arrow),
            "prev",
            "‹",
            false,
        )
        .clicked()
        {
            step = -1;
        }
        if at(
            ui,
            egui::Rect::from_min_size(egui::pos2(bar.right() - arrow.x, bar.top()), arrow),
            "next",
            "›",
            false,
        )
        .clicked()
        {
            step = 1;
        }
        // What a step means depends on what is on show.
        match (mode, step) {
            (LEVEL_DAYS, s) if s != 0 => {
                let z = (m as i64 - 1) + s;
                y += z.div_euclid(12);
                m = (z.rem_euclid(12) + 1) as u32;
            }
            // A page of months is a year; a page of years is a page.
            (LEVEL_MONTHS, s) => y += s,
            (LEVEL_YEARS, s) => y += s * YEARS_SHOWN as i64,
            _ => {}
        }
        let motion_key = view_key.with("motion");
        let now_t = ui.input(|i| i.time);
        // Anything the arrows just did starts a movement now, so the labels
        // and the grid below both carry it on this same frame.
        if (mode, y, m) != opened_on {
            let (dir, sc) = calendar_motion(opened_on, (mode, y, m));
            ui.data_mut(|d| d.insert_temp(motion_key, (dir, sc, now_t)));
        }
        let (dir, from_scale, start) = ui
            .data(|d| d.get_temp::<(f32, f32, f64)>(motion_key))
            // Never moved: long finished, rather than about to start.
            .unwrap_or((0.0, 1.0, f64::NEG_INFINITY));
        let mt = {
            let raw = ((now_t - start) / MOTION_SECS).clamp(0.0, 1.0) as f32;
            if animations_enabled() {
                ease_out(raw)
            } else {
                1.0
            }
        };
        if mt < 1.0 {
            ui.ctx().request_repaint();
        }
        // Slide from the side it came from, and scale toward its resting
        // size. Both settle at nothing, so a still calendar is not
        // transformed at all.
        let dx = (1.0 - mt) * dir * GRID_W * 0.30;
        let scale = from_scale + (1.0 - from_scale) * mt;
        // Everything drawn from here on can still change what the calendar
        // is showing — the month and year labels open their pickers, the
        // grid picks a year or a month. Snapshot before any of them, or a
        // change made below goes unnoticed and arrives without a movement.
        let drawn_with = (mode, y, m);
        // Now the labels, carried by the same slide as the grid — at half
        // the distance, so the name travels with the days it names without
        // racing them.
        let month_name = tr(&format!("cal.month_{m}"));
        let year_text = y.to_string();
        let mw = width_of(ui, &month_name);
        let yw = width_of(ui, &year_text);
        let gap = ui.spacing().item_spacing.x;
        let mx = bar.left() + header_pair_x(GRID_W, mw, yw, gap);
        let hdx = dx * 0.5;
        if at(
            ui,
            egui::Rect::from_min_size(
                egui::pos2(mx + hdx, bar.top()),
                egui::vec2(mw, row_h),
            ),
            "month",
            &month_name,
            mode == LEVEL_MONTHS,
        )
        .clicked()
        {
            mode = if mode == LEVEL_MONTHS {
                LEVEL_DAYS
            } else {
                LEVEL_MONTHS
            };
        }
        if at(
            ui,
            egui::Rect::from_min_size(
                egui::pos2(mx + mw + gap + hdx, bar.top()),
                egui::vec2(yw, row_h),
            ),
            "year",
            &year_text,
            mode == LEVEL_YEARS,
        )
        .clicked()
        {
            mode = if mode == LEVEL_YEARS {
                LEVEL_DAYS
            } else {
                LEVEL_YEARS
            };
        }
        // Each cell's slot in the opening: the grid fills in across itself
        // rather than every square landing at once.
        let slot = |i: usize, n: usize| {
            ((open_t * 1.6) - i as f32 * (0.5 / n.max(1) as f32)).clamp(0.0, 1.0)
        };
        let lead = date::weekday(y, m, 1) as usize;
        let days_in = date::days_in_month(y, m);
        let grid_h = calendar_height(mode, y, m);
        let h = animate_back(ui.ctx(), view_key.with("h"), grid_h, 0.22);
        let (outer, _) = ui.allocate_exact_size(egui::vec2(GRID_W, h), egui::Sense::hover());
        // Scaled about the middle, so a zoom happens in place rather than
        // dragging the grid toward one corner.
        let inset = GRID_W * (1.0 - scale) * 0.5;
        let mut grid_ui = ui.child_ui_with_id_source(
            egui::Rect::from_min_size(
                outer.min + egui::vec2(dx + inset, 0.0),
                egui::vec2(GRID_W * scale, grid_h * scale),
            ),
            egui::Layout::top_down(egui::Align::Min),
            view_key.with("grid"),
            None,
        );
        // Clipped to the space actually allocated, so a grid arriving from
        // the side slides in from behind the edge instead of over it.
        grid_ui.set_clip_rect(grid_ui.clip_rect().intersect(outer));
        // Fade in with the move; a slide that does not fade reads as a
        // jump. Never below a floor, so a calendar that is standing still
        // can only ever be fully drawn.
        grid_ui.set_opacity(mt.max(0.35));
        let mut picked_day = None;
        {
            let ui = &mut grid_ui;
            match mode {
            // Years.
            LEVEL_YEARS => {
                let first = y - y.rem_euclid(YEARS_SHOWN as i64);
                egui::Grid::new(("limen_date_years", id))
                    .num_columns(4)
                    .spacing(egui::vec2(2.0, 2.0))
                    .show(ui, |ui| {
                        for i in 0..YEARS_SHOWN {
                            let yy = first + i as i64;
                            let t = slot(i, YEARS_SHOWN);
                            if calendar_cell(
                                ui,
                                YEAR_CELL * scale,
                                &yy.to_string(),
                                selected.is_some_and(|d| d.year == yy),
                                yy == now.year,
                                t,
                                scale,
                            )
                            .clicked()
                            {
                                y = yy;
                                mode = LEVEL_MONTHS;
                            }
                            if (i + 1).is_multiple_of(4) {
                                ui.end_row();
                            }
                        }
                    });
            }
            // Months.
            LEVEL_MONTHS => {
                egui::Grid::new(("limen_date_months", id))
                    .num_columns(3)
                    .spacing(egui::vec2(2.0, 2.0))
                    .show(ui, |ui| {
                        for i in 0..12u32 {
                            let name = tr(&format!("cal.month_{}", i + 1));
                            if calendar_cell(
                                ui,
                                MONTH_CELL * scale,
                                &name,
                                selected.is_some_and(|d| (d.year, d.month) == (y, i + 1)),
                                (y, i + 1) == (now.year, now.month),
                                slot(i as usize, 12),
                                scale,
                            )
                            .clicked()
                            {
                                m = i + 1;
                                mode = LEVEL_DAYS;
                            }
                            if (i + 1).is_multiple_of(3) {
                                ui.end_row();
                            }
                        }
                    });
            }
            // Days.
            _ => {
                egui::Grid::new(("limen_date_days", id))
                    .num_columns(7)
                    .spacing(egui::vec2(2.0, 2.0))
                    .show(ui, |ui| {
                        for wd in 1..=7 {
                            ui.add_sized(
                                egui::vec2(CELL.x * scale, 18.0 * scale),
                                egui::Label::new(
                                    egui::RichText::new(tr(&format!("cal.wd_{wd}")))
                                        .weak()
                                        .small(),
                                ),
                            );
                        }
                        ui.end_row();
                        // Monday-first, so the leading blanks are however far
                        // into the week the first of the month falls.
                        for _ in 0..lead {
                            ui.allocate_exact_size(CELL * scale, egui::Sense::hover());
                        }
                        let days = days_in;
                        let mut col = lead;
                        for day in 1..=days {
                            let is_today = (y, m, day) == (now.year, now.month, now.day);
                            let chosen = selected
                                .is_some_and(|c| (c.year, c.month, c.day) == (y, m, day));
                            let i = lead + day as usize;
                            if calendar_cell(
                                ui,
                                CELL * scale,
                                &day.to_string(),
                                chosen,
                                is_today,
                                slot(i, lead + days as usize),
                                scale,
                            )
                            .clicked()
                            {
                                picked_day = Some(day);
                            }
                            col += 1;
                            if col.is_multiple_of(7) {
                                ui.end_row();
                            }
                        }
                    });
            }
            }
        }
        if let Some(day) = picked_day {
            // The time already in the field is kept: a date picker should
            // not silently move a boundary from 23:59:59 back to midnight.
            let keep = date::parse(value).unwrap_or(now);
            *value = date::format(
                Date {
                    year: y,
                    month: m,
                    day,
                    ..keep
                },
                with_time,
            );
            changed = true;
            ui.memory_mut(|mem| mem.close_popup());
        }
        // A pick inside the grid — a year, a month — only shows on the
        // next frame, so its movement starts here for that frame to carry.
        if (mode, y, m) != drawn_with {
            let (dir, sc) = calendar_motion(drawn_with, (mode, y, m));
            ui.data_mut(|d| d.insert_temp(motion_key, (dir, sc, now_t)));
        }
        ui.data_mut(|d| d.insert_temp(view_key, (y, m, mode)));
        ui.add_space(2.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui.small_button(tr("cal.today")).clicked() {
                *value = date::format(date::today(), with_time);
                changed = true;
                *close = true;
            }
            // Clearing has to be possible: an empty bound means "no bound",
            // which is a real answer and not the same as today.
            if ui.small_button(tr("cal.clear")).clicked() {
                value.clear();
                changed = true;
                *close = true;
            }
        });
    });
    changed
}

/// The clock: hours, minutes and seconds, and the two ends of a day.
pub fn time_popup(ui: &mut egui::Ui, id: &str, anchor: &egui::Response, value: &mut String) -> bool {
    let mut changed = false;
    field_popup(ui, id, "time", anchor, |ui, _t, close_time| {
                ui.set_width(TIME_W);
                // A field with no date yet still has a time to set,
                // so it starts from today rather than refusing.
                let mut at = date::parse(value).unwrap_or(date::today());
                // Laid out on a fixed grid rather than by letting
                // the columns size themselves: "Години" and
                // "Хвилини" are different widths, so content-sized
                // columns drift apart and leave the colons sitting
                // between nothing in particular.
                const COL: f32 = 66.0;
                /// The number box itself, narrower than its column
                /// so the three sit apart rather than touching.
                const BOX_W: f32 = 48.0;
                const SEP: f32 = 18.0;
                const ROW_H: f32 = 26.0;
                const CAP_H: f32 = 16.0;
                let span = 3.0 * COL + 2.0 * SEP;
                let (block, _) = ui.allocate_exact_size(
                    egui::vec2(TIME_W, ROW_H + 3.0 + CAP_H),
                    egui::Sense::hover(),
                );
                let x0 = block.left() + (TIME_W - span) * 0.5;
                let mut edited = false;
                for (i, (v, max, cap)) in [
                    (&mut at.hour, 23u32, "cal.hours"),
                    (&mut at.minute, 59, "cal.minutes"),
                    (&mut at.second, 59, "cal.seconds"),
                ]
                .into_iter()
                .enumerate()
                {
                    let r = egui::Rect::from_min_size(
                        egui::pos2(x0 + i as f32 * (COL + SEP), block.top()),
                        egui::vec2(COL, ROW_H),
                    );
                    // The box is placed at the middle of its
                    // column and sized to fill what it is given.
                    // Asking a layout to centre it does not work: a
                    // main alignment decides how a region is sized,
                    // not where a widget sits inside one, so every
                    // box ended up against its column's left edge
                    // while the caption underneath was centred.
                    let br = egui::Rect::from_center_size(
                        r.center(),
                        egui::vec2(BOX_W, ROW_H),
                    );
                    let mut cui = ui.child_ui_with_id_source(
                        br,
                        egui::Layout::left_to_right(egui::Align::Center),
                        ("limen_time_part", i),
                        None,
                    );
                    edited |= cui
                        .add_sized(
                            br.size(),
                            egui::DragValue::new(v)
                                .range(0..=max)
                                .speed(0.15)
                                // Always two digits, so the numbers
                                // do not jump width as they change.
                                .custom_formatter(|n, _| format!("{:02}", n as u32)),
                        )
                        .changed();
                    // The caption, centred under its own column.
                    ui.painter().text(
                        egui::pos2(r.center().x, block.bottom() - CAP_H * 0.5),
                        egui::Align2::CENTER_CENTER,
                        tr(cap),
                        egui::FontId::proportional(10.5),
                        color::TEXT_MUTED,
                    );
                    // The colon, in the gap and on the numbers'
                    // centre line rather than the block's.
                    if i < 2 {
                        ui.painter().text(
                            egui::pos2(r.right() + SEP * 0.5, r.center().y),
                            egui::Align2::CENTER_CENTER,
                            ":",
                            egui::TextStyle::Button.resolve(ui.style()),
                            color::TEXT_MUTED,
                        );
                    }
                }
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(2.0);
                // The two ends of a day, which is what a bound usually wants:
                // everything from, or everything up to.
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button(tr("cal.day_start")).clicked() {
                            at.hour = 0;
                            at.minute = 0;
                            at.second = 0;
                            edited = true;
                            *close_time = true;
                        }
                        if ui.small_button(tr("cal.day_end")).clicked() {
                            at.hour = 23;
                            at.minute = 59;
                            at.second = 59;
                            edited = true;
                            *close_time = true;
                        }
                    });
                });
                if edited {
                    *value = date::format(at, true);
                    changed = true;
                }
    });
    changed
}



/// The three levels a calendar can be showing, counted so that 1 is the closest
/// in. Moving toward 1 zooms in; moving away zooms out, and further is more —
/// stepping from years straight past months to days is a bigger movement than
/// one level.
pub const LEVEL_DAYS: u8 = 1;
pub const LEVEL_MONTHS: u8 = 2;
pub const LEVEL_YEARS: u8 = 3;

/// How much one level of zoom is worth. Two levels is twice this.
pub const ZOOM_PER_LEVEL: f32 = 0.14;

/// How long a slide or a zoom takes. Short enough not to be waited on, long
/// enough to show which way the calendar moved.
pub const MOTION_SECS: f64 = 0.22;

/// Years offered at once in the year picker: three rows of four.
pub const YEARS_SHOWN: usize = 12;

/// Which layer a field's own pop-up — a calendar, a clock — is drawn on.
///
/// Above the panels, so it covers the form it belongs to, but *below* the veil
/// a modal window lays down, which sits at `Middle`. On `Foreground` it would
/// share a layer with the modal box itself and float over the dimming: a
/// calendar still bright and still clickable on top of "do you really want to
/// quit?".
pub const FIELD_POPUP_ORDER: egui::Order = egui::Order::PanelResizeLine;

/// The time picker's width. Wide enough for the two day-end buttons side by
/// side, which is the widest thing in it.
pub const TIME_W: f32 = 250.0;

/// The calendar's own width: seven day cells and the gaps between them. Every
/// other mode is laid out to fit inside it.
pub const GRID_W: f32 = 7.0 * CELL.x + 6.0 * 2.0;

/// Every cell is the same size, in every mode. Sized rather than content-sized
/// so the numbers sit in the middle of their squares — a button that shrinks to
/// its label puts `1` and `31` in visibly different boxes.
pub const CELL: egui::Vec2 = egui::vec2(34.0, 30.0);
/// Three month names across the same width, and four years.
pub const MONTH_CELL: egui::Vec2 = egui::vec2((GRID_W - 4.0) / 3.0, 30.0);
pub const YEAR_CELL: egui::Vec2 = egui::vec2((GRID_W - 6.0) / 4.0, 30.0);

/// How a change from one calendar view to another should move.
///
/// Returns which way to slide (-1 left, +1 right, 0 not at all) and the scale to
/// arrive from. Sideways within a level slides; changing level zooms — out
/// toward the coarser view, in toward the finer one — which is how every
/// calendar people already use behaves.
pub fn calendar_motion(from: (u8, i64, u32), to: (u8, i64, u32)) -> (f32, f32) {
    let (fm, fy, fmo) = from;
    let (tm, ty, tmo) = to;
    if fm != tm {
        // Levels count from 1 at the closest in. Moving toward 1 grows into
        // place; moving away settles back out of it, and two levels at once is
        // twice as much movement as one.
        let steps = (tm as i32 - fm as i32) as f32;
        return (0.0, 1.0 + steps * ZOOM_PER_LEVEL);
    }
    if (fy, fmo) == (ty, tmo) {
        return (0.0, 1.0);
    }
    // The new month comes from the side it lives on — earlier from the left,
    // later from the right. Compared as a pair, so December to January is
    // forwards rather than a jump backwards through the month number.
    let back = (fy, fmo) > (ty, tmo);
    (if back { -1.0 } else { 1.0 }, 1.0)
}

/// Where the month/year pair starts, measured from the left of the header.
///
/// Centred as a pair rather than each on its own, so the year does not wander
/// as the month name changes length — between languages, or across a year.
pub fn header_pair_x(bar_w: f32, month_w: f32, year_w: f32, gap: f32) -> f32 {
    (bar_w - (month_w + gap + year_w)) * 0.5
}

/// How tall the calendar's grid is in a given mode.
///
/// The day grid is the variable one: a month spans four, five or six weeks
/// depending on how far into the week it opens, so the popup is a different
/// height in February than in August. Pulled out of the drawing so the
/// arithmetic can be checked without one.
pub fn calendar_height(mode: u8, y: i64, m: u32) -> f32 {
    const GAP: f32 = 2.0;
    match mode {
        LEVEL_YEARS => 3.0 * YEAR_CELL.y + 2.0 * GAP,
        LEVEL_MONTHS => 4.0 * MONTH_CELL.y + 3.0 * GAP,
        _ => {
            let lead = date::weekday(y, m, 1) as usize;
            let rows = (lead + date::days_in_month(y, m) as usize).div_ceil(7) as f32;
            // The weekday headings, then however many weeks this month spans.
            18.0 + GAP + rows * CELL.y + (rows - 1.0) * GAP
        }
    }
}

/// One cell of the calendar — a day, a month, or a year.
///
/// Its own widget rather than an `egui::Button` so it reacts the way everything
/// else in this application does: the hover and press factors are eased over
/// time instead of snapping, and the cell that is currently chosen holds an
/// accent that hover brightens rather than replaces.
///
/// `t` is the cell's slot in the calendar's entrance, so a grid arrives rather
/// than appearing all at once.
pub fn calendar_cell(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    text: &str,
    chosen: bool,
    today: bool,
    t: f32,
    scale: f32,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let a = interact(ui, &resp);
    // Fade the whole cell in on its slot, and lift it a little as it lands.
    let rect = rect.translate(egui::vec2(0.0, (1.0 - t) * 4.0));
    let alpha = |c: egui::Color32, k: f32| with_alpha(c, k * t);

    let painter = ui.painter();
    let rounding = egui::Rounding::ZERO;

    // Two different things are worth marking, and they are not the same thing:
    // the value this field holds, and the day it is today. They coincide often
    // enough that telling them apart only by weight would be no help at all, so
    // one is filled and the other is ringed — and when they coincide, the cell
    // is filled *and* carries the ring.
    if chosen {
        painter.rect_filled(rect, rounding, alpha(color::ACCENT, 0.26 + a.hover * 0.18));
        painter.rect_stroke(
            rect,
            rounding,
            egui::Stroke::new(1.4_f32, alpha(color::ACCENT, 0.95)),
        );
    } else {
        painter.rect_filled(
            rect,
            rounding,
            alpha(color::BG_WIDGET, 0.55 + a.hover * 0.45),
        );
        // Today is ringed rather than filled, and the ring holds whether it is
        // hovered or not — a marker that disappears under the pointer is not a
        // marker.
        let edge = if today {
            lerp_color(color::ACCENT, color::ACCENT_BRIGHT, a.hover)
        } else {
            lerp_color(color::BORDER, color::ACCENT, a.hover)
        };
        let weight: f32 = if today { 1.4 } else { 1.0 };
        let solidity: f32 = if today { 0.85 } else { 0.35 + a.hover * 0.65 };
        painter.rect_stroke(
            rect,
            rounding,
            egui::Stroke::new(weight, alpha(edge, solidity)),
        );
    }
    if today {
        // A bar under the label as well, so today reads at a glance in a grid
        // of forty-two boxes — and in a tone that shows on either ground, since
        // an amber bar on an amber fill is nothing.
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 6.0, rect.bottom() - 4.0),
            egui::vec2(rect.width() - 12.0, 2.0),
        );
        let ink = if chosen {
            color::ON_ACCENT
        } else {
            color::ACCENT
        };
        painter.rect_filled(bar, rounding, alpha(ink, 0.85));
    }

    // The label scales with the cell, or a zoom is boxes moving while the text
    // stands still.
    let mut font = egui::TextStyle::Button.resolve(ui.style());
    font.size *= scale;
    let ink = if chosen {
        color::ACCENT_BRIGHT
    } else if today {
        // Today is legible without being mistaken for the selection.
        color::TEXT
    } else {
        lerp_color(color::TEXT_MUTED, color::TEXT, a.hover.max(0.55))
    };
    let galley = painter.layout_no_wrap(text.to_owned(), font, ink);
    // Centred in the cell, both ways — which is the whole point of the cell
    // having a size of its own.
    let at = rect.center() - galley.size() * 0.5 + egui::vec2(0.0, a.press * 0.5);
    painter.galley(at, galley, alpha(ink, 1.0));
    resp
}
