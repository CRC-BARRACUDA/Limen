//! The user's own OpenPGP key, as the GUI holds it: which `gpg` is used, the
//! keys it has, which of them is the user's — and the signer modules reach
//! through `host.gpg_sign_file`.
//!
//! Everything that runs `gpg` runs on a thread: listing keys can wait on a
//! smart card, and signing waits on the user's passphrase.

use std::path::Path;
use std::sync::{mpsc, Arc};

use eframe::egui;
use limen_core::gpg::{Gpg, GpgError, UserKey};
use limen_host::{FileSigner, SignedFile, SignerIdentity};
use limen_ui as ui;

use crate::i18n;

/// What signs for modules: the configured GnuPG. As whom is said at each
/// signature, or else it is the key and user id chosen on the page — read from
/// the settings each time, so a change there holds at once.
struct UserSigner;

/// The key and user id chosen on the OpenPGP page, as the settings have them.
fn chosen() -> (Option<String>, Option<String>) {
    let cfg = limen_core::Config::load().unwrap_or_default();
    (cfg.gpg_key, cfg.gpg_user_id)
}

/// Whether `key` as `user_id` is the default: the chosen key, as the chosen
/// user id — or as its first, when none of its user ids was chosen.
fn is_default(key: &UserKey, user_id: &str, chosen: &(Option<String>, Option<String>)) -> bool {
    let Some(fingerprint) = &chosen.0 else { return false };
    if !fingerprint.eq_ignore_ascii_case(&key.fingerprint) {
        return false;
    }
    match chosen.1.as_deref().filter(|u| key.user_ids.iter().any(|k| k == u)) {
        Some(u) => u == user_id,
        None => key.user_id == user_id,
    }
}

impl FileSigner for UserSigner {
    fn identities(&self) -> Result<Vec<SignerIdentity>, String> {
        let keys = Gpg::configured().and_then(|gpg| gpg.secret_keys()).map_err(|e| e.to_string())?;
        let chosen = chosen();
        Ok(keys
            .iter()
            .filter(|k| k.can_sign)
            .flat_map(|k| {
                k.user_ids.iter().map(|u| SignerIdentity {
                    fingerprint: k.fingerprint.clone(),
                    user_id: u.clone(),
                    default: is_default(k, u, &chosen),
                })
            })
            .collect())
    }

    fn sign_file(&self, file: &Path, who: Option<(&str, &str)>) -> Result<SignedFile, String> {
        let gpg = Gpg::configured().map_err(|e| e.to_string())?;
        let signed = match who {
            Some((fingerprint, user_id)) => gpg.sign_file(fingerprint, Some(user_id), file),
            None => {
                let (key, user_id) = chosen();
                let key = key.ok_or(GpgError::NoKeyChosen).map_err(|e| e.to_string())?;
                // A user id chosen once and since gone from the key: its first.
                let user_id = user_id.filter(|u| gpg.key(&key).is_ok_and(|k| k.user_ids.contains(u)));
                gpg.sign_file(&key, user_id.as_deref(), file)
            }
        }
        .map_err(|e| e.to_string())?;
        Ok(SignedFile { signature: signed.signature, fingerprint: signed.fingerprint, user_id: signed.user_id })
    }
}

/// What a look at GnuPG found: the program used and the keys it has, or why not.
type Found = Result<(String, Vec<UserKey>), GpgError>;

pub(crate) struct OpenPgpUi {
    ctx: egui::Context,
    /// As last looked. None: not looked yet.
    found: Option<Found>,
    looking: bool,
    looks: (mpsc::Sender<Found>, mpsc::Receiver<Found>),
    /// The chosen key's fingerprint and which of its user ids, as the settings
    /// have them.
    chosen: Option<String>,
    chosen_user_id: Option<String>,
    /// The custom `gpg` path: the field, and what the settings have.
    path_field: String,
    path_saved: String,
    picks: (mpsc::Sender<String>, mpsc::Receiver<String>),
}

