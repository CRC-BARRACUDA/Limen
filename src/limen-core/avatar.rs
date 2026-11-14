//! Avatars and their decorations, from the platform's Avatar Decoration service.
//!
//! A port of the UI kit's rules (`@barracuda/ui-kit`, `components/avatar`):
//! the same spec vocabulary and limits, the same validation with the same
//! error fields, and the same trust rules for the pictures. A decoration is
//! drawn from data only: a spec names a frame image and a code effect from a
//! fixed list, and is re-validated here however the service vouches for it.
//!
//! * The spec: `{schema: 1, frame?: {asset, animated}, effect?: {name, color?, intensity?}}`.
//! * A frame asset is a relative path, resolved against the files host Limen
//!   is configured with — never a URL from the spec.
//! * An icon (the user's picture) must be exactly the files host + `icon/…`.
//!
//! The lookup is the service's public read, `GET {api}/users/decorations?ids=…`,
//! which needs no token.

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::Config;

/// The Avatar Decoration service's API, until one is configured.
pub const DEFAULT_AVATAR_API: &str = "https://avatar.local.barracuda-team.tech:3443/api";
/// The files host the pictures live on, until one is configured.
pub const DEFAULT_ASSET_BASE: &str = "https://files.local.barracuda-team.tech:3443/avatar-decorations/";

/// The only schema version understood. Anything else draws a plain avatar.
pub const CURRENT_SCHEMA: i64 = 1;
/// A raw spec's size cap, in bytes.
pub const MAX_BYTES: usize = 16 << 10;
pub const MAX_COLORS: usize = 6;
pub const MAX_ASSET_LEN: usize = 200;
/// Effects are authored against a 64px avatar; their sizes scale from it.
pub const REFERENCE_SIZE: f32 = 64.0;
/// The most a picture download may be.
pub const MAX_PICTURE_BYTES: u64 = 8 << 20;

/// The code effects, by the names a spec uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectName {
    Aurora,
    Blobinvert,
    Blobtrack,
    Glitch,
    Neon,
    Neonsplit,
    Signal,
    Tracking,
}

impl EffectName {
    pub const ALL: [EffectName; 8] = [
        EffectName::Aurora,
        EffectName::Blobinvert,
        EffectName::Blobtrack,
        EffectName::Glitch,
        EffectName::Neon,
        EffectName::Neonsplit,
        EffectName::Signal,
        EffectName::Tracking,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EffectName::Aurora => "aurora",
            EffectName::Blobinvert => "blobinvert",
            EffectName::Blobtrack => "blobtrack",
            EffectName::Glitch => "glitch",
            EffectName::Neon => "neon",
            EffectName::Neonsplit => "neonsplit",
            EffectName::Signal => "signal",
            EffectName::Tracking => "tracking",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|n| n.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecorationFrame {
    pub asset: String,
    pub animated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecorationEffect {
    pub name: EffectName,
    /// The spec's palette; the effect has its own default.
    pub color: Option<Vec<String>>,
    /// 0..1, default 0.5.
    pub intensity: Option<f64>,
}

/// A validated decoration: built only from checked fields.
#[derive(Debug, Clone, PartialEq)]
pub struct DecorationSpec {
    pub frame: Option<DecorationFrame>,
    pub effect: Option<DecorationEffect>,
}

/// Why a spec was refused: the JSON path of the field, and the reason. The
/// same fields as the backend's, so an error points at the same form input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError {
    pub field: String,
    pub reason: String,
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

fn fail<T>(field: &str, reason: impl Into<String>) -> Result<T, SpecError> {
    Err(SpecError { field: field.into(), reason: reason.into() })
}

// --------------------------------------------------------------------------- //
// The spec: decoding as Go's encoding/json does (unknown fields refused, wrong
// types refused, null and missing both the zero value), then validation.
// --------------------------------------------------------------------------- //

fn wrong_type<T>(path: &str) -> Result<T, SpecError> {
    fail("spec", format!("field \"{path}\" has the wrong type"))
}

fn obj<'a>(
    v: Option<&'a Value>,
    path: &str,
    allowed: &[&str],
) -> Result<Option<&'a serde_json::Map<String, Value>>, SpecError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(m)) => {
            for k in m.keys() {
                if !allowed.contains(&k.as_str()) {
                    return fail("spec", format!("unknown field \"{k}\" is not allowed"));
                }
            }
            Ok(Some(m))
        }
        Some(_) => wrong_type(path),
    }
}

