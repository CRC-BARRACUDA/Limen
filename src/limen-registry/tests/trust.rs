//! Which module digests the user has vouched for.

use limen_registry::*;

#[test]
fn approval_is_pinned_to_digest() {
    let mut trust = TrustStore::default();
    assert!(!trust.is_trusted("crowdstrike", "sha256:aaa"));

    trust.approve("crowdstrike", "sha256:aaa");
    assert!(trust.is_trusted("crowdstrike", "sha256:aaa"));
    // A changed digest invalidates the approval (trust revoked on change).
    assert!(!trust.is_trusted("crowdstrike", "sha256:bbb"));

    assert!(trust.revoke("crowdstrike"));
    assert!(!trust.is_trusted("crowdstrike", "sha256:aaa"));
    assert!(!trust.revoke("crowdstrike"));
}
