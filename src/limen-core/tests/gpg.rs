//! The user's own key, through GnuPG: reading its key listing, and signing
//! with a throwaway key in a keyring of the test's own (never the user's).

use limen_core::gpg::*;

#[test]
fn a_key_listing_is_read() {
    // A key on disk, an expired one, one on a card with only an encryption
    // subkey usable, and a name with a colon in it.
    let listing = "\
sec:u:255:22:140ABA21244518B8:1795441837:1890049837::u:::scESC:::+:::23::0:
fpr:::::::::8D151FD231907502BBFA528E140ABA21244518B8:
grp:::::::::AAAA:
uid:u::::1795441837::HASH::CRC-BARRACUDA <security@barracuda-team.tech>::::::::::0:
ssb:u:255:18:23EC6530F06DC429:1795441837:1890049837:::::e:::+:::23:
fpr:::::::::B8E36424AFA43F8672F2F59123EC6530F06DC429:
sec:e:255:22:4B44F376986C33D2:1577836800:1609372800::u:::sc:::+:::23::0:
fpr:::::::::10D0C1F70D669549879A08F94B44F376986C33D2:
uid:e::::1577836800::HASH::Limen Test Old <old@limen.invalid>::::::::::0:
sec:u:4096:1:04F5252E9BD3FC8D:1736035200:::u:::scESCA:::D2760001240100000006308767290000::23::0:
fpr:::::::::46AC14F35EEB6798C506E9CB04F5252E9BD3FC8D:
uid:u::::1736035200::HASH::Name\\x3a With Colon <n@example.invalid>::::::::::0:
uid:u::::1736035200::HASH::Second Id <second@example.invalid>::::::::::0:
";
    let keys = parse_secret_keys(listing);
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[0].fingerprint, "8D151FD231907502BBFA528E140ABA21244518B8", "the primary's, not the subkey's");
    assert_eq!(keys[0].user_id, "CRC-BARRACUDA <security@barracuda-team.tech>");
    assert!(keys[0].can_sign && !keys[0].on_card);
    assert!(!keys[1].can_sign, "expired");
    assert_eq!(keys[2].user_id, "Name: With Colon <n@example.invalid>", "the first id, unescaped");
    assert_eq!(keys[2].user_ids, ["Name: With Colon <n@example.invalid>", "Second Id <second@example.invalid>"], "all of them");
    assert_eq!(keys[0].user_ids, ["CRC-BARRACUDA <security@barracuda-team.tech>"]);
    // A revoked user id is not one to sign as; a subkey's lines are not the key's.
    let revoked = "sec:u:255:22:AA:1::::::scSC:::+:::23::0:\nfpr:::::::::AB12:\nuid:r::::1::H::Old Name <old@example.invalid>::::::::::0:\nuid:u::::1::H::New Name <new@example.invalid>::::::::::0:\nssb:u:255:18:BB:1::::::e:::+:::23:\nfpr:::::::::CD34:\n";
    let k = &parse_secret_keys(revoked)[0];
    assert_eq!((k.user_id.as_str(), k.user_ids.len()), ("New Name <new@example.invalid>", 1));
    assert!(keys[2].can_sign && keys[2].on_card);
    assert!(parse_secret_keys("").is_empty());
    // A name in another alphabet, and a stray backslash, are read as they are.
    let odd = "sec:u:255:22:AA:1::::::scSC:::+:::23::0:\nfpr:::::::::AB12:\nuid:u::::1::H::Іван \\xЖЖ <i@example.invalid>::::::::::0:\n";
    assert_eq!(parse_secret_keys(odd)[0].user_id, "Іван \\xЖЖ <i@example.invalid>");
}

