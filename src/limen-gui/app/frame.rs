//! The frame loop: everything drawn, and every intent collected, once per
//! frame.

use super::*;

impl eframe::App for LimenApp {
    /// Clear transparent during the startup splash so only the floating icons
    /// show over the desktop; opaque warm-black once the app takes over.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.splash_done {
            let c = ui::color::BG;
            [
                c.r() as f32 / 255.0,
                c.g() as f32 / 255.0,
                c.b() as f32 / 255.0,
                1.0,
            ]
        } else {
            [0.0, 0.0, 0.0, 0.0]
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now_t = ctx.input(|i| i.time);
        self.drain_events(now_t);
        self.drain_file_picks();
        // Keep the screen→UI mapping current, so a file drag (during which the
        // window system stops reporting the pointer) can still be located.
        ui::cursor::calibrate(ctx);
        // A file drag produces exactly one event — `HoveredFile` — and then
        // silence: no cursor motion is reported for its duration. egui would go
        // idle on the very next frame, freezing the cursor sample taken as the
        // drag entered, so the drop zone would follow nothing. Keep asking for
        // frames while one is in flight, and only then.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            ctx.request_repaint();
        }

        // Apply the global UI scale (set_zoom_factor no-ops if unchanged).
        ctx.set_zoom_factor(self.ui_scale / 100.0);

        // The window is created hidden; on the first frame centre it on the
        // monitor and reveal it, so the splash appears cleanly mid-screen instead
        // of flashing a dark, unfocused window at the default position.
        if !self.window_shown {
            let size = egui::vec2(980.0, 640.0);
            if let Some(mon) = ctx.input(|i| i.viewport().monitor_size) {
                let pos = egui::pos2(
                    ((mon.x - size.x) * 0.5).max(0.0),
                    ((mon.y - size.y) * 0.5).max(0.0),
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.window_shown = true;
        }

        // Warm the font atlas over the first few frames while the window is still
        // a blank transparent canvas — *before* the splash starts. The glyph
        // rasterization hitches, but nothing is drawn yet (the window is
        // transparent, showing the desktop) and the splash clock hasn't started,
        // so it's invisible. Warming several frames lets the pixels-per-point /
        // zoom settle first, and returning early keeps the hitch off the splash
        // animation, which then plays smoothly against a settled atlas.
        const WARM_FRAMES: u32 = 6;
        if self.fonts_warm_frames < WARM_FRAMES {
            self.fonts_warm_frames += 1;
            prewarm_fonts(ctx);
            ctx.request_repaint();
            return;
        }

        // Startup splash in the centred window: the marks fade transparent→opaque,
        // hold opaque ~1s, then a 2s exit animation, after which the window
        // maximizes and the app takes over. Worker events keep draining above.
        // Skipped when animations are off.
        if !self.splash_done {
            let start = *self.splash_start.get_or_insert(now_t);
            let elapsed = (now_t - start) as f32;
            // With animations off, keep the transparent lead (so the clumsy opaque
            // startup frames stay hidden), then show the marks statically — no
            // fade/scanline motion.
            let dur = if self.animations {
                SPLASH_SECS
            } else {
                SPLASH_LEAD + SPLASH_HOLD
            };
            if elapsed < dur {
                ctx.request_repaint();
                splash_screen(ctx, elapsed, self.animations);
                return;
            }
            self.splash_done = true;
            // Grow into the app: fill the screen once the intro finishes.
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }

        // Reload is requested from the Modules page (set during the central panel).
        let mut reload = false;

        // Intents collected while rendering, applied after.
        let mut open_tab: Option<Tab> = None;
        // The license is a pop-up, not a tab: it is something you glance at and
        // dismiss, and as a tab it stayed open behind you every time.
        let mut open_license = false;
        let mut open_changes = false;
        let mut switch_to: Option<usize> = None;
        let mut close_idx: Option<usize> = None;
        let mut scale_changed = false;
        let mut anim_changed = false;
        let mut alerts_changed = false;
        let mut lang_changed = false;
        let mut dev_applied = false;

        // Custom-decoration resize grips along the window edges/corners.
        ui::window_resize_grips(ctx);

        // Title bar: brand + quick-open buttons + status.
        egui::TopBottomPanel::top("titlebar")
            .frame(
                egui::Frame::none()
                    .fill(ui::color::BG_ELEVATED)
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        ui::with_alpha(ui::color::ACCENT, 0.18),
                    ))
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
            )
            .show(ctx, |ui| {
                // Whole-bar drag to move the window + double-click to maximize.
                // Done before the row below so its buttons sit on top and keep
                // their own clicks. Constrain the drag zone to the row height —
                // `max_rect` here spans the whole window until content is measured,
                // so an unconstrained rect would hijack clicks on the pages below.
                let full = ui.max_rect();
                let bar_rect =
                    egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, full.min.y + 28.0));
                let bar = ui.interact(
                    bar_rect,
                    egui::Id::new("titlebar_drag"),
                    egui::Sense::click_and_drag(),
                );
                if bar.double_clicked() {
                    let max = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                if bar.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.horizontal(|ui| {
                    // App icon (the ◈ brand mark) in place of the wordmark.
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                    draw_brand(ui.painter(), rect, 1.0, false);
                    ui.add_space(12.0);
                    let active = self.active_tab();
                    if ui::chip(ui, &i18n::t("nav.about"), active == Some(Tab::About)).clicked() {
                        open_tab = Some(Tab::About);
                    }
                    if ui::chip(ui, &i18n::t("nav.modules"), active == Some(Tab::Modules)).clicked()
                    {
                        open_tab = Some(Tab::Modules);
                    }
                    // "Update available" pill, next to Modules.
                    if self.update.is_some() {
                        ui.add_space(6.0);
                        if ui::pill(ui, &i18n::t("nav.update_available"), ui::color::ORANGE)
                            .on_hover_text(i18n::t("nav.update_available_hint"))
                            .clicked()
                        {
                            open_tab = Some(Tab::Update);
                        }
                    }
                    // Portable-interpreter install indicator (spinner + label),
                    // shown while a runtime like Python is being bundled.
                    if let Some(rt) = &self.installing_runtime {
                        ui.add_space(8.0);
                        ui.spinner();
                        ui.label(
                            egui::RichText::new(format!("{} {rt}…", i18n::t("nav.installing")))
                                .small()
                                .color(ui::color::TEXT_MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Window controls (rightmost). GNOME's default title bar
                        // shows only a close button, so on Linux that's all we
                        // draw (double-click still maximizes); Windows/macOS get
                        // the full close · maximize · minimize set.
                        if ui::window_button(ui, ui::WinBtn::Close)
                            .on_hover_text(i18n::t("win.close"))
                            .clicked()
                        {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if !cfg!(target_os = "linux") {
                            let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                            let (mbtn, tip) = if maximized {
                                (ui::WinBtn::Restore, i18n::t("win.restore"))
                            } else {
                                (ui::WinBtn::Maximize, i18n::t("win.maximize"))
                            };
                            if ui::window_button(ui, mbtn).on_hover_text(tip).clicked() {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(
                                    !maximized,
                                ));
                            }
                            if ui::window_button(ui, ui::WinBtn::Minimize)
                                .on_hover_text(i18n::t("win.minimize"))
                                .clicked()
                            {
                                ui.ctx()
                                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        }
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);
                        if ui::chip(ui, "🛠", active == Some(Tab::Developer))
                            .on_hover_text(i18n::t("nav.developer"))
                            .clicked()
                        {
                            open_tab = Some(Tab::Developer);
                        }
                        if ui::chip(ui, "⚙", active == Some(Tab::Settings))
                            .on_hover_text(i18n::t("nav.settings"))
                            .clicked()
                        {
                            open_tab = Some(Tab::Settings);
                        }
                    });
                });
            });

        // Tab strip: open tabs, each with a close button. A tab *is* the
        // module's session — closing it ends the session.
        egui::TopBottomPanel::top("tabstrip")
            .frame(
                egui::Frame::none()
                    .fill(ui::color::BG)
                    .inner_margin(egui::Margin::symmetric(8.0, 4.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // A single-row, horizontally-scrollable tab strip. When the
                    // tabs overflow, nav arrows (« ‹ › ») appear and the mouse
                    // wheel scrolls it.
                    let arrow = |ui: &mut egui::Ui, glyph: &str, enabled: bool| -> bool {
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(20.0, 26.0), egui::Sense::click());
                        let col = if !enabled {
                            ui::with_alpha(ui::color::TEXT_MUTED, 0.3)
                        } else if resp.hovered() {
                            ui::color::ACCENT
                        } else {
                            ui::color::TEXT_MUTED
                        };
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            glyph,
                            egui::FontId::proportional(15.0),
                            col,
                        );
                        enabled && resp.clicked()
                    };
                    const STEP: f32 = 220.0;
                    let max_off = (self.tab_content_w - self.tab_view_w).max(0.0);
                    let overflow = max_off > 1.0;
                    let can_left = self.tab_scroll > 0.5;
                    let can_right = self.tab_scroll + 0.5 < max_off;
                    if overflow {
                        if arrow(ui, "«", can_left) {
                            self.tab_scroll_to = Some(0.0);
                        }
                        if arrow(ui, "‹", can_left) {
                            self.tab_scroll_to = Some((self.tab_scroll - STEP).max(0.0));
                        }
                    }
                    let right_reserve = if overflow { 40.0 } else { 0.0 };
                    let sa_width = (ui.available_width() - right_reserve).max(80.0);
                    let mut area = egui::ScrollArea::horizontal()
                        .id_source("tabstrip_scroll")
                        .auto_shrink([false, false])
                        .max_width(sa_width)
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                        );
                    if let Some(x) = self.tab_scroll_to.take() {
                        area = area.scroll_offset(egui::vec2(x, 0.0));
                    }
                    let out = area.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let font_id = egui::TextStyle::Button.resolve(ui.style());
                            for (i, tab) in self.tabs.iter().enumerate() {
                                let selected = i == self.active;
                                let text = match tab {
                                    Tab::Detail { id } => self
                                        .detail_tabs
                                        .get(id)
                                        .map(|d| d.title.clone())
                                        .filter(|t| !t.is_empty())
                                        .unwrap_or_else(|| i18n::t("tab.details")),
                                    // Show the module's localized display title, not its
                                    // identifier.
                                    Tab::Module(name) => self
                                        .modules
                                        .iter()
                                        .find(|m| &m.name == name)
                                        .map(|m| localized_name(ui.ctx(), m))
                                        .unwrap_or_else(|| name.clone()),
                                    _ => tab.title(),
                                };

                                // Zed-style tab: stable width (the close slot is always
                                // reserved), the × only shows on hover or when active.
                                let pad = 10.0;
                                let close_w = 16.0;
                                let gap = 6.0;
                                let galley = ui.painter().layout_no_wrap(
                                    text,
                                    font_id.clone(),
                                    ui::color::TEXT,
                                );
                                let w = pad + galley.size().x + gap + close_w + pad;
                                let (rect, resp) = ui
                                    .allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::click());
                                // Use the pointer position, not `resp.hovered()`: the close
                                // button below is drawn on top and would otherwise steal the
                                // hover, making the tab flicker as the × shows/hides.
                                let hovered = ui.rect_contains_pointer(rect);

                                // Smoothly fade the hover fill in, and grow the active
                                // underline out from the tab's centre toward its edges.
                                let hover_t = ui::anim_bool(
                                    ui,
                                    resp.id.with("hover"),
                                    hovered && !selected,
                                    0.14,
                                );
                                let active_t =
                                    ui::anim_bool(ui, resp.id.with("active"), selected, 0.05);

                                // Active tab adopts the panel colour; hover fades in.
                                let fill = if selected {
                                    ui::color::BG
                                } else {
                                    let e = ui::color::BG_ELEVATED;
                                    egui::Color32::from_rgba_unmultiplied(
                                        e.r(),
                                        e.g(),
                                        e.b(),
                                        (255.0 * hover_t) as u8,
                                    )
                                };
                                ui.painter().rect_filled(
                                    rect,
                                    egui::Rounding {
                                        nw: 5.0,
                                        ne: 5.0,
                                        sw: 0.0,
                                        se: 0.0,
                                    },
                                    fill,
                                );
                                if active_t > 0.0 {
                                    let half = rect.width() / 2.0 * active_t;
                                    let cx = rect.center().x;
                                    ui.painter().hline(
                                        (cx - half)..=(cx + half),
                                        rect.bottom() - 1.0,
                                        egui::Stroke::new(2.0_f32, ui::color::ACCENT),
                                    );
                                }

                                let text_t = if selected { 1.0 } else { hover_t };
                                let tcol =
                                    ui::lerp_color(ui::color::TEXT_MUTED, ui::color::TEXT, text_t);
                                let tpos = egui::pos2(
                                    rect.left() + pad,
                                    rect.center().y - galley.size().y / 2.0,
                                );
                                ui.painter().galley(tpos, galley, tcol);

                                // Close affordance — only when active or hovered.
                                let mut close_clicked = false;
                                if selected || hovered {
                                    let cc = egui::pos2(
                                        rect.right() - pad - close_w / 2.0,
                                        rect.center().y,
                                    );
                                    let crect = egui::Rect::from_center_size(
                                        cc,
                                        egui::vec2(close_w, close_w),
                                    );
                                    let cresp = ui.interact(
                                        crect,
                                        resp.id.with("close"),
                                        egui::Sense::click(),
                                    );
                                    if cresp.hovered() {
                                        ui.painter().rect_filled(
                                            crect,
                                            egui::Rounding::same(3.0),
                                            ui::color::BG_HOVER,
                                        );
                                    }
                                    ui.painter().text(
                                        cc,
                                        egui::Align2::CENTER_CENTER,
                                        "×",
                                        egui::FontId::proportional(15.0),
                                        if cresp.hovered() {
                                            ui::color::TEXT
                                        } else {
                                            ui::color::TEXT_MUTED
                                        },
                                    );
                                    if cresp.clicked() {
                                        close_idx = Some(i);
                                        close_clicked = true;
                                    }
                                }
                                if resp.clicked() && !close_clicked {
                                    switch_to = Some(i);
                                }
                            }
                        })
                    });
                    self.tab_scroll = out.state.offset.x;
                    self.tab_content_w = out.content_size.x;
                    self.tab_view_w = out.inner_rect.width();
                    if overflow {
                        if arrow(ui, "›", can_right) {
                            self.tab_scroll_to = Some((self.tab_scroll + STEP).min(max_off));
                        }
                        if arrow(ui, "»", can_right) {
                            self.tab_scroll_to = Some(max_off);
                        }
                    }
                });
            });

        // Central content for the active tab (split-borrow to mutate inputs etc).
        let mut action: Option<ui::Invoke> = None;
        let mut open_module: Option<String> = None;
        let mut remove_module: Option<String> = None;
        // Set by the one button an inactive module has — on its tab, or on its
        // card in the manager, which names the module.
        let mut show_panic = false;
        let mut check_panic: Option<String> = None;
        let inactive_modules = self.inactive_modules();
        let mut add_module: Option<String> = None;
        let mut update_module: Option<String> = None;
        let mut do_update = false;
        let active_tab = self.active_tab();
        let update_info = self.update.clone();
        let updating = self.updating;
        // Arm the reveal on first show of the Modules tab; the filter-change replay
        // is handled inside `modules_page` so it lands on the same frame as the
        // click (avoiding a one-frame flash).
        if active_tab == Some(Tab::Modules) {
            self.modules_revealed_at.get_or_insert(now_t);
        } else {
            self.modules_revealed_at = None;
        }
        if active_tab == Some(Tab::About) {
            self.about_revealed_at.get_or_insert(now_t);
        } else {
            self.about_revealed_at = None;
        }
        if active_tab == Some(Tab::Settings) {
            self.settings_revealed_at.get_or_insert(now_t);
        } else {
            self.settings_revealed_at = None;
        }
        if active_tab == Some(Tab::Developer) {
            self.developer_revealed_at.get_or_insert(now_t);
        } else {
            self.developer_revealed_at = None;
        }
        let module_reveal = match &active_tab {
            Some(Tab::Module(n)) => {
                if self.module_reveal.as_ref().map(|(m, _)| m.as_str()) != Some(n.as_str()) {
                    self.module_reveal = Some((n.clone(), now_t));
                }
                self.module_reveal.as_ref().map_or(now_t, |&(_, t)| t)
            }
            _ => {
                self.module_reveal = None;
                now_t
            }
        };
        let about_reveal = self.about_revealed_at.unwrap_or(now_t);
        let settings_reveal = self.settings_revealed_at.unwrap_or(now_t);
        {
            let LimenApp {
                modules,
                git_installed,
                git_meta,
                available_updates,
                view,
                view_error,
                inactive,
                inputs,
                output,
                busy_action,
                fatal,
                search,
                filter,
                remote,
                remote_error,
                remote_loading,
                installing,
                installing_runtime,
                dev_tab,
                logs,
                log_autoscroll,
                ui_scale,
                animations,
                alerts,
                language,
                dev_mode_on,
                dev_limen_path,
                dev_modules_path,
                removing,
                modules_revealed_at,
                shown_filter,
                remote_arrivals,
                developer_revealed_at,
                shown_dev_tab,
                detail_tabs,
                ..
            } = self;
            let content_margin = 16.0_f32;
            let content_frame = egui::Frame::none()
                .fill(ui::color::BG)
                .inner_margin(egui::Margin::same(content_margin));
            let content = egui::CentralPanel::default()
                .frame(content_frame)
                .show(ctx, |ui| {
                    // Framed HUD corner brackets, evenly inset from the window edge;
                    // the panel margin keeps page content padded inside them.
                    let outer = ui.max_rect().expand(content_margin);
                    ui::corner_brackets(ui.painter(), outer, 18.0, 8.0, 0.55);
                    if let Some(err) = fatal {
                        ui.colored_label(egui::Color32::LIGHT_RED, i18n::t("app.engine_failed"));
                        ui.add_space(4.0);
                        ui.monospace(err.as_str());
                        return;
                    }
                    match active_tab {
                        None => {
                            ui.add_space(24.0);
                            ui.vertical_centered(|ui| {
                                ui.label(egui::RichText::new(i18n::t("app.no_tabs")).weak());
                            });
                        }
                        Some(Tab::About) => {
                            if about_view(ui, about_reveal) {
                                open_license = true;
                            }
                        }
                        Some(Tab::Modules) => modules_page(
                            ui,
                            modules,
                            &inactive_modules,
                            git_installed,
                            git_meta,
                            available_updates,
                            remote,
                            *remote_loading,
                            remote_error,
                            installing,
                            installing_runtime,
                            filter,
                            search,
                            &mut open_module,
                            &mut check_panic,
                            &mut remove_module,
                            &mut add_module,
                            &mut update_module,
                            &mut reload,
                            modules_revealed_at,
                            shown_filter,
                            remote_arrivals,
                            removing,
                        ),
                        Some(Tab::Module(name)) => module_view(
                            ui,
                            &name,
                            module_reveal,
                            view,
                            view_error,
                            inactive,
                            inputs,
                            output,
                            busy_action.as_ref(),
                            &mut action,
                            &mut show_panic,
                        ),
                        Some(Tab::Detail { id }) => detail_view(ui, id, detail_tabs, &mut action),
                        Some(Tab::Settings) => settings_view(
                            ui,
                            ui_scale,
                            &mut scale_changed,
                            animations,
                            &mut anim_changed,
                            alerts,
                            &mut alerts_changed,
                            language,
                            &mut lang_changed,
                            settings_reveal,
                        ),
                        Some(Tab::Developer) => developer_view(
                            ui,
                            dev_tab,
                            inputs,
                            logs,
                            log_autoscroll,
                            dev_mode_on,
                            dev_limen_path,
                            dev_modules_path,
                            &mut dev_applied,
                            developer_revealed_at,
                            shown_dev_tab,
                        ),
                        Some(Tab::Update) => {
                            update_view(
                                ui,
                                update_info.as_ref(),
                                updating,
                                &mut do_update,
                                &mut open_changes,
                            )
                        }
                    }
                });
            // Pop-ups belong to the tab that raised them, so this is what they
            // dim and block — the title bar and the tab strip stay live.
            self.content_rect = Some(content.response.rect);
        }

        if do_update && let Some(info) = self.update.clone() {
            self.updating = true;
            self.worker.send(Command::ApplyUpdate(info));
        }

        // Apply tab intents.
        if let Some(i) = switch_to {
            // Every tab change goes through one door, which puts the old tab's
            // state away and gives the new one its own back.
            self.activate(i);
            // A module tab that has never been shown still needs its first view.
            if let Some(Tab::Module(name)) = self.active_tab() {
                self.select_module(name);
            }
        }
        if let Some(i) = close_idx {
            self.close_tab(i);
        }
        if open_license {
            self.license_open = true;
        }
        if open_changes {
            self.changes_open = true;
        }
        if let Some(tab) = open_tab {
            match tab {
                Tab::Module(name) => self.select_module(name),
                other => self.open_tab(other),
            }
        }
        if scale_changed {
            self.save_ui_scale();
        }
        if anim_changed {
            ui::set_animations(self.animations);
            self.save_animations();
        }
        if alerts_changed {
            ui::toast::set_enabled(self.alerts);
            self.save_alerts();
        }
        if lang_changed {
            self.save_language();
            // Re-play the current page's staggered entrance so it animates into
            // the new language (clearing the timers re-arms them on the next
            // frame; only the active page is non-None, so only it re-reveals).
            self.about_revealed_at = None;
            self.settings_revealed_at = None;
            self.developer_revealed_at = None;
            self.modules_revealed_at = None;
            // Installed cards re-resolve their description in-place (localized_desc
            // reads the module's locales/ folder, cached per language) — no engine
            // reload. The org list, though, resolved its descriptions over the
            // network at fetch time, so re-fetch it (async; cards stream back in).
            self.remote_fetched = false;
            self.remote.clear();
            self.remote_arrivals.clear();
        }
        if dev_applied {
            // Re-run both update checks now so the change is reflected without a
            // restart: the app check is otherwise startup-only, and Refresh
            // re-runs the module check against the (possibly new) source.
            self.worker.send(Command::CheckUpdate);
            self.worker.send(Command::Refresh);
        }

        if let Some(name) = open_module {
            self.select_module(name);
        }
        // Ask before removing. The confirmed name is applied further down, so
        // both paths run exactly the same removal.
        if let Some(name) = remove_module {
            self.pending_remove = Some(name);
        }
        if show_panic {
            self.panic_shown = self.inactive.clone();
            self.panic_open = true;
        }
        // From the manager, where the module is named rather than open.
        if let Some(name) = check_panic {
            self.panic_shown = inactive_modules.get(&name).cloned();
            self.panic_open = true;
        }
        // Why the module is out of service. A pop-up rather than the screen: a
        // panic payload is a wall of Rust and a start failure is a stack of
        // loader context, and the tab behind it should still say plainly, in
        // one line, that there is nothing here to use.
        if self.panic_open || self.panic_alive {
            let reason = self.panic_shown.clone();
            let out = panic_dialog(ctx, self.panic_open, reason.as_ref());
            if out.close || out.back {
                self.panic_open = false;
            }
            self.panic_alive = !out.closed;
        }
        // The changelog pop-up.
        if self.changes_open || self.changes_alive {
            let notes = self
                .update
                .as_ref()
                .map(|u| (u.latest.clone(), u.notes.clone()));
            let out = changes_dialog(ctx, self.changes_open, notes.as_ref());
            if out.close || out.back {
                self.changes_open = false;
            }
            self.changes_alive = !out.closed;
        }

        // The license pop-up. Drawn over everything, like the other dialogs.
        if self.license_open || self.license_alive {
            let out = license_dialog(ctx, self.license_open);
            if out.close || out.back {
                self.license_open = false;
            }
            self.license_alive = !out.closed;
        }

        if let Some((level, text)) = self.pending_notice.take() {
            ui::toast::notify(ctx, level, text);
        }
        // Notices last, and above everything: one is usually about whatever the
        // pop-up in front of it is doing.
        ui::toast::draw(ctx);

        if let Some(name) = self.confirmed_removal(ctx) {
            self.status = format!("removing {name}…");
            if self.animations {
                // Play the exit animation first; the actual removal fires when it
                // finishes (see the removal processing below).
                self.removing.insert(name, now_t);
            } else {
                self.busy = true;
                self.worker.send(Command::RemoveModule(name));
            }
        }
        // Drive in-flight removals: once a card's exit animation has run for its
        // duration, send the real removal (once); keep the entry so the card stays
        // invisible until the reload drops it, then clean it up.
        {
            let present: HashSet<&String> = self.modules.iter().map(|m| &m.name).collect();
            self.removing.retain(|name, _| present.contains(name));
            let mut fire: Vec<String> = Vec::new();
            for (name, start) in self.removing.iter_mut() {
                if *start >= 0.0 && now_t - *start >= 0.34 {
                    fire.push(name.clone());
                    *start = -1.0; // mark sent; card stays faded out
                }
            }
            for name in fire {
                self.busy = true;
                self.worker.send(Command::RemoveModule(name));
            }
            if !self.removing.is_empty() {
                ctx.request_repaint();
            }
        }
        if let Some(reference) = add_module {
            self.busy = true;
            self.installing = Some(reference.clone());
            self.status = format!("installing {reference}…");
            self.worker.send(Command::AddModule(reference));
        }
        if let Some(name) = update_module {
            // Close its tab first — its loaded UI is about to be replaced.
            if let Some(i) = self
                .tabs
                .iter()
                .position(|t| *t == Tab::Module(name.clone()))
            {
                self.close_tab(i);
            }
            self.busy = true;
            self.installing = Some(name.clone());
            self.status = format!("updating {name}…");
            self.worker.send(Command::UpdateModule(name));
        }
        if reload {
            // Re-scan the module directories from disk: pick up newly-added
            // modules and drop any whose folder is gone (not just re-list what's
            // already loaded).
            self.worker.send(Command::Reload);
            self.remote_fetched = false;
            // A reload restarts the modules, so a stored screen describes a
            // connection that no longer exists — and a stored pop-up would sit
            // over a module that has forgotten it was ever open.
            self.module_pages.clear();
        }
        if let Some(a) = action {
            // Elevated methods prompt for consent (once) before running.
            if self.action_needs_consent(&a.action) {
                self.pending_action = Some(a);
            } else {
                self.dispatch(a);
            }
        }
        // A Browse button was pressed somewhere in the view just drawn.
        self.serve_browse_request(ctx);

        // Consent dialog for a pending elevated action.
        //
        // Kept alive for a moment after it is answered so it can animate away —
        // the pending action is cleared immediately, so the *last* one is what
        // the closing frames draw.
        {
            let open = self.pending_action.is_some();
            if open {
                self.consent_showing = self.pending_action.clone();
            }
            let mut decision: Option<bool> = None;
            if let Some(pending) = self.consent_showing.clone() {
                let module = self.module_of(&pending.action.capability).cloned();
                let gone = ui::consent_dialog(ctx, open, self.content_rect, |ui| {
                    let fallback = i18n::t("perm.this_module");
                    let name = module
                        .as_ref()
                        .map(|m| m.name.as_str())
                        .unwrap_or(&fallback);
                    ui.label(
                        egui::RichText::new(
                            i18n::t("perm.wants_to_run")
                                .replace("{name}", name)
                                .replace("{method}", &pending.action.method),
                        )
                        .size(15.0),
                    );
                    if let Some(m) = &module {
                        let perms = m.permissions.summary();
                        if !perms.is_empty() {
                            ui.add_space(8.0);
                            for p in perms {
                                let admin = p.contains("administrator");
                                let col = if admin {
                                    egui::Color32::from_rgb(0xe6, 0x9a, 0x5c)
                                } else {
                                    ui::color::TEXT_MUTED
                                };
                                ui.label(egui::RichText::new(format!("•  {p}")).color(col));
                            }
                        }
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui::primary_button(ui, &i18n::t("perm.grant_run"), egui::Vec2::ZERO)
                            .clicked()
                        {
                            decision = Some(true);
                        }
                        if ui::outline_button(ui, &i18n::t("perm.deny"), egui::Vec2::ZERO).clicked()
                        {
                            decision = Some(false);
                        }
                        ui.label(
                            egui::RichText::new(i18n::t("perm.remembered"))
                                .small()
                                .color(ui::color::TEXT_MUTED),
                        );
                    });
                });
                if gone {
                    self.consent_showing = None;
                }
                // Only a live dialog can be answered; a closing one is an
                // animation and its buttons are on their way out.
                if open {
                    match decision {
                        Some(true) => {
                            if let Some(m) = &module {
                                self.grant_trust(&m.name);
                            }
                            self.pending_action = None;
                            self.dispatch(pending); // now allowed
                        }
                        Some(false) => {
                            self.pending_action = None;
                            self.status = i18n::t("perm.denied");
                        }
                        None => {}
                    }
                }
            }
        }

        // Prefetch the org's module list once at startup (not lazily on first
        // Modules open) — the fetch is async on the worker, so it runs in the
        // background during/after the splash and the manager is already populated
        // by the time the user opens it. Also re-fires after a reload or a
        // language change (both reset `remote_fetched`).
        if !self.remote_fetched {
            self.remote_fetched = true;
            self.remote_loading = true;
            self.remote.clear();
            self.remote_arrivals.clear();
            self.remote_error = None;
            self.worker.send(Command::ListRemote);
        }

        // A module action's own confirmation, drawn with the host's dialog so it
        // matches the one the module manager uses to remove a module.
        {
            let open = self.pending_confirm.is_some();
            if open {
                self.confirm_showing = self.pending_confirm.clone();
            }
            if let Some(inv) = self.confirm_showing.clone() {
                let c = inv.confirm.clone().unwrap_or_default();
                let subject = (!c.subject.is_empty()).then_some(c.subject.as_str());
                let yes = if c.confirm_label.is_empty() {
                    i18n::t("confirm.remove_yes")
                } else {
                    c.confirm_label.clone()
                };
                let no = if c.cancel_label.is_empty() {
                    i18n::t("confirm.cancel")
                } else {
                    c.cancel_label.clone()
                };
                match ui::confirm_dialog(
                    ctx,
                    "module",
                    open,
                    &c.title,
                    subject,
                    &yes,
                    &no,
                    self.content_rect,
                ) {
                    Some(true) => {
                        self.pending_confirm = None;
                        // Run it now that it has been answered; the question is
                        // dropped so it is not asked a second time.
                        let mut go = inv;
                        go.confirm = None;
                        self.dispatch(go);
                    }
                    Some(false) => self.pending_confirm = None,
                    None => {
                        if !open {
                            self.confirm_showing = None;
                        }
                    }
                }
            }
        }

        // Module pop-ups, over everything the panels drew. Driven every frame
        // rather than only while one is open, so a closing pop-up has something
        // to animate away with.
        {
            let busy_action = self.busy_action.clone();
            let depth = self.modal_stack.len();
            let top = self
                .modal_stack
                .last()
                .or(self.modal_closing.as_ref())
                .cloned();
            let out = ui::modal_layer(
                ctx,
                top.as_ref(),
                !self.modal_stack.is_empty(),
                depth,
                self.content_rect,
                &mut self.inputs,
                busy_action.as_ref(),
            );
            if out.closed {
                self.modal_closing = None;
            }
            if out.close_all {
                self.close_modals();
            } else if out.dismissed {
                self.dismiss_modal();
            } else if let Some(inv) = out.invoke {
                // A `dismiss` button is answered here — it has nothing to ask the
                // module, so a round trip would only add a flicker.
                if inv.dismiss {
                    self.dismiss_modal();
                } else {
                    self.dispatch(inv);
                }
            }
        }

        // Closing asks first. Deliberately the *whole* window rather than a
        // tab's content: quitting is not one tab's business, and everything
        // behind the question should be inert until it is answered.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.quit_asking = true;
        }
        if self.quit_asking {
            match ui::confirm_dialog(
                ctx,
                "quit",
                true,
                &i18n::t("confirm.quit_title"),
                None,
                &i18n::t("confirm.quit_yes"),
                &i18n::t("confirm.cancel"),
                // No bounds: this one covers the app, not a tab.
                None,
            ) {
                Some(true) => {
                    self.quit_asking = false;
                    self.quit_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(false) => self.quit_asking = false,
                None => {}
            }
        } else {
            // Keep driving it so it animates away after an answer.
            let _ = ui::confirm_dialog(
                ctx,
                "quit",
                false,
                &i18n::t("confirm.quit_title"),
                None,
                &i18n::t("confirm.quit_yes"),
                &i18n::t("confirm.cancel"),
                None,
            );
        }

        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

// --------------------------------------------------------------------------- //
