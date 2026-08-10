//! The Modules page: what is installed, and what could be.

use crate::app::*;

/// The Modules page — a Zed-Extensions-style list: installed modules plus the
/// ones available in the GitHub org (installable in a click).
#[allow(clippy::too_many_arguments)]
pub(crate) fn modules_page(
    ui: &mut egui::Ui,
    modules: &[ModuleSpec],
    git_installed: &HashSet<String>,
    git_meta: &HashMap<String, (String, String)>,
    available_updates: &HashMap<String, String>,
    remote: &[RemoteModule],
    remote_loading: bool,
    remote_error: &Option<String>,
    installing: &Option<String>,
    installing_runtime: &Option<String>,
    filter: &mut ModuleFilter,
    search: &mut String,
    open: &mut Option<String>,
    remove: &mut Option<String>,
    add: &mut Option<String>,
    update: &mut Option<String>,
    reload: &mut bool,
    modules_revealed_at: &mut Option<f64>,
    shown_filter: &mut ModuleFilter,
    remote_arrivals: &HashMap<String, f64>,
    removing: &HashMap<String, f64>,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.heading(i18n::t("modules.title"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui::outline_button(ui, &i18n::t("modules.reload"), egui::Vec2::ZERO).clicked() {
                *reload = true;
            }
        });
    });
    ui.add_space(10.0);

    // One universal search across installed (local) and org (remote) modules.
    ui::text_field(
        ui,
        search,
        &i18n::t("modules.search_hint"),
        f32::INFINITY,
        false,
    );
    ui.add_space(8.0);

    // Installed / Available filter.
    ui.horizontal(|ui| {
        for (value, key) in [
            (ModuleFilter::All, "modules.filter.all"),
            (ModuleFilter::Installed, "modules.filter.installed"),
            (ModuleFilter::Available, "modules.filter.available"),
        ] {
            if ui::chip(ui, &i18n::t(key), *filter == value).clicked() {
                *filter = value;
            }
        }
        if remote_loading {
            ui.add_space(8.0);
            ui.spinner();
        }
    });

    // The chips above may have just flipped the filter — reset the reveal timer
    // *this* frame so the new list animates in from opacity 0 rather than flashing
    // at full opacity for one frame before restarting.
    let now = ui.input(|i| i.time);
    if *filter != *shown_filter {
        *modules_revealed_at = Some(now);
        *shown_filter = *filter;
    }
    let reveal_at = modules_revealed_at.unwrap_or(now);

    ui.add_space(6.0);
    ui.separator();

    let terms = parse_query(search);
    let installed_names: HashSet<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    // A tag or dependency clicked on any card; applied to the search box once the
    // list is done being drawn, since `search` is what the loop above filters on.
    let mut tag_click: Option<String> = None;
    // Which installed module provides each capability, so a dependency can be
    // shown as the module behind it rather than the raw capability string.
    let providers: HashMap<&str, &ModuleSpec> = modules
        .iter()
        .flat_map(|m| m.capabilities.iter().map(move |c| (c.as_str(), m)))
        .collect();

    // `auto_shrink` off is what makes the wheel work here, not just a layout
    // tweak: a shrunk scroll area is only as wide as its widest card, and the
    // wheel is ignored anywhere outside it — so scrolling did nothing over the
    // empty space beside the list. Filling the panel makes the whole page take
    // the wheel.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        let mut shown = 0;
        let animate = ui::animations_enabled();

        for m in modules.iter() {
            if *filter == ModuleFilter::Available || !module_matches(m, &terms) {
                continue;
            }
            let rt = match removing.get(m.name.as_str()) {
                Some(&s) if s < 0.0 => 1.0, // removal sent — stay faded out
                Some(&s) => (((now - s) / 0.34).clamp(0.0, 1.0)) as f32,
                None => 0.0,
            };
            let id = egui::Id::new(("modcard", m.name.as_str()));
            reveal_card(ui, id, shown, reveal_at, now, animate, rt, |ui| {
                module_card(
                    ui,
                    m,
                    git_installed.contains(&m.name),
                    git_meta.get(&m.name),
                    available_updates.get(&m.name).map(String::as_str),
                    installing,
                    installing_runtime,
                    open,
                    remove,
                    update,
                    &mut tag_click,
                    &providers,
                );
            });
            shown += 1;
        }

        // Available in the org (not already installed). These stream in from
        // GitHub in parallel, so a card has two possible entrances and takes
        // whichever is newer: it either lands *after* the page revealed, and
        // pops in alone the moment it arrives, or it was already listed and
        // joins the page's cascade at its position. Keying only off arrival —
        // as this did — meant a card fetched minutes ago had a long-past start
        // time and snapped in without animating whenever the tab was reopened.
        for r in remote {
            if *filter == ModuleFilter::Installed
                || installed_names.contains(r.name.as_str())
                || !remote_matches(r, &terms)
            {
                continue;
            }
            let id = egui::Id::new(("availcard", r.name.as_str()));
            let (start, k) = match remote_arrivals.get(&r.name) {
                Some(&a) if a > reveal_at => (a, 0),
                _ => (reveal_at, shown),
            };
            reveal_card(ui, id, k, start, now, animate, 0.0, |ui| {
                available_card(ui, r, installing, add, &mut tag_click);
            });
            shown += 1;
        }

        if let Some(err) = remote_error {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("{} {err}", i18n::t("modules.org_error")))
                    .small()
                    .color(ui::color::TEXT_MUTED),
            );
        }
        if shown == 0 && !remote_loading {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(i18n::t("modules.none_match")).color(ui::color::TEXT_MUTED),
                );
            });
        }
    });

    // Clicking a tag filters by it: drop it into the search box, which every
    // card already matches against.
    if let Some(t) = tag_click {
        *search = t;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn module_card(
    ui: &mut egui::Ui,
    m: &ModuleSpec,
    from_git: bool,
    git_meta: Option<&(String, String)>,
    latest: Option<&str>,
    installing: &Option<String>,
    installing_runtime: &Option<String>,
    open: &mut Option<String>,
    remove: &mut Option<String>,
    update: &mut Option<String>,
    tag_click: &mut Option<String>,
    providers: &HashMap<&str, &ModuleSpec>,
) {
    // This card is mid-update; another install/update is running somewhere.
    let this_busy = installing.as_deref() == Some(m.name.as_str());
    let any_busy = installing.is_some();
    // The scripted runtime this module needs is still downloading — every module
    // sharing that runtime (e.g. all Python modules) is not launchable yet, so
    // its Open button is disabled until the interpreter is bundled.
    let runtime_busy = Runtime::for_language(m.language)
        .is_some_and(|rt| installing_runtime.as_deref() == Some(rt.display()));
    egui::Frame::none()
        .fill(ui::color::BG_ELEVATED)
        .stroke(egui::Stroke::new(1.0_f32, ui::color::BORDER))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            // Stretch the box to the full available width.
            ui.set_min_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let right_w = 112.0;
                let spacing = ui.spacing().item_spacing.x;
                let left_w = (ui.available_width() - right_w - spacing).max(200.0);

                // Left column: name, badges, description, authors.
                ui.allocate_ui_with_layout(
                    egui::vec2(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        // Force the region to fill left_w so the right column is
                        // pushed to the box's end (allocate_ui otherwise collapses
                        // to content width).
                        ui.set_min_width(left_w);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(localized_name(ui.ctx(), m))
                                    .size(16.0)
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new(format!("v{}", m.version))
                                    .monospace()
                                    .color(ui::color::TEXT_MUTED),
                            );
                            for cap in &m.capabilities {
                                badge(ui, cap);
                            }
                        });
                        if let Some(desc) = localized_desc(ui, m) {
                            ui.add_space(6.0);
                            ui.label(desc);
                        }
                        if !m.tags.is_empty() {
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                for t in &m.tags {
                                    if tag_chip(ui, t).clicked() {
                                        *tag_click = Some(format!("tag:{t}"));
                                    }
                                }
                            });
                        }
                        // Host-privilege heads-up: some methods may need admin on
                        // this machine. Informational only — no listing, no prompt.
                        if m.permissions.may_require_admin {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(i18n::t("modules.may_require_admin"))
                                    .small()
                                    .color(egui::Color32::from_rgb(0xe6, 0x9a, 0x5c)),
                            );
                        }
                        if !m.authors.is_empty() {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} {}",
                                    i18n::t("modules.by"),
                                    m.authors.join(", ")
                                ))
                                .small()
                                .color(ui::color::TEXT_MUTED),
                            );
                        }
                        // Git revision this module was installed from — branch on
                        // top, commit below.
                        if let Some((branch, commit)) = git_meta {
                            let mut lines: Vec<String> = Vec::new();
                            if !branch.is_empty() {
                                lines.push(format!("{} - {branch}", i18n::t("about.branch")));
                            }
                            if !commit.is_empty() {
                                lines.push(format!("{} - {commit}", i18n::t("about.commit")));
                            }
                            if !lines.is_empty() {
                                ui.add_space(4.0);
                                for line in lines {
                                    ui.label(
                                        egui::RichText::new(line)
                                            .monospace()
                                            .small()
                                            .color(ui::color::TEXT_MUTED),
                                    );
                                }
                            }
                        }
                        // Hard dependencies (capabilities it needs) and optional
                        // integrations (extra features when a provider is loaded).
                        if !m.requires.is_empty() {
                            ui.add_space(6.0);
                            dep_row(
                                ui,
                                &i18n::t("modules.requires"),
                                ui::color::TEXT_MUTED,
                                m.requires.keys(),
                                providers,
                                tag_click,
                            );
                        }
                        if !m.optional.is_empty() {
                            ui.add_space(4.0);
                            dep_row(
                                ui,
                                &i18n::t("modules.optional"),
                                ui::color::ACCENT,
                                m.optional.keys(),
                                providers,
                                tag_click,
                            );
                        }
                    },
                );

                // Right column: Open / Remove / GitHub, stacked at the box end.
                ui.allocate_ui_with_layout(
                    egui::vec2(right_w, 0.0),
                    egui::Layout::top_down(egui::Align::Max),
                    |ui| {
                        // Size every action button to the widest label shown on
                        // this card, so the column is one uniform width. `+ 40`
                        // covers the button padding (the larger, primary, one) so
                        // both outline and primary buttons land at the same width.
                        let btn_font = egui::TextStyle::Button.resolve(ui.style());
                        // Resolve each label once so the width measurement and the
                        // buttons use exactly the same (localized) text.
                        let open_lbl = i18n::t("modules.open");
                        let remove_lbl = i18n::t("modules.remove");
                        let github_lbl = i18n::t("modules.github");
                        let update_lbl = (from_git && latest.is_some()).then(|| match latest {
                            Some(v) => format!("{} {v}", i18n::t("modules.update")),
                            None => i18n::t("modules.update"),
                        });

                        let mut labels: Vec<&str> = vec![open_lbl.as_str()];
                        if let Some(u) = &update_lbl {
                            labels.push(u);
                        }
                        labels.push(&remove_lbl);
                        if from_git && m.repo.is_some() {
                            labels.push(&github_lbl);
                        }
                        let max_text = labels
                            .iter()
                            .map(|l| {
                                ui.fonts(|f| {
                                    f.layout_no_wrap(
                                        l.to_uppercase(),
                                        btn_font.clone(),
                                        egui::Color32::WHITE,
                                    )
                                    .size()
                                    .x
                                })
                            })
                            .fold(0.0_f32, f32::max);
                        let bw = egui::vec2(max_text + 40.0, ui.spacing().interact_size.y);
                        if this_busy {
                            // Mid-update: spinner in place of the action buttons.
                            ui.allocate_ui_with_layout(
                                bw,
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new(i18n::t("modules.updating"))
                                            .small()
                                            .color(ui::color::TEXT_MUTED),
                                    );
                                },
                            );
                            return;
                        }
                        // Open is disabled while this module's runtime downloads.
                        let open_clicked = ui
                            .add_enabled_ui(!runtime_busy, |ui| {
                                ui::outline_button(ui, &open_lbl, bw)
                            })
                            .inner
                            .clicked();
                        if open_clicked {
                            *open = Some(m.name.clone());
                        }
                        if runtime_busy {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    egui::RichText::new(i18n::t("modules.preparing_runtime"))
                                        .small()
                                        .color(ui::color::TEXT_MUTED),
                                );
                            });
                        }
                        if let Some(update_lbl) = &update_lbl {
                            // Update only shows when a newer release exists.
                            let clicked = ui
                                .add_enabled_ui(!any_busy, |ui| {
                                    ui::primary_button(ui, update_lbl, bw)
                                })
                                .inner
                                .clicked();
                            if clicked {
                                *update = Some(m.name.clone());
                            }
                        }
                        // Red: this one deletes the module and everything under it.
                        if ui::danger_button(ui, &remove_lbl, bw).clicked() {
                            *remove = Some(m.name.clone());
                        }
                        // GitHub only for git-installed modules.
                        if from_git
                            && let Some(repo) = &m.repo
                            && ui::outline_button(ui, &github_lbl, bw).clicked()
                        {
                            ui.output_mut(|o| {
                                o.open_url = Some(egui::OpenUrl::new_tab(repo_url(repo)));
                            });
                        }
                    },
                );
            });
        });
}

