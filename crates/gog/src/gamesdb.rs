//! GOG's game metadata in the shapes the app reads: the answers of the
//! `gamesdb` and `products` APIs become `GameInfo` JSON. Pure functions, no I/O.

use serde_json::{json, Map, Value};

/// Image URL of an image object: `url_format` has `{formatter}` and `{ext}` holes.
pub fn image_url(image: Option<&Value>, ext: &str) -> Option<String> {
    let format = image?.get("url_format")?.as_str()?;
    let url = format.replace("{formatter}", "").replace("{ext}", ext);
    Some(https(&url))
}

/// `//host/path` (protocol-relative) becomes an https address.
pub fn https(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    }
}

/// An id that GOG sends as a string or a number.
pub fn id_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A localized string object (`{"*": "text"}`) or a plain string.
fn localized(v: Option<&Value>) -> Option<String> {
    let text = match v? {
        Value::String(s) => s.as_str(),
        Value::Object(o) => o.get("*").and_then(Value::as_str).or_else(|| o.values().find_map(Value::as_str))?,
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn set(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(v) = value {
        map.insert(key.into(), Value::String(v));
    }
}

/// The skeleton every library entry shares.
fn base(app_name: &str, title: &str, folder_name: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("runner".into(), json!("gog"));
    m.insert("app_name".into(), json!(app_name));
    m.insert("title".into(), json!(title));
    m.insert("is_installed".into(), json!(false));
    m.insert("install".into(), json!({"is_dlc": false}));
    m.insert("folder_name".into(), json!(folder_name));
    m.insert("cloud_save_enabled".into(), json!(false));
    m.insert("canRunOffline".into(), json!(true));
    m
}

/// Entry for a `gamesdb` external release. `None` when it is not a game or a
/// mod, or is hidden from the library (DLCs are not listed on their own).
pub fn from_gamesdb(external_id: &str, doc: &Value, folder_name: &str) -> Option<Value> {
    if !matches!(doc.get("type").and_then(Value::as_str), Some("game" | "mod")) {
        return None;
    }
    let game = doc.get("game")?;
    if game.get("visible_in_library").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let title = localized(doc.get("title")).or_else(|| localized(game.get("title")))?;
    let mut m = base(external_id, &title, folder_name);

    let logo = image_url(game.get("logo"), "jpg");
    let background = image_url(game.get("background"), "webp");
    let cover = logo.or_else(|| background.clone());
    let square = image_url(game.get("vertical_cover"), "jpg").or_else(|| cover.clone());
    set(&mut m, "art_cover", cover);
    set(&mut m, "art_square", square);
    set(&mut m, "art_background", background);
    set(&mut m, "art_icon", image_url(game.get("square_icon"), "jpg").or_else(|| image_url(game.get("icon"), "jpg")));

    let developers: Vec<String> = game
        .get("developers")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|d| localized(d.get("name"))).collect())
        .unwrap_or_default();
    if !developers.is_empty() {
        m.insert("developer".into(), json!(developers.join(", ")));
    }

    let summary = localized(doc.get("summary"));
    let genres: Vec<String> = game
        .get("genres")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|g| localized(g.get("name"))).collect())
        .unwrap_or_default();
    let mut extra = Map::new();
    extra.insert("about".into(), json!({"description": summary.clone().unwrap_or_default()}));
    extra.insert("genres".into(), json!(genres));
    if let Some(date) = game.get("first_release_date").or_else(|| doc.get("first_release_date")).and_then(Value::as_str) {
        extra.insert("releaseDate".into(), json!(date));
    }
    m.insert("extra".into(), Value::Object(extra));
    m.insert("description".into(), json!(summary.unwrap_or_default()));

    let has_os = |slug: &str| {
        doc.get("supported_operating_systems")
            .and_then(Value::as_array)
            .is_some_and(|list| list.iter().any(|o| o.get("slug").and_then(Value::as_str) == Some(slug)))
    };
    m.insert("is_linux_native".into(), json!(has_os("linux")));
    m.insert("is_mac_native".into(), json!(has_os("osx")));

    let slug = game.get("slug").or_else(|| doc.get("slug")).and_then(Value::as_str).filter(|s| !s.is_empty());
    if let Some(slug) = slug {
        m.insert("store_url".into(), json!(format!("https://www.gog.com/game/{slug}")));
    }
    Some(Value::Object(m))
}

