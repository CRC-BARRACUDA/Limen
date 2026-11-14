//! Which Barracuda platform a build talks to unless told otherwise.
//!
//! A debug build is a developer's: it talks to the platform running on their
//! own machine. A release build is what people install: it talks to
//! production. Either can be pointed elsewhere — `platform_url`,
//! `avatar_url` and `avatar_asset_base` in settings.json, or the
//! `LIMEN_PLATFORM_URL` / `LIMEN_AVATAR_URL` / `LIMEN_AVATAR_ASSET_BASE`
//! variables — for the dev deployment, say.

/// One deployment of the platform: where its services are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Environment {
    pub name: &'static str,
    /// Auth's API root: `/oauth/…` and `/me` are under it.
    pub auth_api: &'static str,
    /// The Avatar Decoration service's API root.
    pub avatar_api: &'static str,
    /// The files host avatars and frames are served from.
    pub asset_base: &'static str,
}

/// The platform on a developer's own machine.
pub const LOCAL: Environment = Environment {
    name: "local",
    auth_api: "https://auth.local.barracuda-team.tech:3443/api",
    avatar_api: "https://avatar.local.barracuda-team.tech:3443/api",
    asset_base: "https://files.local.barracuda-team.tech:3443/avatar-decorations/",
};

/// The shared dev deployment.
pub const DEV: Environment = Environment {
    name: "dev",
    auth_api: "https://auth-dev.barracuda-team.tech/api",
    avatar_api: "https://avatar-dev.barracuda-team.tech/api",
    asset_base: "https://files-dev.barracuda-team.tech/avatar-decorations/",
};

/// Production.
pub const PRODUCTION: Environment = Environment {
    name: "production",
    auth_api: "https://auth.barracuda-team.tech/api",
    avatar_api: "https://avatar.barracuda-team.tech/api",
    asset_base: "https://files.barracuda-team.tech/avatar-decorations/",
};

/// The environment a build of this kind talks to: local when built for
/// debugging, production when built for release.
pub const fn for_build(debug: bool) -> &'static Environment {
    if debug {
        &LOCAL
    } else {
        &PRODUCTION
    }
}

/// This build's.
pub fn current() -> &'static Environment {
    for_build(cfg!(debug_assertions))
}
