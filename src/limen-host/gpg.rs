//! Signing a file with the user's own OpenPGP key, for a module:
//! `host.gpg_sign_file` — and `host.gpg_identities`, the keys and user ids
//! there are to sign as, so the user can pick one at the moment of signing and
//! a document can say who signed it before it is signed.
//!
//! The key is the user's, kept by their GnuPG; a module never sees it. What a
//! module can do is ask for a signature — and only of a file the user chose a
//! place for through that module's own save dialog (`host.save_file`), in this
//! session. So a module can have its report signed where the user saved it,
//! and cannot have the user's name put to a file of its own choosing.
//!
//! It must declare the `gpg` permission, which the user saw when they trusted
//! it. The signer itself lives above the host (GnuPG, through limen-core, with
//! the key picked in the GUI); it is plugged in with [`set_file_signer`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use limen_proto::rpc::{self, RpcError};
use serde_json::{json, Value};

use crate::Logger;

/// A file signed: where the detached signature is, and whose it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedFile {
    pub signature: PathBuf,
    /// The signing key's fingerprint, 40 hex digits.
    pub fingerprint: String,
    /// Its user id: "Name <email>".
    pub user_id: String,
}

/// One way the user can sign: a key of theirs, as one of its user ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerIdentity {
    /// 40 hex digits.
    pub fingerprint: String,
    /// "Name <email>".
    pub user_id: String,
    /// The one chosen on the OpenPGP page: what signs when none is named.
    pub default: bool,
}

/// What signs: the user's GnuPG, with the key they chose.
pub trait FileSigner: Send + Sync {
    /// Every key the user can sign with, as each of its user ids. Asks the
    /// user nothing. The error is words for the user: no GnuPG.
    fn identities(&self) -> Result<Vec<SignerIdentity>, String>;
    /// Sign `file`, writing a detached signature beside it — as `who`
    /// (fingerprint, user id), or as the one chosen on the OpenPGP page. The
    /// error is words for the user: no GnuPG, no key chosen, the passphrase
    /// turned down.
    fn sign_file(&self, file: &Path, who: Option<(&str, &str)>) -> Result<SignedFile, String>;
}

static SIGNER: RwLock<Option<Arc<dyn FileSigner>>> = RwLock::new(None);

/// Plug the signer in (or take it out).
pub fn set_file_signer(signer: Option<Arc<dyn FileSigner>>) {
    if let Ok(mut s) = SIGNER.write() {
        *s = signer;
    }
}

/// The files the user chose a place for, per module, this session.
static CHOSEN: Mutex<Option<HashMap<String, HashSet<PathBuf>>>> = Mutex::new(None);

/// Module `who`'s save dialog returned `path`: the user chose it.
pub(crate) fn remember_chosen(who: &str, path: &str) {
    if let Ok(mut chosen) = CHOSEN.lock() {
        chosen.get_or_insert_with(HashMap::new).entry(who.to_string()).or_default().insert(PathBuf::from(path));
    }
}

/// Whether `path` is a file the user chose through `who`'s save dialog: the
/// path the dialog gave, or that path with an extension added to it — a name
/// typed without one is given one by the module, and it is the same file.
fn was_chosen(who: &str, path: &Path) -> bool {
    let Ok(chosen) = CHOSEN.lock() else { return false };
    let Some(set) = chosen.as_ref().and_then(|m| m.get(who)) else { return false };
    if set.contains(path) {
        return true;
    }
    let bare = path.with_extension("");
    path.extension().is_some() && bare.extension().is_none() && set.contains(&bare)
}

