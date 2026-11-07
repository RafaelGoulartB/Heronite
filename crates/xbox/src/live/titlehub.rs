//! The titles the account played (`titlehub.xboxlive.com`'s title history)
//! and the minutes Xbox network counted for them (`userstats.xboxlive.com`).

use serde_json::{json, Value};

use crate::live::auth::Xsts;
use crate::live::http::{Http, Request};
use anyhow::{bail, Context, Result};

pub const TITLEHUB: &str = "https://titlehub.xboxlive.com";
pub const USERSTATS: &str = "https://userstats.xboxlive.com";

/// What the title history is asked to carry.
const DECORATIONS: &str = "detail,achievement,image,scid,productId";
/// At most this many titles per history request.
const MAX_ITEMS: u32 = 1000;
/// At most this many titles per `userstats` request.
const STATS_BATCH: usize = 100;

/// One title of the account's history, as much as Heronite uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Title {
    /// Decimal (`2046611937`): Xbox network's id of the title.
    pub title_id: String,
    /// Package family name of the PC version, when there is one.
    pub pfn: Option<String>,
    pub name: String,
    /// `Game`, `App`, …
    pub kind: String,
    /// `PC`, `XboxOne`, `XboxSeries`, `Xbox360`, …
    pub devices: Vec<String>,
    /// RFC 3339.
    pub last_played: Option<String>,
    pub achievements: Option<Achievements>,
    /// The title's tile (a public image address).
    pub image: Option<String>,
    /// The Store product id (`9NKX70BBCDRN`).
    pub product_id: Option<String>,
    pub publisher: Option<String>,
    pub developer: Option<String>,
    pub release_date: Option<String>,
    pub description: Option<String>,
    /// Minutes Xbox network counted (`userstats`), filled in later.
    pub minutes: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Achievements {
    pub unlocked: u32,
    pub total: u32,
    pub gamerscore: u32,
    pub total_gamerscore: u32,
}

impl Title {
    /// A game with a PC version.
    pub fn is_pc_game(&self) -> bool {
        self.kind.eq_ignore_ascii_case("Game") && self.devices.iter().any(|d| d.eq_ignore_ascii_case("PC"))
    }

    /// The title id as the game's files write it (`79FC11E1`).
    pub fn title_id_hex(&self) -> Option<String> {
        self.title_id.parse::<u32>().ok().filter(|v| *v != 0).map(|v| format!("{v:08X}"))
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn number(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(|n| n.as_u64().or_else(|| n.as_str().and_then(|s| s.parse().ok()))).and_then(|n| u32::try_from(n).ok())
}

/// `{"titles": [...]}` of the title history or of a batch request.
pub fn parse_titles(doc: &Value) -> Vec<Title> {
    let Some(items) = doc.get("titles").and_then(Value::as_array) else { return Vec::new() };
    items.iter().filter_map(parse_title).collect()
}

fn parse_title(t: &Value) -> Option<Title> {
    let title_id = t.get("titleId").and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string())))?;
    let title_id = title_id.trim().to_string();
    if title_id.is_empty() || title_id.parse::<u64>().is_err() {
        return None;
    }
    let detail = t.get("detail").filter(|d| d.is_object());
    let achievements = t.get("achievement").filter(|a| a.is_object()).map(|a| Achievements {
        unlocked: number(a, "currentAchievements").unwrap_or(0),
        total: number(a, "totalAchievements").unwrap_or(0),
        gamerscore: number(a, "currentGamerscore").unwrap_or(0),
        total_gamerscore: number(a, "totalGamerscore").unwrap_or(0),
    });
    // The tile: `displayImage`, else the box art of the images.
    let image = text(t, "displayImage").or_else(|| {
        let images = t.get("images").and_then(Value::as_array)?;
        let of =
            |kind: &str| images.iter().find(|i| text(i, "type").is_some_and(|k| k.eq_ignore_ascii_case(kind))).and_then(|i| text(i, "url"));
        of("BoxArt").or_else(|| of("Poster")).or_else(|| of("Tile"))
    });
    let image = image.map(|u| if u.starts_with("//") { format!("https:{u}") } else { u }).filter(|u| u.starts_with("https://"));
    Some(Title {
        title_id,
        pfn: text(t, "pfn"),
        name: text(t, "name").unwrap_or_default(),
        kind: text(t, "type").unwrap_or_default(),
        devices: t.get("devices").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
        last_played: t.get("titleHistory").and_then(|h| text(h, "lastTimePlayed")),
        achievements: achievements.filter(|a| a.total > 0),
        image,
        product_id: text(t, "productId"),
        publisher: detail.and_then(|d| text(d, "publisherName")),
        developer: detail.and_then(|d| text(d, "developerName")),
        release_date: detail.and_then(|d| text(d, "releaseDate")),
        description: detail.and_then(|d| text(d, "shortDescription").or_else(|| text(d, "description"))),
        minutes: None,
    })
}