/// An "available in the org, not installed" card, with an Install action.
pub(crate) fn available_card(
    ui: &mut egui::Ui,
    r: &RemoteModule,
    installing: &Option<String>,
    add: &mut Option<String>,
    tag_click: &mut Option<String>,
) {
    // While any install runs, every Install button is disabled; the one being
    // installed shows a spinner in place of the button.
    let this_installing = installing.as_deref() == Some(r.repo.as_str());
    let any_installing = installing.is_some();
    egui::Frame::none()
        .fill(ui::color::BG_ELEVATED)
        .stroke(egui::Stroke::new(1.0_f32, ui::color::BORDER))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let right_w = 112.0;
                let spacing = ui.spacing().item_spacing.x;
                let left_w = (ui.available_width() - right_w - spacing).max(200.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(left_w);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(r.title.as_deref().unwrap_or(&r.name))
                                    .size(16.0)
                                    .strong(),
                            );
                            if let Some(v) = &r.version {
                                ui.label(
                                    egui::RichText::new(format!("v{v}"))
                                        .monospace()
                                        .color(ui::color::TEXT_MUTED),
                                );
                            }
                            for cap in &r.capabilities {
                                badge(ui, cap);
                            }
                            badge(ui, "not installed");
                        });
                        if let Some(desc) = &r.description {
                            ui.add_space(6.0);
                            ui.label(desc);
                        }
                        if !r.tags.is_empty() {
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                for t in &r.tags {
                                    if tag_chip(ui, t).clicked() {
                                        *tag_click = Some(format!("tag:{t}"));
                                    }
                                }
                            });
                        }
                        // Git status of what a fresh install would fetch — same
                        // shape as installed modules (branch / commit).
                        if let Some(b) = &r.branch {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("branch - {b}"))
                                    .monospace()
                                    .small()
                                    .color(ui::color::TEXT_MUTED),
                            );
                        }
                        if let Some(c) = &r.commit {
                            ui.label(
                                egui::RichText::new(format!("commit - {c}"))
                                    .monospace()
                                    .small()
                                    .color(ui::color::TEXT_MUTED),
                            );
                        }
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(&r.repo)
                                .small()
                                .color(ui::color::TEXT_MUTED),
                        );
                    },
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(right_w, 0.0),
                    egui::Layout::top_down(egui::Align::Max),
                    |ui| {
                        // Uniform button width = the widest label (Install / GitHub).
                        let btn_font = egui::TextStyle::Button.resolve(ui.style());
                        let install_lbl = i18n::t("modules.install");
                        let github_lbl = i18n::t("modules.github");
                        let max_text = [&install_lbl, &github_lbl]
                            .iter()
                            .map(|l| {
                                ui.fonts(|f| {
                                    f.layout_no_wrap(
                                        l.to_uppercase(),
                                        btn_font.clone(),
                                        egui::Color32::WHITE,
                                    )
                                    .size()
                                    .x
                                })
                            })
                            .fold(0.0_f32, f32::max);
                        let bw = egui::vec2(max_text + 40.0, ui.spacing().interact_size.y);
                        if this_installing {
                            // This module is downloading — spinner + label.
                            ui.allocate_ui_with_layout(
                                bw,
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new(i18n::t("modules.installing"))
                                            .small()
                                            .color(ui::color::TEXT_MUTED),
                                    );
                                },
                            );
                        } else {
                            // Disable while another install is in flight.
                            let clicked = ui
                                .add_enabled_ui(!any_installing, |ui| {
                                    ui::primary_button(ui, &install_lbl, bw)
                                })
                                .inner
                                .clicked();
                            if clicked {
                                *add = Some(r.repo.clone());
                            }
                        }
                        if ui::outline_button(ui, &github_lbl, bw).clicked() {
                            let url = r.url.clone();
                            ui.output_mut(|o| o.open_url = Some(egui::OpenUrl::new_tab(url)));
                        }
                    },
                );
            });
        });
}
