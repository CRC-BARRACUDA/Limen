//! Whether a module being installed was signed by the team.
//!
//! **What is signed.** A module is installed from its repository, as files —
//! there is no one archive to sign. So the team signs a *statement*: one line
//! naming the module, its version, and the digest of its files, the same
//! digest the lockfile keeps ([`statement`]). The detached signature of that
//! line is published with the module's release, as `limen-module.sig` (or
//! `.asc`). Limen works the statement out again from what it fetched, and
//! checks the signature against that: files changed since they were signed
//! make another statement, and the signature does not fit it.
//!
//! Signing again — a new key, say — is uploading a new signature file to the
//! same release: the module itself is not touched.
//!
//! **A native module's library** is not in the repository: it is a file of the
//! release, downloaded for this platform. It has a signature of its own beside
//! it, as Limen's own build has — the library's name with `.sig` or `.asc`.
//! A native module is the team's only if both are.
//!
//! **A module from a local path** is the developer's own: nothing is checked.

use limen_sign::{Keyring, Verdict};

/// The names a module's statement signature may have among a release's files.
pub const SIGNATURE_ASSETS: [&str; 2] = ["limen-module.sig", "limen-module.asc"];

/// The line the team signs for a module: which module, which version, and the
/// digest of its files (`sha256:…`, as [`crate::digest_dir`] gives it).
pub fn statement(name: &str, version: &str, digest: &str) -> String {
    format!("limen-module {name} {version} {digest}\n")
}

/// From a release's `(name, url)` files, the statement's signature.
pub(crate) fn statement_signature(assets: &[(String, String)]) -> Option<&str> {
    SIGNATURE_ASSETS
        .iter()
        .find_map(|want| assets.iter().find(|(n, _)| n.eq_ignore_ascii_case(want)))
        .map(|(_, url)| url.as_str())
}

/// From a release's files, the signature of the file named `asset`.
pub(crate) fn asset_signature<'a>(assets: &'a [(String, String)], asset: &str) -> Option<&'a str> {
    [".sig", ".asc"]
        .iter()
        .find_map(|ext| assets.iter().find(|(n, _)| n.strip_suffix(ext).is_some_and(|base| base.eq_ignore_ascii_case(asset))))
        .map(|(_, url)| url.as_str())
}

/// A small file from the network: a signature.
pub(crate) fn fetch(url: &str) -> Option<Vec<u8>> {
    let out = crate::http::get(&["-fsSL", "--max-filesize", "1000000"], None, url).ok()?;
    out.status.success().then_some(out.stdout)
}

/// Check a module's files — already digested — against its statement's
/// signature.
pub(crate) fn check_statement(keys: &Keyring, name: &str, version: &str, digest: &str, signature: Option<&[u8]>) -> Verdict {
    keys.verify(statement(name, version, digest).as_bytes(), signature)
}

/// A module's signature and its library's, as one: the worse of the two. A
/// wrong one refuses; else anything not the team's makes the whole not the
/// team's; only two good ones are good.
pub(crate) fn both(source: Verdict, library: Verdict) -> Verdict {
    fn rank(v: &Verdict) -> u8 {
        match v {
            Verdict::Bad(_) => 0,
            Verdict::UnknownSigner { .. } => 1,
            Verdict::Unsigned => 2,
            Verdict::Valid(_) => 3,
        }
    }
    if rank(&library) < rank(&source) { library } else { source }
}

/// Whether what was resolved may be installed: each module's name, version and
/// verdict (None: from a local path, not checked). A wrong signature refuses
/// all of it, said plainly; modules not the team's hold it back unless
/// `accept_unsigned` — as [`UnsignedModules`], for the caller to ask about.
pub(crate) fn admit(modules: &[(String, String, Option<Verdict>)], accept_unsigned: bool) -> anyhow::Result<()> {
    let mut unsigned = Vec::new();
    for (name, version, verdict) in modules {
        match verdict {
            Some(Verdict::Bad(why)) => anyhow::bail!("{name} {version} is refused: {why}"),
            Some(Verdict::Unsigned) => {
                unsigned.push(UnsignedModule { name: name.clone(), version: version.clone(), unknown_key: None })
            }
            Some(Verdict::UnknownSigner { key }) => unsigned.push(UnsignedModule {
                name: name.clone(),
                version: version.clone(),
                unknown_key: Some(key.clone()),
            }),
            Some(Verdict::Valid(_)) | None => {}
        }
    }
    if unsigned.is_empty() || accept_unsigned {
        Ok(())
    } else {
        Err(UnsignedModules { modules: unsigned }.into())
    }
}

/// A module that is not signed by the team: by an unknown key, or by none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsignedModule {
    pub name: String,
    pub version: String,
    pub unknown_key: Option<String>,
}

/// Why an install stopped short: these modules are not signed by the team.
/// Nothing was installed. Installing them all the same is the user's to say —
/// see [`crate::Registry::accept_unsigned`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsignedModules {
    pub modules: Vec<UnsignedModule>,
}

impl std::fmt::Display for UnsignedModules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self.modules.iter().map(|m| format!("{} {}", m.name, m.version)).collect();
        write!(f, "not signed by the Limen team: {}", names.join(", "))
    }
}

impl std::error::Error for UnsignedModules {}