/// Entry from `api.gog.com/products/{id}?expand=description`, used when
/// `gamesdb` knows nothing of the game. `None` for DLCs and untitled products.
pub fn from_product(external_id: &str, doc: &Value, folder_name: &str) -> Option<Value> {
    if doc.get("game_type").and_then(Value::as_str).is_some_and(|t| t != "game") {
        return None;
    }
    let title = localized(doc.get("title"))?;
    let mut m = base(external_id, &title, folder_name);
    let images = doc.get("images");
    let image = |key: &str| images.and_then(|i| i.get(key)).and_then(Value::as_str).filter(|s| !s.is_empty()).map(https);
    let cover = image("logo2x").or_else(|| image("logo"));
    set(&mut m, "art_cover", cover.clone());
    set(&mut m, "art_square", cover);
    set(&mut m, "art_background", image("background"));
    set(&mut m, "art_icon", image("icon"));

    let lead = localized(doc.pointer("/description/lead"));
    m.insert("extra".into(), json!({"about": {"description": lead.clone().unwrap_or_default()}, "genres": []}));
    m.insert("description".into(), json!(lead.unwrap_or_default()));
    let compat = |os: &str| doc.pointer(&format!("/content_system_compatibility/{os}")).and_then(Value::as_bool).unwrap_or(false);
    m.insert("is_linux_native".into(), json!(compat("linux")));
    m.insert("is_mac_native".into(), json!(compat("osx")));
    if let Some(slug) = doc.get("slug").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        m.insert("store_url".into(), json!(format!("https://www.gog.com/game/{slug}")));
    }
    Some(Value::Object(m))
}

