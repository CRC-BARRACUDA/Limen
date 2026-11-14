//! Avatar decorations: the UI kit's own test tables (`spec.test.ts`,
//! `asset.test.ts`), ported, so Limen draws exactly what the web draws and
//! refuses exactly what it refuses. Plus how a lookup answer becomes a look.

use limen_core::avatar::*;
use serde_json::{json, Value};

const VALID: &str = r##"{
  "schema": 1,
  "frame":  { "asset": "deco/neon-ring.webp", "animated": true },
  "effect": { "name": "neon", "color": ["#ffb347", "#ff6a00"], "intensity": 0.6 }
}"##;

fn mutate(f: impl FnOnce(&mut Value)) -> String {
    let mut m: Value = serde_json::from_str(VALID).unwrap();
    f(&mut m);
    m.to_string()
}

#[test]
fn accepts_the_backend_readme_example() {
    let s = parse_spec(VALID).unwrap();
    assert_eq!(s.effect.as_ref().unwrap().name, EffectName::Neon);
    assert_eq!(s.frame, Some(DecorationFrame { asset: "deco/neon-ring.webp".into(), animated: true }));
    assert_eq!(s.effect.unwrap().color, Some(vec!["#ffb347".into(), "#ff6a00".into()]));
}

#[test]
fn accepts_frame_only_and_effect_only() {
    assert!(parse_spec(&mutate(|m| { m.as_object_mut().unwrap().remove("effect"); })).is_ok());
    assert!(parse_spec(&mutate(|m| { m.as_object_mut().unwrap().remove("frame"); })).is_ok());
}