/// Answer `host.gpg_sign_file` for module `who`.
pub(crate) fn host_gpg(
    method: &str,
    params: Value,
    may: bool,
    log: &Logger,
    who: &str,
) -> std::result::Result<Value, RpcError> {
    if !matches!(method, "host.gpg_sign_file" | "host.gpg_identities") {
        return Err(RpcError::new(rpc::METHOD_NOT_FOUND, format!("no {method}")));
    }
    if !may {
        log(&format!("[gpg] {who}: refused, no `gpg` permission"));
        return Err(RpcError::new(rpc::INVALID_REQUEST, "this module does not declare the `gpg` permission"));
    }
    if method == "host.gpg_identities" {
        let Some(signer) = SIGNER.read().ok().and_then(|s| s.clone()) else {
            return Err(RpcError::new(rpc::INTERNAL_ERROR, "signing with OpenPGP is not available here"));
        };
        return match signer.identities() {
            Ok(ids) => Ok(json!({
                "identities": ids
                    .iter()
                    .map(|id| json!({ "fingerprint": id.fingerprint, "user_id": id.user_id, "default": id.default }))
                    .collect::<Vec<_>>(),
            })),
            Err(why) => Err(RpcError::new(rpc::INTERNAL_ERROR, why)),
        };
    }
    let path = params.get("path").and_then(Value::as_str).unwrap_or_default();
    if path.is_empty() {
        return Err(RpcError::new(rpc::INVALID_PARAMS, "which file: `path`"));
    }
    let path = PathBuf::from(path);
    if !was_chosen(who, &path) {
        log(&format!("[gpg] {who}: refused, {} is not a file the user saved through it", path.display()));
        return Err(RpcError::new(
            rpc::INVALID_PARAMS,
            "only a file the user chose through this module's save dialog can be signed",
        ));
    }
    let Some(signer) = SIGNER.read().ok().and_then(|s| s.clone()) else {
        return Err(RpcError::new(rpc::INTERNAL_ERROR, "signing with OpenPGP is not available here"));
    };
    // As whom: a key and one of its user ids, both or neither. The signer
    // checks that they are the user's own.
    let text = |key: &str| params.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    let as_whom = match (text("fingerprint"), text("user_id")) {
        (Some(f), Some(u)) => Some((f, u)),
        (None, None) => None,
        _ => return Err(RpcError::new(rpc::INVALID_PARAMS, "sign as whom: `fingerprint` and `user_id`, or neither")),
    };
    match signer.sign_file(&path, as_whom) {
        Ok(signed) => {
            log(&format!("[gpg] {who}: signed {} as {}", path.display(), signed.user_id));
            Ok(json!({
                "signature": signed.signature.to_string_lossy(),
                "fingerprint": signed.fingerprint,
                "user_id": signed.user_id,
            }))
        }
        Err(why) => {
            log(&format!("[gpg] {who}: not signed, {why}"));
            Err(RpcError::new(rpc::INTERNAL_ERROR, why))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signer that writes a pretend signature, or refuses.
    struct Fake(Result<(), &'static str>);

    impl FileSigner for Fake {
        fn identities(&self) -> Result<Vec<SignerIdentity>, String> {
            self.0.map_err(str::to_string)?;
            let id = |f: &str, u: &str, default| SignerIdentity { fingerprint: f.into(), user_id: u.into(), default };
            Ok(vec![
                id("AB12", "Me <me@example.invalid>", true),
                id("AB12", "Me at work <me@work.invalid>", false),
                id("CD34", "Me, the other token <me@example.invalid>", false),
            ])
        }
        fn sign_file(&self, file: &Path, who: Option<(&str, &str)>) -> Result<SignedFile, String> {
            self.0.map_err(str::to_string)?;
            let (fingerprint, user_id) = who.unwrap_or(("AB12", "Me <me@example.invalid>"));
            let mut sig = file.as_os_str().to_owned();
            sig.push(".asc");
            Ok(SignedFile { signature: sig.into(), fingerprint: fingerprint.into(), user_id: user_id.into() })
        }
    }

    /// One test at a time: the signer is one global.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    fn logged() -> (Logger, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let l = lines.clone();
        (Arc::new(move |s: &str| l.lock().unwrap().push(s.to_string())), lines)
    }

    #[test]
    fn a_module_has_signed_only_what_the_user_saved_through_it() {
        let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        set_file_signer(Some(Arc::new(Fake(Ok(())))));
        let (log, lines) = logged();
        let sign = |path: &str, may: bool, who: &str| host_gpg("host.gpg_sign_file", json!({ "path": path }), may, &log, who);

        remember_chosen("report", "/home/me/report.pdf");
        let out = sign("/home/me/report.pdf", true, "report").expect("the user saved it through this module");
        assert_eq!(out["signature"], "/home/me/report.pdf.asc");
        assert_eq!(out["fingerprint"], "AB12");
        assert_eq!(out["user_id"], "Me <me@example.invalid>");

        // A name the user typed without an extension, which the module gave
        // one: the same file. Not so a name that had one, changed.
        remember_chosen("report", "/home/me/typed");
        assert!(sign("/home/me/typed.pdf", true, "report").is_ok());
        assert_eq!(sign("/home/me/report.exe", true, "report").unwrap_err().code, rpc::INVALID_PARAMS);
        assert_eq!(sign("/home/me/report.pdf.sh", true, "report").unwrap_err().code, rpc::INVALID_PARAMS);
        assert_eq!(sign("/home/me/typed.tar.gz", true, "report").unwrap_err().code, rpc::INVALID_PARAMS);

        // No permission; a file of the module's own choosing; another module's file.
        assert_eq!(sign("/home/me/report.pdf", false, "report").unwrap_err().code, rpc::INVALID_REQUEST);
        assert_eq!(sign("/home/me/.ssh/authorized_keys", true, "report").unwrap_err().code, rpc::INVALID_PARAMS);
        assert_eq!(sign("/home/me/report.pdf", true, "other").unwrap_err().code, rpc::INVALID_PARAMS);
        assert_eq!(sign("", true, "report").unwrap_err().code, rpc::INVALID_PARAMS);
        assert_eq!(
            host_gpg("host.gpg_export", json!({}), true, &log, "report").unwrap_err().code,
            rpc::METHOD_NOT_FOUND
        );
        // What there is to sign as, before anything is signed — with the
        // permission only: every key, as each of its user ids.
        let ids = host_gpg("host.gpg_identities", json!({}), true, &log, "report").unwrap();
        let ids = ids["identities"].as_array().unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!((ids[0]["fingerprint"].as_str(), ids[0]["default"].as_bool()), (Some("AB12"), Some(true)));
        assert_eq!(ids[1]["user_id"], "Me at work <me@work.invalid>");
        assert_eq!(ids[2]["fingerprint"], "CD34");
        assert_eq!(host_gpg("host.gpg_identities", json!({}), false, &log, "report").unwrap_err().code, rpc::INVALID_REQUEST);

        // Signed as the one picked: another user id, another key.
        let pick = |f: &str, u: &str| {
            host_gpg("host.gpg_sign_file", json!({ "path": "/home/me/report.pdf", "fingerprint": f, "user_id": u }), true, &log, "report")
        };
        let as_work = pick("AB12", "Me at work <me@work.invalid>").unwrap();
        assert_eq!(as_work["user_id"], "Me at work <me@work.invalid>");
        assert_eq!(pick("CD34", "Me, the other token <me@example.invalid>").unwrap()["fingerprint"], "CD34");
        // Half an answer is none.
        let half = host_gpg("host.gpg_sign_file", json!({ "path": "/home/me/report.pdf", "fingerprint": "AB12" }), true, &log, "report");
        assert_eq!(half.unwrap_err().code, rpc::INVALID_PARAMS);
        let lines = lines.lock().unwrap();
        assert!(lines.iter().any(|l| l.contains("report: signed /home/me/report.pdf as Me")), "{lines:?}");
        set_file_signer(None);
    }

    #[test]
    fn a_refusal_by_the_signer_and_no_signer_at_all_say_so() {
        let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let (log, _) = logged();
        remember_chosen("report", "/tmp/r.pdf");
        set_file_signer(None);
        let none = host_gpg("host.gpg_sign_file", json!({ "path": "/tmp/r.pdf" }), true, &log, "report").unwrap_err();
        assert_eq!(none.code, rpc::INTERNAL_ERROR);
        set_file_signer(Some(Arc::new(Fake(Err("no OpenPGP key is chosen")))));
        let refused = host_gpg("host.gpg_sign_file", json!({ "path": "/tmp/r.pdf" }), true, &log, "report").unwrap_err();
        assert_eq!(refused.code, rpc::INTERNAL_ERROR);
        assert!(refused.message.contains("no OpenPGP key is chosen"), "{}", refused.message);
        let nobody = host_gpg("host.gpg_identities", json!({}), true, &log, "report").unwrap_err();
        assert!(nobody.message.contains("no OpenPGP key is chosen"), "{}", nobody.message);
        set_file_signer(None);
    }
}