/// Shape of a `gamesdb` external release.
#[cfg(test)]
pub(crate) fn sample_release() -> Value {
    json!({
        "type": "game",
        "id": "gog_1207658924",
        "platform_id": "gog",
        "external_id": "1207658924",
        "title": {"*": "  The Witcher: Enhanced Edition  "},
        "summary": {"*": "A monster hunter takes on contracts."},
        "first_release_date": "2008-09-16T00:00:00+0000",
        "supported_operating_systems": [{"name": "Windows", "slug": "windows"}, {"name": "Linux", "slug": "linux"}],
        "game": {
            "id": "1207658924",
            "slug": "the_witcher",
            "title": {"*": "The Witcher"},
            "visible_in_library": true,
            "genres": [{"name": {"*": "Role-playing"}, "slug": "role-playing"}, {"name": {"*": "Fantasy"}, "slug": "fantasy"}],
            "developers": [{"name": "CD PROJEKT RED", "slug": "cdpr"}, {"name": "Atari", "slug": "atari"}],
            "first_release_date": "2007-10-26T00:00:00+0000",
            "logo": {"url_format": "https://images.gog.com/aaa{formatter}.{ext}", "formatters": ["800"]},
            "background": {"url_format": "https://images.gog.com/bbb{formatter}.{ext}"},
            "vertical_cover": {"url_format": "https://images.gog.com/ccc{formatter}.{ext}"},
            "square_icon": {"url_format": "https://images.gog.com/ddd{formatter}.{ext}"},
            "icon": {"url_format": "https://images.gog.com/eee{formatter}.{ext}"}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_formats_are_expanded() {
        let img = json!({"url_format": "https://images.gog.com/abc{formatter}.{ext}"});
        assert_eq!(image_url(Some(&img), "jpg").as_deref(), Some("https://images.gog.com/abc.jpg"));
        assert_eq!(image_url(Some(&img), "webp").as_deref(), Some("https://images.gog.com/abc.webp"));
        assert_eq!(image_url(None, "jpg"), None);
        assert_eq!(image_url(Some(&json!({})), "jpg"), None);
        assert_eq!(https("//images.gog.com/x.png"), "https://images.gog.com/x.png");
    }

    #[test]
    fn a_gamesdb_release_becomes_a_game_info() {
        let info = from_gamesdb("1207658924", &sample_release(), "The Witcher").unwrap();
        assert_eq!(info["runner"], "gog");
        assert_eq!(info["app_name"], "1207658924");
        assert_eq!(info["title"], "The Witcher: Enhanced Edition");
        assert_eq!(info["art_cover"], "https://images.gog.com/aaa.jpg");
        assert_eq!(info["art_square"], "https://images.gog.com/ccc.jpg");
        assert_eq!(info["art_background"], "https://images.gog.com/bbb.webp");
        assert_eq!(info["art_icon"], "https://images.gog.com/ddd.jpg");
        assert_eq!(info["developer"], "CD PROJEKT RED, Atari");
        assert_eq!(info["description"], "A monster hunter takes on contracts.");
        assert_eq!(info["extra"]["about"]["description"], "A monster hunter takes on contracts.");
        assert_eq!(info["extra"]["genres"], json!(["Role-playing", "Fantasy"]));
        assert_eq!(info["extra"]["releaseDate"], "2007-10-26T00:00:00+0000");
        assert_eq!((info["is_linux_native"].clone(), info["is_mac_native"].clone()), (json!(true), json!(false)));
        assert_eq!(info["store_url"], "https://www.gog.com/game/the_witcher");
        assert_eq!(info["folder_name"], "The Witcher");
        assert_eq!(info["install"], json!({"is_dlc": false}));
        assert_eq!(info["is_installed"], false);
        assert_eq!(info["cloud_save_enabled"], false);
        assert_eq!(info["canRunOffline"], true);
    }

    #[test]
    fn art_falls_back_and_a_missing_slug_has_no_store_url() {
        let mut doc = sample_release();
        let game = doc["game"].as_object_mut().unwrap();
        for k in ["logo", "vertical_cover", "square_icon", "slug"] {
            game.remove(k);
        }
        let info = from_gamesdb("1", &doc, "").unwrap();
        assert_eq!(info["art_cover"], "https://images.gog.com/bbb.webp");
        assert_eq!(info["art_square"], "https://images.gog.com/bbb.webp");
        assert_eq!(info["art_icon"], "https://images.gog.com/eee.jpg");
        assert!(info.get("store_url").is_none());
    }

    #[test]
    fn dlcs_hidden_and_other_types_are_dropped() {
        let mut dlc = sample_release();
        dlc["type"] = json!("dlc");
        assert!(from_gamesdb("1", &dlc, "").is_none());
        let mut hidden = sample_release();
        hidden["game"]["visible_in_library"] = json!(false);
        assert!(from_gamesdb("1", &hidden, "").is_none());
        let mut modded = sample_release();
        modded["type"] = json!("mod");
        assert!(from_gamesdb("1", &modded, "").is_some());
        assert!(from_gamesdb("1", &json!({}), "").is_none());
    }

    #[test]
    fn products_are_the_fallback() {
        let doc = json!({
            "id": 1207658924u64, "title": "Some Game", "slug": "some_game", "game_type": "game",
            "content_system_compatibility": {"windows": true, "osx": false, "linux": true},
            "images": {"logo": "//images.gog.com/l.png", "logo2x": "//images.gog.com/l2.png", "icon": "//images.gog.com/i.png"},
            "description": {"lead": "Short text"}
        });
        let info = from_product("1207658924", &doc, "").unwrap();
        assert_eq!(info["art_cover"], "https://images.gog.com/l2.png");
        assert_eq!(info["art_icon"], "https://images.gog.com/i.png");
        assert_eq!(info["is_linux_native"], true);
        assert_eq!(info["description"], "Short text");
        assert_eq!(info["store_url"], "https://www.gog.com/game/some_game");
        let mut dlc = doc.clone();
        dlc["game_type"] = json!("dlc");
        assert!(from_product("1", &dlc, "").is_none());
    }
}
