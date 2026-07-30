//! The Limen module manager.
//!
//! Modules are distributed as **GitHub repositories** (feature #4). Given a ref
//! like `owner/repo@1.2.0`, the [`Registry`] fetches it, reads its `limen.toml`,
//! recursively resolves its `[dependencies]` (which may point at other repos or
//! local paths), installs everything into `~/.limen/modules/<name>/`, records a
//! content digest, and writes a [`Lockfile`] for reproducibility.
//!
//! Fetching is transport-pluggable ([`source`]): the default handles both
//! `git`-cloned repos and local paths, so the whole resolve/install/lock/remove
//! machinery is exercisable offline with path-based fixtures — no network, in
//! the same spirit as v1's hermetic test suite.
//!
//! Signature verification (minisign/cosign) is intentionally deferred to the
//! hardening phase; today we compute and lock a SHA-256 digest of every
//! installed module so tampering is at least detectable.

mod github;
mod lockfile;
mod registry;
mod source;
mod trust;
mod util;

pub use github::{
    fetch_remote_module, list_org_module_repos, list_org_modules,
    set_cache_dir as set_registry_cache_dir, set_token as set_github_token,
    test_token as test_github_token, RemoteModule, RepoCandidate,
};
pub use lockfile::{LockEntry, Lockfile};
pub use registry::{
    latest_release_version, set_update_modules_dir, InstallReport, Registry, VerifyItem,
    VerifyStatus,
};
pub use source::SourceSpec;
pub use trust::TrustStore;
pub use util::digest_dir;