/// A keyring of the test's own, with one throwaway key in it. None when there
/// is no GnuPG to test with.
fn keyring(name: &str) -> Option<(Gpg, std::path::PathBuf, String)> {
    let gpg = Gpg::find().ok()?;
    let home = std::env::temp_dir().join(format!("limen-gpg-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let made = std::process::Command::new("gpg")
        .arg("--homedir")
        .arg(&home)
        .args(["--batch", "--quiet", "--passphrase", "", "--pinentry-mode", "loopback"])
        .args(["--quick-generate-key", "Limen Test User <user@limen.invalid>", "ed25519", "sign", "1d"])
        .output()
        .ok()?;
    if !made.status.success() {
        return None;
    }
    let gpg = gpg.with_home(&home);
    let fingerprint = gpg.secret_keys().ok()?.first()?.fingerprint.clone();
    // A second user id on the same key, as a key with two addresses has.
    let added = std::process::Command::new("gpg")
        .arg("--homedir")
        .arg(&home)
        .args(["--batch", "--quiet", "--passphrase", "", "--pinentry-mode", "loopback"])
        .args(["--quick-add-uid", &fingerprint, "Limen Test Other <other@limen.invalid>"])
        .output()
        .ok()?;
    if !added.status.success() {
        return None;
    }
    Some((gpg, home, fingerprint))
}

fn done(home: &std::path::Path) {
    let _ = std::process::Command::new("gpgconf").arg("--homedir").arg(home).args(["--kill", "gpg-agent"]).output();
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn a_file_is_signed_with_the_chosen_key_and_the_signature_verifies() {
    let Some((gpg, home, fingerprint)) = keyring("sign") else {
        eprintln!("no GnuPG here: skipped");
        return;
    };
    let keys = gpg.secret_keys().unwrap();
    assert_eq!(keys.len(), 1);
    assert!(keys[0].can_sign && !keys[0].on_card);
    let (user, other) = ("Limen Test User <user@limen.invalid>", "Limen Test Other <other@limen.invalid>");
    let mut ids = keys[0].user_ids.clone();
    ids.sort();
    assert_eq!(ids, [other, user], "both user ids of the key");

    let report = home.join("report.pdf");
    std::fs::write(&report, b"%PDF-1.7 pretend").unwrap();
    // Whose it is, before anything is signed.
    assert_eq!(gpg.key(&fingerprint).unwrap().user_ids.len(), 2);
    assert_eq!(gpg.key(""), Err(GpgError::NoKeyChosen));
    assert!(matches!(gpg.key("00FF"), Err(GpgError::KeyMissing(_))));
    let signed = gpg.sign_file(&fingerprint, Some(user), &report).expect("signed");
    assert_eq!(signed.signature, home.join("report.pdf.asc"));
    assert_eq!(signed.fingerprint, fingerprint);
    assert_eq!(signed.user_id, user);
    let armored = std::fs::read_to_string(&signed.signature).unwrap();
    assert!(armored.starts_with("-----BEGIN PGP SIGNATURE-----"));
    // GnuPG itself agrees it is that key's signature of that file.
    let verified = std::process::Command::new("gpg")
        .arg("--homedir")
        .arg(&home)
        .args(["--batch", "--status-fd", "1", "--verify"])
        .arg(&signed.signature)
        .arg(&report)
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&verified.stdout);
    assert!(status.contains("GOODSIG") && status.contains(&format!("VALIDSIG {fingerprint}")), "{status}");

    // Which user id it was signed as is in the signature itself.
    // (The packet itself, un-armored: the address is in it as it was given.)
    let signer_of = |sig: &std::path::Path| {
        let out = std::process::Command::new("gpg").arg("--homedir").arg(&home).args(["--batch", "--dearmor", "--output", "-"]).arg(sig).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(signer_of(&signed.signature).contains("user@limen.invalid"), "{}", signer_of(&signed.signature));

    // Signing again replaces the signature rather than failing on it — this
    // time as the key's other user id.
    let again = gpg.sign_file(&fingerprint.to_lowercase(), Some(other), &report).expect("again, and the case of the fingerprint is no matter");
    assert_eq!(again.user_id, other);
    assert!(signer_of(&again.signature).contains("other@limen.invalid"), "{}", signer_of(&again.signature));
    // No user id asked for: the key's first. One it does not have: refused.
    assert_eq!(gpg.sign_file(&fingerprint, None, &report).unwrap().user_id, keys[0].user_id);
    assert!(matches!(gpg.sign_file(&fingerprint, Some("Somebody Else <x@example.invalid>"), &report), Err(GpgError::Failed(_))));
    assert!(matches!(gpg.sign_file(&fingerprint, Some("--passphrase-fd 0"), &report), Err(GpgError::Failed(_))));
    done(&home);
}

#[test]
fn what_cannot_be_signed_says_why() {
    let Some((gpg, home, fingerprint)) = keyring("refuse") else {
        eprintln!("no GnuPG here: skipped");
        return;
    };
    let report = home.join("r.pdf");
    std::fs::write(&report, b"x").unwrap();
    assert_eq!(gpg.sign_file("", None, &report), Err(GpgError::NoKeyChosen));
    assert_eq!(gpg.sign_file("  ", None, &report), Err(GpgError::NoKeyChosen));
    let other = "0000000000000000000000000000000000000000";
    assert_eq!(gpg.sign_file(other, None, &report), Err(GpgError::KeyMissing(other.into())));
    // Not a fingerprint at all: never handed to gpg as an argument.
    assert!(matches!(gpg.sign_file("--export-secret-keys", None, &report), Err(GpgError::KeyMissing(_))));
    assert!(matches!(gpg.sign_file(&fingerprint, None, &home.join("not-there.pdf")), Err(GpgError::Failed(_))));
    assert!(!home.join("not-there.pdf.asc").exists());
    done(&home);
}

#[test]
fn a_gpg_named_in_the_settings_is_the_one_used_or_none_is() {
    let Ok(installed) = Gpg::find() else {
        eprintln!("no GnuPG here: skipped");
        return;
    };
    // No path given (or an empty one): the installed one.
    assert_eq!(Gpg::locate(None).unwrap().program(), installed.program());
    assert_eq!(Gpg::locate(Some("  ")).unwrap().program(), installed.program());

    // A path that is not there, and a file that is no gpg: said, and nothing
    // found in their place.
    let dir = std::env::temp_dir().join(format!("limen-gpg-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("gpg").display().to_string();
    assert_eq!(Gpg::locate(Some(&missing)).err(), Some(GpgError::NotGpg(missing.clone())));
    let not_gpg = dir.join("notes.txt");
    std::fs::write(&not_gpg, b"not a program").unwrap();
    assert!(matches!(Gpg::locate(Some(not_gpg.to_str().unwrap())), Err(GpgError::NotGpg(_))));

    // The real one, by its full path — as a portable copy would be named.
    #[cfg(unix)]
    {
        let which = std::process::Command::new("sh").args(["-c", "command -v gpg"]).output().unwrap();
        let full = String::from_utf8_lossy(&which.stdout).trim().to_string();
        let found = Gpg::locate(Some(&full)).expect("a full path to gpg");
        assert_eq!(found.program(), std::path::Path::new(&full));
        assert!(found.secret_keys().is_ok());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
