//! Signatures made by `gpg` with throwaway keys (see `fixtures/`; their private
//! halves were deleted), checked against keyrings that trust some of them.

use limen_sign::*;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn fingerprint(who: &str) -> &'static str {
    let all = String::from_utf8(fixture("fingerprints.txt")).unwrap();
    let line = all.lines().find(|l| l.starts_with(who)).unwrap().to_string();
    Box::leak(line.split_whitespace().nth(1).unwrap().to_string().into_boxed_str())
}

fn key(who: &'static str, status: KeyStatus) -> TrustedKey {
    let armored = Box::leak(String::from_utf8(fixture(&format!("key.{who}.asc"))).unwrap().into_boxed_str());
    TrustedKey { name: who, fingerprint: fingerprint(who), armored, status }
}

/// Trusting `old` (retired) and `new` (active) — not `stranger`.
fn ring() -> Keyring {
    Keyring::from_keys(&[key("old", KeyStatus::Retired), key("new", KeyStatus::Active)]).unwrap()
}

fn check(ring: &Keyring, sig: &str) -> Verdict {
    ring.verify(&fixture("release.txt"), Some(&fixture(sig)))
}

#[test]
fn the_built_in_team_key_reads_and_is_the_pinned_one() {
    let team = Keyring::from_keys(TEAM_KEYS).expect("the built-in keys read");
    assert_eq!(TEAM_KEYS[0].fingerprint, "8D151FD231907502BBFA528E140ABA21244518B8");
    // As the OpenPGP page shows it.
    let shown = team.keys();
    assert_eq!(shown.len(), TEAM_KEYS.len());
    assert_eq!(shown[0].name, "CRC-BARRACUDA");
    assert_eq!(shown[0].fingerprint, TEAM_KEYS[0].fingerprint);
    assert_eq!(shown[0].status, KeyStatus::Active);
    assert_eq!(shown[0].user_ids, ["CRC-BARRACUDA <security@barracuda-team.tech>"]);
    // And it is what `verify` uses: nothing signed by it here, so a stranger's
    // signature is a stranger's.
    assert!(matches!(check(&team, "release.new.asc"), Verdict::UnknownSigner { .. }));
    assert_eq!(verify(b"x", None), Verdict::Unsigned);
}

#[test]
fn a_trusted_signature_is_valid_armored_or_binary_and_says_who() {
    for sig in ["release.new.asc", "release.new.sig"] {
        let Verdict::Valid(signer) = check(&ring(), sig) else { panic!("{sig} is the team's") };
        assert_eq!(signer.name, "new");
        assert_eq!(signer.fingerprint, fingerprint("new"));
        assert!(!signer.retired);
        assert!(signer.signed_at.is_some());
    }
}

#[test]
fn a_retired_key_long_expired_still_vouches_for_what_it_signed() {
    // `old` was made in 2020 for a year, and signed in 2020.
    for sig in ["release.old.asc", "release.old.sig"] {
        let Verdict::Valid(signer) = check(&ring(), sig) else { panic!("{sig}: an old release still verifies") };
        assert_eq!(signer.name, "old");
        assert!(signer.retired);
        let at = signer.signed_at.unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert!((1_590_000_000..1_600_000_000).contains(&at), "signed in 2020: {at}");
    }
}

#[test]
fn changed_content_under_a_team_signature_is_refused() {
    let mut changed = fixture("release.txt");
    changed[20] ^= 1;
    let verdict = ring().verify(&changed, Some(&fixture("release.new.asc")));
    assert!(verdict.refuses(), "{verdict:?}");
    assert!(matches!(&verdict, Verdict::Bad(why) if why.contains("new") && why.contains("does not match")), "{verdict:?}");
}

#[test]
fn a_strangers_signature_is_not_the_teams_but_is_not_refused() {
    let verdict = check(&ring(), "release.stranger.asc");
    assert_eq!(verdict, Verdict::UnknownSigner { key: fingerprint("stranger").to_string() });
    assert!(!verdict.refuses() && !verdict.is_valid());
    // A key taken out of the list — leaked, say — is a stranger from then on.
    let without_old = Keyring::from_keys(&[key("new", KeyStatus::Active)]).unwrap();
    assert!(matches!(check(&without_old, "release.old.asc"), Verdict::UnknownSigner { .. }));
}

#[test]
fn one_good_team_signature_among_several_is_enough() {
    // A stranger's, then the team's, as two armored blocks.
    let Verdict::Valid(signer) = check(&ring(), "release.stranger+new.asc") else { panic!("the second is the team's") };
    assert_eq!(signer.name, "new");
    // The old key's and the new key's, binary, one after the other: re-signed.
    assert!(check(&ring(), "release.old+new.sig").is_valid());
    let only_new = Keyring::from_keys(&[key("new", KeyStatus::Active)]).unwrap();
    let Verdict::Valid(signer) = check(&only_new, "release.old+new.sig") else { panic!("the new key's is there") };
    assert_eq!(signer.name, "new");
}

#[test]
fn an_rsa_key_works_too() {
    let ring = Keyring::from_keys(&[key("rsa", KeyStatus::Active)]).unwrap();
    assert!(check(&ring, "release.rsa.asc").is_valid());
}

#[test]
fn no_signature_is_unsigned_and_rubbish_is_refused() {
    let content = fixture("release.txt");
    assert_eq!(ring().verify(&content, None), Verdict::Unsigned);
    for rubbish in [&b""[..], b"not a signature", b"-----BEGIN PGP SIGNATURE-----\n\nnope\n-----END PGP SIGNATURE-----\n"] {
        let verdict = ring().verify(&content, Some(rubbish));
        assert!(verdict.refuses(), "{rubbish:?}: {verdict:?}");
    }
    // A public key is not a signature either.
    assert!(ring().verify(&content, Some(&fixture("key.new.asc"))).refuses());
}

#[test]
fn a_key_that_is_not_the_pinned_one_is_not_loaded() {
    let mut wrong = key("new", KeyStatus::Active);
    wrong.fingerprint = fingerprint("stranger");
    let err = Keyring::from_keys(&[wrong]).err().expect("the fingerprint is what is trusted");
    assert!(err.contains("not the pinned"), "{err}");
    let mut rubbish = key("new", KeyStatus::Active);
    rubbish.armored = "not a key";
    assert!(Keyring::from_keys(&[rubbish]).is_err());
}