#[cfg(test)]
mod tests {
    use super::*;
    use limen_sign::{KeyStatus, TrustedKey};

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// Trusting the throwaway key the fixtures were signed with.
    pub(crate) fn test_keys() -> Keyring {
        let text = |n: &str| -> &'static str { Box::leak(String::from_utf8(fixture(n)).unwrap().into_boxed_str()) };
        Keyring::from_keys(&[TrustedKey {
            name: "test team",
            fingerprint: text("fingerprint.txt").trim(),
            armored: text("key.asc"),
            status: KeyStatus::Active,
        }])
        .unwrap()
    }

    fn module_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(format!("{}/tests/fixtures/signed-module", env!("CARGO_MANIFEST_DIR")))
    }

    #[test]
    fn a_modules_files_as_signed_are_the_teams() {
        let digest = crate::digest_dir(&module_dir()).unwrap();
        let sig = fixture("limen-module.asc");
        let verdict = check_statement(&test_keys(), "signed-demo", "1.2.3", &digest, Some(&sig));
        assert!(verdict.is_valid(), "{verdict:?} for {digest}");
        // Another version, another name, or other files: not what was signed.
        for (name, version, digest) in [
            ("signed-demo", "1.2.4", digest.as_str()),
            ("other", "1.2.3", digest.as_str()),
            ("signed-demo", "1.2.3", "sha256:00"),
        ] {
            assert!(check_statement(&test_keys(), name, version, digest, Some(&sig)).refuses(), "{name} {version} {digest}");
        }
        assert_eq!(check_statement(&test_keys(), "signed-demo", "1.2.3", &digest, None), Verdict::Unsigned);
        // To Limen's own keys the throwaway key is a stranger.
        assert!(matches!(
            check_statement(Keyring::team(), "signed-demo", "1.2.3", &digest, Some(&sig)),
            Verdict::UnknownSigner { .. }
        ));
    }

    #[test]
    fn the_signatures_are_found_among_a_releases_files() {
        let a = |n: &str| (n.to_string(), format!("https://x/{n}"));
        let assets = vec![a("demo-linux-x86_64.so"), a("demo-linux-x86_64.so.sha256"), a("demo-linux-x86_64.so.sig"), a("limen-module.asc")];
        assert_eq!(statement_signature(&assets), Some("https://x/limen-module.asc"));
        assert_eq!(asset_signature(&assets, "demo-linux-x86_64.so"), Some("https://x/demo-linux-x86_64.so.sig"));
        assert_eq!(asset_signature(&assets, "demo-windows-x86_64.dll"), None);
        assert_eq!(statement_signature(&assets[..3]), None);
        // `.sig` is taken before `.asc` when both are there.
        let two = vec![a("limen-module.asc"), a("limen-module.sig")];
        assert_eq!(statement_signature(&two), Some("https://x/limen-module.sig"));
    }

    #[test]
    fn a_wrong_signature_refuses_and_a_missing_one_asks() {
        let m = |name: &str, v: Option<Verdict>| (name.to_string(), "1.0.0".to_string(), v);
        let good = test_keys().verify(&fixture("native-lib.bin"), Some(&fixture("native-lib.bin.sig")));
        let ok = vec![m("signed", Some(good.clone())), m("local", None)];
        assert!(admit(&ok, false).is_ok(), "the team's, and a local one: nothing to ask");

        let mixed = vec![m("signed", Some(good.clone())), m("plain", Some(Verdict::Unsigned)), m("odd", Some(Verdict::UnknownSigner { key: "AB".into() }))];
        let held = admit(&mixed, false).unwrap_err();
        let unsigned = held.downcast_ref::<UnsignedModules>().expect("held back, to be asked about");
        assert_eq!(unsigned.modules.len(), 2);
        assert_eq!((unsigned.modules[0].name.as_str(), &unsigned.modules[0].unknown_key), ("plain", &None));
        assert_eq!(unsigned.modules[1].unknown_key.as_deref(), Some("AB"));
        assert!(admit(&mixed, true).is_ok(), "the user said so");

        let tampered = vec![m("plain", Some(Verdict::Unsigned)), m("evil", Some(Verdict::Bad("does not match".into())))];
        for accept in [false, true] {
            let err = admit(&tampered, accept).unwrap_err();
            assert!(err.downcast_ref::<UnsignedModules>().is_none(), "not a question: a refusal");
            assert!(err.to_string().contains("evil") && err.to_string().contains("refused"), "{err}");
        }
    }

    #[test]
    fn a_library_is_checked_as_a_file() {
        let lib = fixture("native-lib.bin");
        let sig = fixture("native-lib.bin.sig");
        assert!(test_keys().verify(&lib, Some(&sig)).is_valid());
        let mut changed = lib.clone();
        changed[0] ^= 1;
        assert!(test_keys().verify(&changed, Some(&sig)).refuses());
    }

    #[test]
    fn a_module_and_its_library_are_as_good_as_the_worse_of_them() {
        let good = || test_keys().verify(&fixture("native-lib.bin"), Some(&fixture("native-lib.bin.sig")));
        let bad = || Verdict::Bad("changed".into());
        let stranger = || Verdict::UnknownSigner { key: "AB".into() };
        assert!(both(good(), good()).is_valid());
        assert_eq!(both(good(), Verdict::Unsigned), Verdict::Unsigned);
        assert_eq!(both(Verdict::Unsigned, good()), Verdict::Unsigned);
        assert_eq!(both(stranger(), Verdict::Unsigned), stranger());
        assert!(both(good(), bad()).refuses());
        assert!(both(bad(), good()).refuses());
        assert!(both(Verdict::Unsigned, bad()).refuses());
    }
}