impl OpenPgpUi {
    /// Plug the signer into the host, and take the settings.
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        limen_host::set_file_signer(Some(Arc::new(UserSigner)));
        let cfg = limen_core::Config::load().unwrap_or_default();
        let path = cfg.gpg_path.unwrap_or_default();
        Self {
            ctx: ctx.clone(),
            found: None,
            looking: false,
            looks: mpsc::channel(),
            chosen: cfg.gpg_key,
            chosen_user_id: cfg.gpg_user_id,
            path_field: path.clone(),
            path_saved: path,
            picks: mpsc::channel(),
        }
    }

    /// Ask GnuPG again which keys there are.
    fn look(&mut self) {
        if self.looking {
            return;
        }
        self.looking = true;
        let (tx, ctx) = (self.looks.0.clone(), self.ctx.clone());
        let custom = self.path_saved.clone();
        std::thread::spawn(move || {
            let found = Gpg::locate(Some(&custom)).and_then(|gpg| {
                let keys = gpg.secret_keys()?;
                Ok((gpg.program().display().to_string(), keys))
            });
            let _ = tx.send(found);
            ctx.request_repaint();
        });
    }

    fn save(&self) {
        if let Ok(mut cfg) = limen_core::Config::load() {
            cfg.gpg_key = self.chosen.clone();
            cfg.gpg_user_id = self.chosen_user_id.clone();
            cfg.gpg_path = Some(self.path_saved.clone()).filter(|p| !p.is_empty());
            let _ = cfg.save();
        }
    }

    /// The "Your key" part of the OpenPGP page.
    pub(crate) fn section(&mut self, ui: &mut egui::Ui) {
        let muted = |ui: &mut egui::Ui, t: String| {
            ui.label(egui::RichText::new(t).small().color(ui::color::TEXT_MUTED));
        };
        while let Ok(found) = self.looks.1.try_recv() {
            self.looking = false;
            self.found = Some(found);
        }
        while let Ok(path) = self.picks.1.try_recv() {
            self.path_field = path;
        }
        if self.found.is_none() {
            self.look();
        }

        ui.label(egui::RichText::new(i18n::t("openpgp.your_key")).strong());
        muted(ui, i18n::t("openpgp.your_key_hint"));
        ui.add_space(8.0);

        // Which gpg.
        match &self.found {
            Some(Ok((program, _))) => muted(ui, i18n::t("openpgp.gpg_found").replace("{path}", program)),
            Some(Err(GpgError::NotInstalled)) => {
                ui.label(egui::RichText::new(i18n::t("openpgp.gpg_missing")).color(ui::color::WARNING));
            }
            Some(Err(GpgError::NotGpg(path))) => {
                ui.label(egui::RichText::new(i18n::t("openpgp.gpg_bad_path").replace("{path}", path)).color(ui::color::WARNING));
            }
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e.to_string()).color(ui::color::WARNING));
            }
            None => {}
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new(i18n::t("openpgp.gpg_path")).small());
        ui.horizontal(|ui| {
            ui::text_field(ui, &mut self.path_field, &i18n::t("openpgp.gpg_path_auto"), (ui.available_width() - 290.0).clamp(160.0, 520.0), false);
            if ui::outline_button(ui, &i18n::t("openpgp.browse"), egui::Vec2::ZERO).clicked() {
                let (tx, ctx) = (self.picks.0.clone(), self.ctx.clone());
                std::thread::spawn(move || {
                    if let Some(p) = rfd::FileDialog::new().pick_file() {
                        let _ = tx.send(p.display().to_string());
                        ctx.request_repaint();
                    }
                });
            }
            let changed = self.path_field.trim() != self.path_saved;
            let apply = ui.add_enabled_ui(changed && !self.looking, |ui| ui::primary_button(ui, &i18n::t("openpgp.apply"), egui::Vec2::ZERO)).inner;
            if apply.clicked() {
                self.path_saved = self.path_field.trim().to_string();
                self.path_field = self.path_saved.clone();
                self.save();
                self.look();
            }
            if self.looking {
                ui.spinner();
            }
        });
        muted(ui, i18n::t("openpgp.gpg_path_hint"));
        ui.add_space(10.0);

        // Which key.
        let keys: Vec<UserKey> = match &self.found {
            Some(Ok((_, keys))) => keys.clone(),
            _ => Vec::new(),
        };
        if matches!(self.found, Some(Ok(_))) && keys.is_empty() {
            muted(ui, i18n::t("openpgp.no_keys"));
        }
        // A key, then each of its user ids as one to sign as: the default is
        // one of those, not the key alone.
        let now = (self.chosen.clone(), self.chosen_user_id.clone());
        let mut choose: Option<Option<(String, String)>> = None;
        for key in &keys {
            let mut notes = vec![super::pages::in_fours(&key.fingerprint)];
            if key.on_card {
                notes.push(i18n::t("openpgp.on_card"));
            }
            if !key.can_sign {
                notes.push(i18n::t("openpgp.cannot_sign"));
            }
            ui.label(egui::RichText::new(notes.join(" · ")).small().monospace().color(ui::color::TEXT_MUTED));
            for user_id in &key.user_ids {
                let is_chosen = is_default(key, user_id, &now);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let pick = ui.add_enabled_ui(key.can_sign, |ui| ui::chip(ui, &i18n::t(if is_chosen { "openpgp.chosen" } else { "openpgp.choose" }), is_chosen)).inner;
                    if pick.clicked() {
                        choose = Some(if is_chosen { None } else { Some((key.fingerprint.clone(), user_id.clone())) });
                    }
                    let mut name = egui::RichText::new(user_id);
                    if !key.can_sign {
                        name = name.color(ui::color::TEXT_MUTED);
                    }
                    ui.label(name);
                });
            }
            ui.add_space(8.0);
        }
        // A key chosen before that GnuPG does not have now.
        if let (Some(chosen), Some(Ok(_))) = (&self.chosen, &self.found)
            && !keys.iter().any(|k| k.fingerprint.eq_ignore_ascii_case(chosen))
        {
            ui.label(egui::RichText::new(i18n::t("openpgp.chosen_missing").replace("{key}", &super::pages::in_fours(chosen))).color(ui::color::WARNING));
            if ui::outline_button(ui, &i18n::t("openpgp.forget"), egui::Vec2::ZERO).clicked() {
                choose = Some(None);
            }
        }
        if let Some(to) = choose {
            (self.chosen, self.chosen_user_id) = match to {
                Some((key, user_id)) => (Some(key), Some(user_id)),
                None => (None, None),
            };
            self.save();
        }
        ui.add_space(4.0);
        if ui.add_enabled_ui(!self.looking, |ui| ui::outline_button(ui, &i18n::t("openpgp.refresh"), egui::Vec2::ZERO)).inner.clicked() {
            self.look();
        }
    }
}
