// VNDB, the Visual Novel Database, through its public API v2 ("Kana",
// https://api.vndb.org/kana): `POST /vn` with a filter and the fields to
// return, no key. A game is found by its title in any script and by its
// aliases (VNDB's own search), or by a VNDB id or address typed in a
// search; it is read by its id.
//
// VNDB allows 200 requests per 5 minutes; the manifest's rate keeps under
// it. A 429 (too many requests) or a server error ends the call as
// unreachable (`ctx.unreachable`): the game is asked again later, and the
// extension is not stopped for it.
//
// Pictures carry the flagging votes of VNDB's users (`sexual`, `violence`,
// 0..2 each). VNDB calls one safe while both averages are at most 0.4 and
// someone voted; any other is flagged adult, dropped unless the user
// allows adult covers.

const API = "https://api.vndb.org/kana/vn";

// The names of an entry, for matching and for a hit.
const HIT_FIELDS =
  "title, alttitle, titles{lang,title,latin}, aliases, released, platforms, image{url,thumbnail,sexual,violence,votecount}";
const RECORD_FIELDS =
  "title, alttitle, released, description, rating, image{url,sexual,violence,votecount}, " +
  "screenshots{url,dims,sexual,violence,votecount}, developers{name}, tags{name,rating,spoiler,lie,category}";
// Entries a title search reads, in VNDB's order (`searchrank`).
const RESULTS = 20;

// How close a name must be to the game's title to be a candidate
// (the app's own `MIN_MATCH_SCORE`).
const MIN_SCORE = 45;
// The highest average flagging vote VNDB still calls safe (above it,
// suggestive or violent; above 1.3 explicit or brutal).
const SAFE_MAX = 0.4;
// The least rating (0..3) a tag needs to be a theme: VNDB's users agree it
// is a clear part of the story.
const MIN_TAG_RATING = 2;

const PLATFORMS = {
  win: "Windows", lin: "Linux", mac: "Mac", ios: "iOS", and: "Android", web: "Web", dvd: "DVD player",
  ps1: "PlayStation", ps2: "PlayStation 2", ps3: "PlayStation 3", ps4: "PlayStation 4", ps5: "PlayStation 5",
  psp: "PSP", psv: "PS Vita", swi: "Switch", sw2: "Switch 2", nds: "Nintendo DS", n3d: "Nintendo 3DS",
  xb3: "Xbox 360", xbo: "Xbox One", xxs: "Xbox Series",
};

// One `POST /vn`. A 429 is tried once more when VNDB asks for a pause the
// rate already leaves (no `Retry-After`, or one of a second at most).
async function query(ctx, body) {
  for (let attempt = 0; ; attempt++) {
    const r = await ctx.http.post(API, { json: body });
    if (r.status === 200) return r.json().results || [];
    if (r.status === 429) {
      const wait = Number(r.headers["retry-after"]);
      if (attempt === 0 && !(wait > 1)) continue;
      throw ctx.unreachable("VNDB asks to slow down (429)");
    }
    if (r.status >= 500) throw ctx.unreachable("VNDB answered " + r.status);
    throw new Error("VNDB answered " + r.status + ": " + r.text.slice(0, 200));
  }
}