/// The headers of every Xbox network request.
fn headers(xsts: &Xsts) -> Vec<(&'static str, String)> {
    vec![("Authorization", xsts.header()), ("x-xbl-contract-version", "2".into()), ("Accept-Language", "en-US".into())]
}

fn ok_json(status: u16, body: &str, what: &str) -> Result<Value> {
    match status {
        200..=299 => serde_json::from_str(body).with_context(|| format!("{what}: unreadable answer")),
        401 | 403 => Err(crate::live::SignedOut.into()),
        429 => bail!("{what}: Xbox network asks to wait (HTTP 429)"),
        s => bail!("{what}: HTTP {s}"),
    }
}

/// The account's title history.
pub fn history(http: &dyn Http, xsts: &Xsts) -> Result<Vec<Title>> {
    let url = format!("{TITLEHUB}/users/xuid({})/titles/titlehistory/decoration/{DECORATIONS}?maxItems={MAX_ITEMS}", xsts.xuid);
    let (status, body) = http.send(Request::get(&url, headers(xsts)))?;
    Ok(parse_titles(&ok_json(status, &body, "title history")?))
}

/// The titles of packages the history does not have (never played), by
/// package family name.
pub fn by_package(http: &dyn Http, xsts: &Xsts, pfns: &[String]) -> Result<Vec<Title>> {
    if pfns.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!("{TITLEHUB}/titles/batch/decoration/{DECORATIONS}");
    let body = json!({"pfns": pfns, "windowsPhoneProductIds": []});
    let (status, text) = http.send(Request::post_json(&url, headers(xsts), body))?;
    if status == 404 {
        return Ok(Vec::new());
    }
    Ok(parse_titles(&ok_json(status, &text, "titles")?))
}

/// `{"statlistscollection": [{"stats": [{"titleid": "...", "value": "..."}]}]}`.
pub fn parse_minutes(doc: &Value) -> Vec<(String, u64)> {
    doc.get("statlistscollection")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|l| l.get("stats").and_then(Value::as_array).into_iter().flatten())
        .filter_map(|s| {
            let id = text(s, "titleid")?;
            let value = s.get("value").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|t| t.trim().parse().ok())))?;
            (value > 0).then_some((id, value))
        })
        .collect()
}