#[test]
fn accepts_every_effect_form_the_kit_accepts() {
    for raw in [
        r#"{"schema":1,"effect":{"name":"aurora"}}"#,
        r##"{"schema":1,"effect":{"name":"tracking","color":["#fff","#a0c4ff"],"intensity":0.4}}"##,
        r#"{"schema":1,"effect":{"name":"aurora","intensity":0}}"#,
        r#"{"schema":1,"effect":{"name":"aurora","intensity":1}}"#,
        r#"{"schema":1,"effect":{"name":"aurora","color":null}}"#,
        r#"{"schema":1,"effect":{"name":"glitch"}}"#,
        r##"{"schema":1,"effect":{"name":"neon","color":["#0ff","#f0f"],"intensity":0.8}}"##,
        r#"{"schema":1,"effect":{"name":"signal"}}"#,
        r##"{"schema":1,"effect":{"name":"signal","color":["#39ff14"],"intensity":0.5}}"##,
        r#"{"schema":1,"effect":{"name":"blobtrack"}}"#,
        r#"{"schema":1,"effect":{"name":"blobinvert"}}"#,
        r##"{"schema":1,"effect":{"name":"neonsplit","color":["#50dcff","#ff8a3c"]}}"##,
        r##"{"schema":1,"effect":{"name":"blobtrack","color":["#ff00aa","#00e5ff"],"intensity":0.7}}"##,
    ] {
        let s = parse_spec(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
        assert!(s.effect.is_some(), "{raw}");
    }
}

#[test]
fn refuses_what_the_kit_refuses_with_the_same_field() {
    let big = format!(r#"{{"schema":1,"x":"{}"}}"#, "a".repeat(MAX_BYTES));
    let cases: Vec<(&str, String, &str)> = vec![
        ("empty", "".into(), "spec"),
        ("not json", "{".into(), "spec"),
        ("trailing value", format!("{VALID}{{}}"), "spec"),
        ("wrong schema", mutate(|m| m["schema"] = json!(2)), "schema"),
        ("nothing to draw", r#"{"schema":1}"#.into(), "spec"),
        ("oversized", big, "spec"),
        ("particles retired", mutate(|m| m["particles"] = json!({"emitter": {"shape": "ring"}})), "spec"),
        ("interactive retired", mutate(|m| m["interactive"] = json!({"click": {"preset": "burst"}})), "spec"),
        ("unknown top-level field", mutate(|m| m["script"] = json!("alert(1)")), "spec"),
        ("absolute url", mutate(|m| m["frame"] = json!({"asset": "https://evil.example/x.png"})), "frame.asset"),
        ("protocol relative", mutate(|m| m["frame"] = json!({"asset": "//evil.example/x.png"})), "frame.asset"),
        ("javascript scheme", mutate(|m| m["frame"] = json!({"asset": "javascript:alert(1)//.png"})), "frame.asset"),
        ("traversal", mutate(|m| m["frame"] = json!({"asset": "deco/../../secret.png"})), "frame.asset"),
        ("svg", mutate(|m| m["frame"] = json!({"asset": "deco/ring.svg"})), "frame.asset"),
        ("query string", mutate(|m| m["frame"] = json!({"asset": "deco/ring.png?x=1"})), "frame.asset"),
        ("empty asset", mutate(|m| m["frame"] = json!({"asset": ""})), "frame.asset"),
        ("unknown frame field", mutate(|m| m["frame"] = json!({"asset": "deco/a.png", "onload": "x"})), "spec"),
        ("effect unknown name", r#"{"schema":1,"effect":{"name":"eval"}}"#.into(), "effect.name"),
        ("effect removed name", r#"{"schema":1,"effect":{"name":"snowfall"}}"#.into(), "effect.name"),
        ("effect empty name", r#"{"schema":1,"effect":{"name":""}}"#.into(), "effect.name"),
        ("effect name case", r#"{"schema":1,"effect":{"name":"Aurora"}}"#.into(), "effect.name"),
        ("effect name spelling", r#"{"schema":1,"effect":{"name":"blob-track"}}"#.into(), "effect.name"),
        ("effect unknown field", r#"{"schema":1,"effect":{"name":"aurora","src":"x"}}"#.into(), "spec"),
        ("effect css in colour", r#"{"schema":1,"effect":{"name":"aurora","color":["red;x:y"]}}"#.into(), "effect.color[0]"),
        ("effect named colour", r#"{"schema":1,"effect":{"name":"aurora","color":["red"]}}"#.into(), "effect.color[0]"),
        ("effect empty colours", r#"{"schema":1,"effect":{"name":"aurora","color":[]}}"#.into(), "effect.color"),
        (
            "effect too many colours",
            r##"{"schema":1,"effect":{"name":"aurora","color":["#000","#111","#222","#333","#444","#555","#666"]}}"##.into(),
            "effect.color",
        ),
        ("effect intensity high", r#"{"schema":1,"effect":{"name":"aurora","intensity":1.5}}"#.into(), "effect.intensity"),
        ("effect intensity negative", r#"{"schema":1,"effect":{"name":"aurora","intensity":-0.1}}"#.into(), "effect.intensity"),
        ("effect not an object", r#"{"schema":1,"effect":"aurora"}"#.into(), "spec"),
        ("string number", mutate(|m| m["effect"]["intensity"] = json!("0.5")), "spec"),
        ("fractional schema", mutate(|m| m["schema"] = json!(1.5)), "spec"),
        ("array at top level", "[]".into(), "spec"),
    ];
    for (name, raw, field) in cases {
        let err = parse_spec(&raw).expect_err(name);
        assert_eq!(err.field, field, "{name}: {err}");
    }
}

#[test]
fn never_echoes_input_and_never_panics() {
    let err = parse_spec(r#"{"schema":"<img src=x onerror=alert(1)>"}"#).unwrap_err();
    assert!(!err.reason.contains("onerror"), "{err}");
    assert!(validate_spec(&json!({"schema": 2, "frame": {"asset": "a.png", "animated": false}})).is_err());
    for v in [Value::Null, json!(0), json!("x"), json!([]), json!({"schema": 1, "frame": 5}), json!({"schema": 1, "particles": []})] {
        assert!(validate_spec(&v).is_err(), "{v}");
    }
}

const BASE: &str = "https://files.local.barracuda-team.tech:3443/avatar-decorations/";

#[test]
fn a_frame_resolves_only_under_the_files_host() {
    assert_eq!(
        resolve_decoration_asset("deco/neon-ring.webp", Some(BASE)).as_deref(),
        Some("https://files.local.barracuda-team.tech:3443/avatar-decorations/deco/neon-ring.webp")
    );
    assert_eq!(
        resolve_decoration_asset("a.png", Some(BASE.trim_end_matches('/'))).as_deref(),
        Some(format!("{BASE}a.png").as_str()),
        "a base without its trailing slash is still a directory"
    );
    for bad in [
        "https://evil/x.png",
        "//evil/x.png",
        "javascript:alert(1)//.png",
        "../x.png",
        "deco/../../x.png",
        "data:image/png;base64,iVBORw0KGgo=",
        "/x.png",
        "deco/x.png?q=1",
        "deco/x.png#f",
        "deco/x.svg",
        "\\\\evil\\x.png",
        " https://evil/x.png",
    ] {
        assert_eq!(resolve_decoration_asset(bad, Some(BASE)), None, "{bad}");
    }
    assert_eq!(resolve_decoration_asset("a.png", None), None);
    assert_eq!(resolve_decoration_asset("a.png", Some("not a url")), None);
    assert_eq!(resolve_decoration_asset("a.png", Some("javascript:alert(1)/")), None);
    assert_eq!(resolve_decoration_asset("a.png", Some("data:text/html,x/")), None);
    for ok in ["a.png", "x/y/z.gif", "a-b_c/d.e.webp"] {
        let url = resolve_decoration_asset(ok, Some(BASE)).unwrap();
        assert!(url.starts_with("https://files.local.barracuda-team.tech:3443/"), "{url}");
    }
}

#[test]
fn an_icon_is_exactly_the_files_host_plus_icon_name() {
    let key = format!("icon/{}.png", "a".repeat(64));
    assert_eq!(resolve_icon_url(&format!("{BASE}{key}"), Some(BASE)), Some(format!("{BASE}{key}")));
    assert_eq!(resolve_icon_url(&format!("{BASE}{key}"), Some(BASE.trim_end_matches('/'))), Some(format!("{BASE}{key}")));
    for bad in [
        format!("https://evil.example/avatar-decorations/{key}"),
        format!("{BASE}deco/x.png"),
        format!("{BASE}icon/../deco/x.png"),
        format!("{BASE}{key}?x=1"),
        format!("{BASE}icon/x.svg"),
        "javascript:alert(1)".into(),
        "data:image/png;base64,AAAA".into(),
    ] {
        assert_eq!(resolve_icon_url(&bad, Some(BASE)), None, "{bad}");
    }
    assert_eq!(resolve_icon_url(&format!("{BASE}icon/default"), Some(BASE)), Some(format!("{BASE}icon/default")));
    assert_eq!(resolve_icon_url(&format!("{BASE}icon/defaults"), Some(BASE)), None);
    assert_eq!(resolve_icon_url(&format!("{BASE}icon/other"), Some(BASE)), None);
    assert_eq!(resolve_icon_url(&format!("{BASE}deco/default"), Some(BASE)), None);
    assert_eq!(resolve_decoration_asset("icon/default", Some(BASE)), None);
    assert_eq!(resolve_icon_url(&format!("{BASE}{key}"), None), None);
    let jpg = format!("icon/{}.jpg", "d".repeat(64));
    assert_eq!(resolve_icon_url(&format!("{BASE}{jpg}"), Some(BASE)), Some(format!("{BASE}{jpg}")), "photos may be JPEG");
    assert_eq!(resolve_decoration_asset("deco/x.jpg", Some(BASE)), None, "a frame never is");
}

fn svc() -> AvatarService {
    AvatarService { api_url: "https://avatar.example/api".into(), asset_base: BASE.into() }
}

#[test]
fn a_lookup_answer_becomes_a_look() {
    let body = json!({
        "assignments": {"u1": "d1", "u2": "gone"},
        "decorations": {"d1": {"id": "d1", "spec": {"schema": 1, "effect": {"name": "glitch", "intensity": 0.5},
                                                     "frame": {"asset": "deco/ring.gif", "animated": true}}}},
        "avatars": {"u1": {"url": format!("{BASE}icon/{}.webp", "b".repeat(64)), "animated": true}},
        "default_avatar": {"url": format!("{BASE}icon/{}.jpg", "c".repeat(64)), "animated": false},
    });
    let u1 = look_from(&body, "u1", &svc());
    assert_eq!(u1.spec.as_ref().unwrap().effect.as_ref().unwrap().name, EffectName::Glitch);
    assert_eq!(u1.frame, Some(Picture { url: format!("{BASE}deco/ring.gif"), animated: true }));
    assert!(u1.icon.as_ref().unwrap().url.ends_with(".webp") && u1.icon.as_ref().unwrap().animated, "their own icon");

    let u2 = look_from(&body, "u2", &svc());
    assert_eq!(u2.spec, None, "an assignment to nothing is a plain avatar");
    assert!(u2.icon.unwrap().url.ends_with(".jpg"), "no icon of their own: the default");

    let evil = json!({
        "assignments": {"u": "d"},
        "decorations": {"d": {"spec": {"schema": 1, "frame": {"asset": "https://evil/x.png"}}}},
        "avatars": {"u": {"url": "https://evil.example/me.png"}},
        "default_avatar": null,
    });
    let u = look_from(&evil, "u", &svc());
    assert_eq!((u.spec, u.icon, u.frame), (None, None, None), "nothing off the files host, nothing invalid");
}