// The VNDB id a text names: `v17`, `V17` or a vndb.org address
// (`https://vndb.org/v17/chars`). A bare number is a title.
function parseId(text) {
  text = String(text || "").trim();
  const at = text.indexOf("vndb.org/");
  const path = at >= 0 ? text.slice(at + "vndb.org/".length).split(/[/?#]/)[0] : text;
  const m = /^[vV](\d+)$/.exec(path);
  const digits = m ? m[1].replace(/^0+/, "") : "";
  return digits && digits.length <= 12 ? "v" + digits : null;
}

// The title search queries of the app: the title, without an edition, with
// " - " as ": ", without a demo or beta marker.
function titleQueries(title) {
  const cleaned = String(title).replace(/_/g, " ").replace(/\s+/g, " ").trim();
  const noEdition = cleaned.replace(/\s*[-–:]\s*(definitive|ultimate|complete|goty|deluxe|gold|standard|windows|linux).*$/i, "");
  const colon = cleaned.replace(/\s+-\s+/g, ": ");
  const noMarker = noEdition
    .replace(
      /\s*[-–:]?\s*[([]?\s*\b(?:(?:multiplayer|open|closed|public|free)\s+)?(?:demo|beta|alpha|playtest|trial|preview|early\s+access)\b\s*[)\]]?\s*$/i,
      ""
    )
    .trim();
  return [...new Set([cleaned, noEdition, colon, noMarker])].filter((q) => [...q].length >= 2);
}

function key(ctx, text) {
  return ctx.normalize(String(text).slice(0, 500));
}

// The app's title score (0..100): 100 when the names are the same once
// normalised, less as they part.
function score(wanted, got) {
  if (!wanted || !got) return 0;
  if (wanted === got) return 100;
  if (got.startsWith(wanted) || wanted.startsWith(got)) return Math.max(60, 90 - Math.abs([...got].length - [...wanted].length));
  if (got.includes(wanted) || wanted.includes(got)) return 50;
  const parts = wanted.split(" ").filter(Boolean);
  const found = parts.filter((p) => got.includes(p)).length;
  return parts.length && found / parts.length >= 0.7 ? 40 : 0;
}

// Every name of an entry: its titles (original and romanised) and aliases.
function names(vn) {
  const titles = (vn.titles || []).flatMap((t) => [t.title, t.latin]);
  return [vn.title, vn.alttitle, ...titles, ...(vn.aliases || [])].filter((n) => typeof n === "string" && n);
}

// The best score of `title` against the names of `vn`, and that name.
function best(ctx, wanted, vn) {
  let top = { score: 0, name: null };
  for (const name of names(vn)) {
    const s = score(wanted, key(ctx, name));
    if (s > top.score) top = { score: s, name };
  }
  return top;
}

// The title shown and the other one, per the "Title to show" setting.
function shownTitles(vn, originalFirst) {
  const latin = vn.title || null;
  const original = vn.alttitle && vn.alttitle !== latin ? vn.alttitle : null;
  if (originalFirst && original) return [original, latin];
  return [latin || original, original];
}

function originalFirst(ctx) {
  return ctx.settings.get("titleOrder") === "original";
}

const isHttp = (url) => typeof url === "string" && /^https?:\/\//.test(url);

// VNDB's "safe and tame": voted on, and both averages low.
function safe(image) {
  return !!image && image.votecount > 0 && image.sexual <= SAFE_MAX && image.violence <= SAFE_MAX;
}

// `YYYY-MM-DD`, `YYYY-MM` or `YYYY` as VNDB gives it; nothing for `TBA`.
function releaseDate(released) {
  return /^\d{4}(-\d{2}(-\d{2})?)?$/.test(released || "") ? released : undefined;
}

// A hit of `vn`. Its other name is the one that is the game's title when
// that is not the name shown (an alias, another script), else the title in
// the other script: the app reads both when it compares titles.
function hit(ctx, vn, originalFirst, matched) {
  const [title, other] = shownTitles(vn, originalFirst);
  if (!title) return null;
  const h = { id: vn.id, title };
  const alt = matched && key(ctx, matched) !== key(ctx, title) ? matched : other;
  if (alt) h.altTitle = alt;
  const date = releaseDate(vn.released);
  if (date) h.year = Number(date.slice(0, 4));
  const platforms = (vn.platforms || []).slice(0, 3).map((p) => PLATFORMS[p] || String(p).toUpperCase());
  if (platforms.length) h.platform = platforms.join(", ");
  const image = vn.image;
  const thumb = image && (isHttp(image.thumbnail) ? image.thumbnail : image.url);
  if (isHttp(thumb)) {
    h.thumb = thumb;
    if (!safe(image)) h.adult = true;
  }
  return h;
}

// A search typed by the user: `game` holds the typed title and nothing
// else. An automatic lookup also carries the game's platform, file name,
// store and ids, even when they are empty.
function isSearch(game) {
  const empty = (v) => !v || Object.keys(v).length === 0;
  return Object.keys(game).every((k) => k === "title" || ((k === "ids" || k === "hashes") && empty(game[k])));
}

export async function find(game, ctx) {
  const first = originalFirst(ctx);
  // Only a search takes an id: a game titled "V4" is searched by its name.
  const id = isSearch(game) ? parseId(game.title) : null;
  if (id) {
    const rows = await query(ctx, { filters: ["id", "=", id], fields: HIT_FIELDS, results: 1 });
    return rows.map((vn) => hit(ctx, vn, first, null)).filter(Boolean);
  }
  const wanted = key(ctx, game.title);
  for (const q of titleQueries(game.title)) {
    const rows = await query(ctx, { filters: ["search", "=", q], fields: HIT_FIELDS, sort: "searchrank", results: RESULTS });
    const scored = rows
      .map((vn) => ({ vn, ...best(ctx, wanted, vn) }))
      .filter((s) => s.score >= MIN_SCORE)
      // Closest first; VNDB's order breaks ties.
      .sort((a, b) => b.score - a.score);
    if (scored.length) return scored.map((s) => hit(ctx, s.vn, first, s.score === 100 ? s.name : null)).filter(Boolean);
  }
  return [];
}

// VNDB's formatting codes as plain text: spoilers left out, links reduced to
// their text, the other codes dropped. The source note ("[From Steam]") stays.
function plainText(text) {
  return String(text || "")
    .replace(/\[spoiler\][\s\S]*?\[\/spoiler\]/gi, "")
    .replace(/\[url=[^\]]*\]([\s\S]*?)\[\/url\]/gi, "$1")
    .replace(/\[\/?(?:b|i|u|s|quote|raw|code)\]/gi, "")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

// The content tags that are not spoilers or lies, rated MIN_TAG_RATING or
// more, highest first, 12 at most.
function themes(tags) {
  return (tags || [])
    .filter((t) => t.name && t.spoiler === 0 && !t.lie && t.category === "cont" && t.rating >= MIN_TAG_RATING)
    .sort((a, b) => b.rating - a.rating)
    .slice(0, 12)
    .map((t) => t.name);
}

// The background: the safe landscape screenshot with the most flagging
// votes (the first on a tie); else the first landscape one, flagged adult.
function background(shots) {
  const landscape = (shots || []).filter((s) => s.dims && s.dims[0] > s.dims[1] && isHttp(s.url));
  let top = null;
  for (const s of landscape) {
    if (safe(s) && (!top || s.votecount > top.votecount)) top = s;
  }
  if (top) return { kind: "background", url: top.url };
  return landscape.length ? { kind: "background", url: landscape[0].url, adult: true } : null;
}

export async function fetch(id, ctx) {
  const vid = parseId(id);
  if (!vid) return null;
  const vn = (await query(ctx, { filters: ["id", "=", vid], fields: RECORD_FIELDS, results: 1 }))[0];
  if (!vn) return null;
  const images = [];
  if (vn.image && isHttp(vn.image.url)) {
    images.push(safe(vn.image) ? { kind: "cover", url: vn.image.url } : { kind: "cover", url: vn.image.url, adult: true });
  }
  if (ctx.settings.get("screenshotBackground") !== false) {
    const hero = background(vn.screenshots);
    if (hero) images.push(hero);
  }
  const rating = typeof vn.rating === "number" && vn.rating >= 1 && vn.rating <= 100 ? vn.rating : undefined;
  return {
    title: shownTitles(vn, originalFirst(ctx))[0],
    description: plainText(vn.description) || undefined,
    releaseDate: releaseDate(vn.released),
    developers: (vn.developers || []).map((d) => d.name).filter(Boolean),
    // Every VNDB entry is one: the Collection's genre, as IGDB names it.
    genres: ["Visual Novel"],
    themes: themes(vn.tags),
    score: rating,
    images,
  };
}

export async function test(ctx) {
  const rows = await query(ctx, { filters: ["id", "=", "v17"], fields: "title", results: 1 });
  return rows.length ? true : "VNDB did not find its own entry v17";
}