fn int(v: Option<&Value>, path: &str) -> Result<i64, SpecError> {
    match v {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Number(n)) => match n.as_f64() {
            // A number with nothing after the point is an integer, as in JS.
            Some(f) if f.is_finite() && f.fract() == 0.0 => Ok(f as i64),
            _ => wrong_type(path),
        },
        Some(_) => wrong_type(path),
    }
}

fn float(v: Option<&Value>, path: &str) -> Result<Option<f64>, SpecError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.is_finite() => Ok(Some(f)),
            _ => wrong_type(path),
        },
        Some(_) => wrong_type(path),
    }
}

fn string(v: Option<&Value>, path: &str) -> Result<String, SpecError> {
    match v {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => wrong_type(path),
    }
}

fn boolean(v: Option<&Value>, path: &str) -> Result<bool, SpecError> {
    match v {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => wrong_type(path),
    }
}

/// Validate the `spec` object from the API. Never panics, whatever it is.
pub fn validate_spec(v: &Value) -> Result<DecorationSpec, SpecError> {
    if matches!(v, Value::Array(_) | Value::Bool(_) | Value::Number(_) | Value::String(_)) {
        return fail("spec", "must be a JSON object");
    }
    let top = obj(Some(v), "", &["schema", "frame", "effect"])?;
    let get = |k: &str| top.and_then(|m| m.get(k));
    let schema = int(get("schema"), "schema")?;

    let frame = match obj(get("frame"), "frame", &["asset", "animated"])? {
        Some(f) => Some(DecorationFrame {
            asset: string(f.get("asset"), "frame.asset")?,
            animated: boolean(f.get("animated"), "frame.animated")?,
        }),
        None => None,
    };

    let effect = match obj(get("effect"), "effect", &["name", "color", "intensity"])? {
        Some(e) => {
            let name = string(e.get("name"), "effect.name")?;
            let color = match e.get("color") {
                None | Some(Value::Null) => None,
                Some(Value::Array(items)) => Some(
                    items
                        .iter()
                        .map(|x| string(Some(x), "effect.color"))
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                Some(_) => return wrong_type("effect.color"),
            };
            let intensity = float(e.get("intensity"), "effect.intensity")?;
            Some((name, color, intensity))
        }
        None => None,
    };

    if schema != CURRENT_SCHEMA {
        return fail("schema", format!("must be {CURRENT_SCHEMA}"));
    }
    if frame.is_none() && effect.is_none() {
        return fail("spec", "needs a frame, an effect, or both");
    }
    if let Some(f) = &frame {
        if f.asset.is_empty() {
            return fail("frame.asset", "is required");
        }
        if f.asset.chars().count() > MAX_ASSET_LEN {
            return fail("frame.asset", format!("exceeds {MAX_ASSET_LEN} characters"));
        }
        if f.asset.contains("..") || !asset_path_ok(&f.asset) {
            return fail("frame.asset", "must be a relative path to a .png, .apng, .webp or .gif file");
        }
    }
    let effect = match effect {
        None => None,
        Some((name, color, intensity)) => {
            let Some(name) = EffectName::parse(&name) else {
                let mut names: Vec<&str> = EffectName::ALL.iter().map(|n| n.as_str()).collect();
                names.sort();
                return fail("effect.name", format!("must be one of {}", names.join(", ")));
            };
            if let Some(colors) = &color {
                if colors.is_empty() || colors.len() > MAX_COLORS {
                    return fail("effect.color", format!("must list 1 to {MAX_COLORS} colours"));
                }
                for (i, c) in colors.iter().enumerate() {
                    if !hex_color_ok(c) {
                        return fail(&format!("effect.color[{i}]"), "must be a hex colour like #ff6a00");
                    }
                }
            }
            if let Some(i) = intensity
                && !(0.0..=1.0).contains(&i)
            {
                return fail("effect.intensity", "must be between 0 and 1");
            }
            Some(DecorationEffect { name, color, intensity })
        }
    };
    Ok(DecorationSpec { frame, effect })
}

/// Parse and validate raw JSON text.
pub fn parse_spec(text: &str) -> Result<DecorationSpec, SpecError> {
    if text.trim().is_empty() {
        return fail("spec", "is required");
    }
    if text.len() > MAX_BYTES {
        return fail("spec", format!("exceeds {MAX_BYTES} bytes"));
    }
    let v: Value = serde_json::from_str(text).or_else(|_| fail("spec", "is not valid JSON"))?;
    validate_spec(&v)
}

/// `#rgb` or `#rrggbb`.
pub fn hex_color_ok(s: &str) -> bool {
    let Some(h) = s.strip_prefix('#') else { return false };
    (h.len() == 3 || h.len() == 6) && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The kit's `ASSET_PATH`: `^[a-z0-9][a-z0-9_-]*(?:/[a-z0-9][a-z0-9_.-]*)*\.(?:png|apng|webp|gif)$`.
fn asset_path_ok(s: &str) -> bool {
    ["png", "apng", "webp", "gif"].iter().any(|ext| {
        let Some(rest) = s.strip_suffix(ext).and_then(|r| r.strip_suffix('.')) else {
            return false;
        };
        let mut segs = rest.split('/');
        let first_ok = |seg: &str, dots: bool| {
            let mut b = seg.bytes();
            matches!(b.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
                && b.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-' || (dots && c == b'.'))
        };
        matches!(segs.next(), Some(first) if first_ok(first, false)) && segs.all(|seg| first_ok(seg, true))
    })
}

/// The kit's `ICON_KEY`: `^icon/(?:[a-z0-9_-]+\.(?:png|apng|webp|gif|jpg)|default)$`.
fn icon_key_ok(key: &str) -> bool {
    let Some(name) = key.strip_prefix("icon/") else { return false };
    if name == "default" {
        return true;
    }
    let Some((stem, ext)) = name.rsplit_once('.') else { return false };
    ["png", "apng", "webp", "gif", "jpg"].contains(&ext)
        && !stem.is_empty()
        && stem.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

/// A relative key under the base: http(s), same origin, inside the base's
/// path, no query or fragment.
fn under_base(key: &str, base: Option<&str>) -> Option<String> {
    let mut base = url::Url::parse(base?).ok()?;
    if base.scheme() != "https" && base.scheme() != "http" {
        return None;
    }
    if !base.path().ends_with('/') {
        let p = format!("{}/", base.path());
        base.set_path(&p);
    }
    base.set_query(None);
    base.set_fragment(None);
    let url = base.join(key).ok()?;
    if url.origin() != base.origin() || !url.path().starts_with(base.path()) {
        return None;
    }
    if url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    Some(url.to_string())
}

/// A spec's `frame.asset` against the configured files host, or None (no frame).
pub fn resolve_decoration_asset(asset: &str, base: Option<&str>) -> Option<String> {
    if asset.chars().count() > MAX_ASSET_LEN || asset.contains("..") || !asset_path_ok(asset) {
        return None;
    }
    under_base(asset, base)
}

/// An icon URL from the read API, accepted only if it is exactly the files
/// host + `icon/<name>.<raster>` (or `icon/default`).
pub fn resolve_icon_url(url: &str, base: Option<&str>) -> Option<String> {
    let base = base?;
    let base = if base.ends_with('/') { base.to_string() } else { format!("{base}/") };
    let key = url.strip_prefix(&base)?;
    if !icon_key_ok(key) {
        return None;
    }
    under_base(key, Some(&base))
}

// --------------------------------------------------------------------------- //
// The service.
// --------------------------------------------------------------------------- //

/// Where the Avatar service and its files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarService {
    pub api_url: String,
    pub asset_base: String,
}

impl AvatarService {
    /// From the settings, `LIMEN_AVATAR_URL` / `LIMEN_AVATAR_ASSET_BASE` winning.
    pub fn from_config(cfg: &Config) -> Self {
        let env = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let pick = |e: &str, set: &Option<String>, default: &str| {
            env(e)
                .or_else(|| set.clone().filter(|v| !v.trim().is_empty()))
                .unwrap_or_else(|| default.to_string())
        };
        Self {
            api_url: pick("LIMEN_AVATAR_URL", &cfg.avatar_url, DEFAULT_AVATAR_API).trim_end_matches('/').to_string(),
            asset_base: pick("LIMEN_AVATAR_ASSET_BASE", &cfg.avatar_asset_base, DEFAULT_ASSET_BASE),
        }
    }
}

/// A picture to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub url: String,
    pub animated: bool,
}

/// How one user looks: their decoration (if any, and valid), their icon (if
/// any), and the frame the decoration names (if it resolves).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Look {
    pub spec: Option<DecorationSpec>,
    pub icon: Option<Picture>,
    pub frame: Option<Picture>,
}

/// Read a `/users/decorations` answer for `user_id`, as the kit's
/// DecorationProvider does: the assigned decoration, the user's own icon or
/// else the default one. Exposed for tests; [`look_up`] fetches and calls it.
pub fn look_from(body: &Value, user_id: &str, svc: &AvatarService) -> Look {
    let base = Some(svc.asset_base.as_str());
    let spec = body["assignments"][user_id]
        .as_str()
        .and_then(|deco| validate_spec(&body["decorations"][deco]["spec"]).ok());
    let picture = |v: &Value| -> Option<Picture> {
        let url = v.get("url")?.as_str()?;
        Some(Picture {
            url: resolve_icon_url(url, base)?,
            animated: v.get("animated").and_then(Value::as_bool).unwrap_or(false),
        })
    };
    let own = &body["avatars"][user_id];
    let icon = if own.get("url").and_then(Value::as_str).is_some() {
        picture(own)
    } else {
        picture(&body["default_avatar"])
    };
    let frame = spec.as_ref().and_then(|s| s.frame.as_ref()).and_then(|f| {
        Some(Picture { url: resolve_decoration_asset(&f.asset, base)?, animated: f.animated })
    });
    Look { spec, icon, frame }
}

/// How `user_id` looks. When the service cannot be asked, the decoration is
/// unknown (a plain avatar) and the icon is the files host's copy of the
/// default, which it serves even while the service is down.
pub fn look_up(svc: &AvatarService, user_id: &str) -> Look {
    match ask(svc, user_id) {
        Ok(body) => look_from(&body, user_id, svc),
        Err(_) => Look {
            spec: None,
            icon: resolve_icon_url(&format!("{}icon/default", with_slash(&svc.asset_base)), Some(&svc.asset_base))
                .map(|url| Picture { url, animated: false }),
            frame: None,
        },
    }
}

fn with_slash(s: &str) -> String {
    if s.ends_with('/') { s.to_string() } else { format!("{s}/") }
}

fn ask(svc: &AvatarService, user_id: &str) -> Result<Value> {
    let ids: String = url::form_urlencoded::byte_serialize(user_id.as_bytes()).collect();
    let (status, body) = crate::net::request(&format!("{}/users/decorations?ids={ids}", svc.api_url), None, None)?;
    if status != 200 {
        bail!("decoration lookup failed: HTTP {status}");
    }
    let v: Value = serde_json::from_str(&body).context("reading the decoration lookup")?;
    if !v.is_object() {
        bail!("decoration lookup returned a malformed body");
    }
    Ok(v)
}

/// A picture's bytes, capped at [`MAX_PICTURE_BYTES`].
pub fn fetch(picture: &Picture) -> Result<Vec<u8>> {
    crate::net::bytes(&picture.url, MAX_PICTURE_BYTES)
}