/// Writes into `titles` the minutes Xbox network counted for each.
pub fn add_minutes(http: &dyn Http, xsts: &Xsts, titles: &mut [Title]) -> Result<()> {
    let ids: Vec<String> = titles.iter().map(|t| t.title_id.clone()).collect();
    for chunk in ids.chunks(STATS_BATCH) {
        let body = json!({
            "arrangebyfield": "xuid",
            "xuids": [xsts.xuid],
            "stats": chunk.iter().map(|id| json!({"name": "MinutesPlayed", "titleid": id})).collect::<Vec<_>>(),
        });
        let (status, text) = http.send(Request::post_json(&format!("{USERSTATS}/batch"), headers(xsts), body))?;
        for (id, minutes) in parse_minutes(&ok_json(status, &text, "play time")?) {
            if let Some(t) = titles.iter_mut().find(|t| t.title_id == id) {
                t.minutes = Some(minutes);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::live::http::{Body, Fake};
    use chrono::{TimeZone, Utc};

    pub(crate) const HISTORY: &str = include_str!("fixtures/titlehistory.json");

    fn xsts() -> Xsts {
        Xsts {
            token: "xsts-token".into(),
            uhs: "uhs1".into(),
            xuid: "2533274800000001".into(),
            gamertag: "Lamplighter".into(),
            not_after: Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn reads_the_title_history() {
        let titles = parse_titles(&serde_json::from_str(HISTORY).unwrap());
        assert_eq!(titles.len(), 5, "the ones without a numeric id are left out");
        let sky = &titles[0];
        assert_eq!(sky.title_id, "1784935649");
        assert_eq!(sky.title_id_hex().as_deref(), Some("6A63F4E1"));
        assert_eq!(sky.pfn.as_deref(), Some("Fable.SkywardLantern_abc123"));
        assert!(sky.is_pc_game());
        assert_eq!(sky.last_played.as_deref(), Some("2026-11-05T21:14:03.1200000Z"));
        assert_eq!(sky.achievements, Some(Achievements { unlocked: 12, total: 40, gamerscore: 210, total_gamerscore: 1000 }));
        assert_eq!(sky.image.as_deref(), Some("https://store-images.s-microsoft.com/image/apps.1.skyward"));
        assert_eq!(sky.product_id.as_deref(), Some("9ABCDEF12345"));
        assert_eq!(sky.developer.as_deref(), Some("Fable Works | Lantern Team"));
        assert_eq!(sky.description.as_deref(), Some("Float a lantern across the sky."));

        let tide = &titles[1];
        assert_eq!(tide.pfn, None);
        assert_eq!(tide.achievements, None, "a title without achievements has none to show");
        assert_eq!(tide.image.as_deref(), Some("https://store-images.s-microsoft.com/image/apps.2.tidebox"), "box art, made https");
        assert_eq!(titles[2].image, None, "only https images");
        assert!(!titles[3].is_pc_game(), "console only");
        assert!(!titles[4].is_pc_game(), "an app");
    }

    #[test]
    fn asks_the_history_with_the_tokens() {
        let fake = Fake::new(&[(200, HISTORY)]);
        let titles = history(&fake, &xsts()).unwrap();
        assert_eq!(titles.len(), 5);
        let sent = fake.sent();
        assert_eq!(
            sent[0].url,
            "https://titlehub.xboxlive.com/users/xuid(2533274800000001)/titles/titlehistory/decoration/detail,achievement,image,scid,productId?maxItems=1000"
        );
        assert!(sent[0].headers.contains(&("Authorization", "XBL3.0 x=uhs1;xsts-token".into())));
        assert!(sent[0].headers.contains(&("x-xbl-contract-version", "2".into())));

        let refused = Fake::new(&[(401, "")]);
        assert!(history(&refused, &xsts()).unwrap_err().is::<crate::live::SignedOut>());
        let busy = Fake::new(&[(429, "")]);
        assert!(history(&busy, &xsts()).unwrap_err().to_string().contains("429"));
    }

    #[test]
    fn asks_titles_by_package() {
        let fake = Fake::new(&[(200, r#"{"titles": [{"titleId": "77", "pfn": "A_x", "name": "A", "type": "Game", "devices": ["PC"]}]}"#)]);
        let got = by_package(&fake, &xsts(), &["A_x".into()]).unwrap();
        assert_eq!(got[0].title_id, "77");
        assert_eq!(fake.sent()[0].body, Body::Json(json!({"pfns": ["A_x"], "windowsPhoneProductIds": []})));
        assert!(by_package(&Fake::new(&[]), &xsts(), &[]).unwrap().is_empty(), "nothing asked");
        assert!(by_package(&Fake::new(&[(404, "")]), &xsts(), &["B_x".into()]).unwrap().is_empty());
    }

    #[test]
    fn reads_the_minutes_in_batches() {
        let doc = json!({"statlistscollection": [{"arrangebyfield": "xuid", "xuid": "1", "stats": [
            {"groupproperties": {}, "xuid": "1", "scid": "s", "titleid": "1784935649", "name": "MinutesPlayed", "type": "Integer", "value": "754"},
            {"titleid": "448139855", "name": "MinutesPlayed", "value": 30},
            {"titleid": "1234567890", "name": "MinutesPlayed", "value": "0"},
            {"titleid": "1", "name": "MinutesPlayed"}
        ]}]});
        assert_eq!(parse_minutes(&doc), [("1784935649".to_string(), 754), ("448139855".to_string(), 30)]);
        assert!(parse_minutes(&json!({"statlistscollection": []})).is_empty());

        let mut titles: Vec<Title> = (0..150).map(|i| Title { title_id: i.to_string(), ..Default::default() }).collect();
        let fake = Fake::new(&[
            (200, r#"{"statlistscollection": [{"stats": [{"titleid": "3", "value": "61"}]}]}"#),
            (200, r#"{"statlistscollection": [{"stats": [{"titleid": "120", "value": "5"}]}]}"#),
        ]);
        add_minutes(&fake, &xsts(), &mut titles).unwrap();
        assert_eq!((titles[3].minutes, titles[120].minutes, titles[4].minutes), (Some(61), Some(5), None));
        let sent = fake.sent();
        assert_eq!(sent.len(), 2, "100 titles per request");
        let Body::Json(body) = &sent[0].body else { panic!() };
        assert_eq!(body["stats"].as_array().unwrap().len(), 100);
        assert_eq!(body["stats"][0], json!({"name": "MinutesPlayed", "titleid": "0"}));
        assert_eq!(body["xuids"], json!(["2533274800000001"]));
        assert_eq!(sent[0].url, "https://userstats.xboxlive.com/batch");
    }
}
