// DLsite (dlsite.com), unofficially: DLsite has no public API for others,
// so this reads the JSON its own product pages load,
// `GET /maniax/api/=/product.json?workno=<code>`, which answers for a
// product of any of its sites (the site in the path does not matter) and
// gives `[]` for a code it does not have.
//
// A game is found only by its DLsite product code (RJ01234567, BJ123456,
// VJ012345...), which the app finds in the game's title or folder name by
// the manifest's `[ids] pattern` and passes as `game.ids.dlsite`. A product
// read by its code is the game for sure, whatever its title. There is no
// search by name: DLsite has none outside its HTML pages.
//
// Requests go out at the manifest's low rate. When DLsite refuses (403,
// 429), has a server error or answers with something other than JSON (a
// challenge page), the call ends as unreachable (`ctx.unreachable`): the
// game is asked again later, and the extension is not stopped for it.
//
// Adult pictures: DLsite rates each product (`age_category`: 1 all ages,
// 2 R-15, 3 adult only). Every picture of a product not rated for all ages
// (or whose rating is missing) is flagged adult, so Heronite drops it unless
// the user allows adult covers. Its genre tags are taken only for products
// rated for all ages.

const API = "https://www.dlsite.com/maniax/api/=/product.json";
const PAGE = "https://www.dlsite.com/";
// A product code as the manifest's pattern finds it.
const CODE = /^(?:RJ|RE|BJ|VJ)[0-9]{6,8}$/;
// DLsite's locales, by the app's interface language: genre names and the
// short description come in that language where DLsite has them.
const LOCALES = {
  en: "en_US", ja: "ja_JP", "zh-CN": "zh_CN", "zh-TW": "zh_TW", zh: "zh_CN", ko: "ko_KR", es: "es_ES",
  de: "de_DE", fr: "fr_FR", it: "it_IT", "pt-BR": "pt_BR", pt: "pt_BR", id: "id_ID", vi: "vi_VN", th: "th_TH", sv: "sv_SE",
};
const ALL_AGES = 1;

function locale(ctx) {
  const lang = String(ctx.language || "en");
  return LOCALES[lang] || LOCALES[lang.split("-")[0]] || "en_US";
}

function code(text) {
  const c = String(text || "").trim().toUpperCase();
  return CODE.test(c) ? c : null;
}

// The product of `workno`, or null when DLsite has none.
async function product(ctx, workno) {
  const r = await ctx.http.get(API, { query: { workno, locale: locale(ctx) } });
  if (r.status === 404) return null;
  if (r.status === 403 || r.status === 429 || r.status >= 500) throw ctx.unreachable("DLsite answered " + r.status);
  if (r.status !== 200) throw new Error("DLsite answered " + r.status);
  let rows;
  try {
    rows = r.json();
  } catch (e) {
    throw ctx.unreachable("DLsite answered with a page, not data");
  }
  if (!Array.isArray(rows)) throw ctx.unreachable("DLsite answered with something else than a product list");
  const p = rows.find((row) => row && code(row.workno) === workno);
  return p || null;
}

// `//img.dlsite.jp/...` as an https address; others are left out.
function https(url) {
  if (typeof url !== "string" || !url) return undefined;
  if (url.startsWith("//")) url = "https:" + url;
  return url.startsWith("https://") ? url : undefined;
}

function adult(p) {
  return p.age_category !== ALL_AGES;
}

function year(p) {
  const y = Number(String(p.regist_date || "").slice(0, 4));
  return y >= 1950 && y <= 2100 ? y : undefined;
}

function text(value) {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export async function find(game, ctx) {
  const workno = code(game.ids && game.ids.dlsite);
  if (!workno) return [];
  const p = await product(ctx, workno);
  if (!p || !text(p.work_name)) return [];
  return [
    {
      id: workno,
      title: p.work_name,
      year: year(p),
      thumb: https(p.image_thumb),
      adult: adult(p),
      matchedBy: { id: "dlsite" },
    },
  ];
}

export async function fetch(id, ctx) {
  const workno = code(id);
  if (!workno) return null;
  const p = await product(ctx, workno);
  if (!p || !text(p.work_name)) return null;
  const flagged = adult(p);
  const images = [];
  const main = https(p.image_main && p.image_main.url);
  // The main picture is DLsite's landscape cover (4:3 or so).
  if (main) images.push({ kind: "cover", shape: "wide", url: main, adult: flagged });
  const sample = (p.image_samples || []).find((s) => s && Number(s.width) > Number(s.height) && https(s.url));
  if (sample) images.push({ kind: "background", url: https(sample.url), adult: flagged });
  const date = /^\d{4}-\d{2}-\d{2}/.exec(String(p.regist_date || ""));
  const site = /^[a-z]+$/.test(p.site_id || "") ? p.site_id : "maniax";
  return {
    title: p.work_name,
    description: text(p.intro_s),
    releaseDate: date ? date[0] : undefined,
    developers: [text(p.maker_name)].filter(Boolean),
    genres: [text(p.work_type_string)].filter(Boolean),
    themes: flagged ? [] : (p.genres || []).map((g) => g && text(g.name)).filter(Boolean),
    series: text(p.series_name),
    websites: [PAGE + site + "/work/=/product_id/" + workno + ".html"],
    images,
  };
}

// DLsite answers a code it does not have with `[]`: reachable.
export async function test(ctx) {
  await product(ctx, "RJ00000001");
  return true;
}
