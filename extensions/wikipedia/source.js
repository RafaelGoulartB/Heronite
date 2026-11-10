// Wikipedia: finds a game's article among the English Wikipedia's pages
// described as a video game, and gives the lead of the article in the
// interface's language (through the English article's language links),
// else in English. The release year comes from the article's short
// description ("2011 video game"); nothing finer is reliable without
// reading the infobox, so the date stays a year.

const EN = "https://en.wikipedia.org";

// "pt-BR" -> "pt", "zh-Hans" -> "zh".
function language(ctx) {
  return (ctx.language || "en").split(/[-_]/)[0].toLowerCase();
}

// "2011 video game" -> 2011.
function yearOf(description) {
  const m = /^(\d{4}) (?:.* )?video game$/i.exec(description || "");
  return m ? Number(m[1]) : undefined;
}

// "Portal (video game)" -> "Portal": the name of the game, not the page.
function shown(title) {
  return title.replace(/\s*\((?:\d{4} )?(?:video )?game\)$/i, "");
}

function isGame(page) {
  return /video game$/i.test(page.description || "");
}

export async function find(game, ctx) {
  const r = await ctx.http.get(EN + "/w/rest.php/v1/search/title", { query: { q: game.title, limit: 10 } });
  if (r.status !== 200) throw new Error("the search answered " + r.status);
  return (r.json().pages || []).filter(isGame).map((page) => {
    const year = yearOf(page.description);
    // Same name, other year: another game of that name.
    const otherYear = game.year && year && Math.abs(game.year - year) > 1;
    return { id: page.key, title: shown(page.title), year, confidence: otherYear ? 0.3 : 1 };
  });
}

async function summary(ctx, site, title) {
  const r = await ctx.http.get(site + "/api/rest_v1/page/summary/" + encodeURIComponent(title.replace(/ /g, "_")));
  if (r.status === 404) return null;
  if (r.status !== 200) throw new Error("the summary answered " + r.status);
  const page = r.json();
  return page.type === "standard" ? page : null;
}

// The title of the article in `lang`, from the English one's language links.
async function localTitle(ctx, id, lang) {
  const r = await ctx.http.get(EN + "/w/api.php", {
    query: { action: "query", titles: id, prop: "langlinks", lllang: lang, format: "json", formatversion: 2 },
  });
  if (r.status !== 200) return null;
  const page = ((r.json().query || {}).pages || [])[0] || {};
  return ((page.langlinks || [])[0] || {}).title || null;
}

export async function fetch(id, ctx) {
  const english = await summary(ctx, EN, id);
  if (!english || !isGame(english)) return null;
  let text = english.extract;
  const lang = language(ctx);
  if (lang !== "en") {
    const title = await localTitle(ctx, id, lang);
    const local = title ? await summary(ctx, `https://${lang}.wikipedia.org`, title) : null;
    if (local && local.extract) text = local.extract;
    else ctx.log(`no ${lang} article for ${id}, English instead`);
  }
  const year = yearOf(english.description);
  return {
    title: shown(english.title),
    description: text,
    releaseDate: year ? String(year) : undefined,
  };
}

export async function test(ctx) {
  const r = await ctx.http.get(EN + "/api/rest_v1/page/summary/Wikipedia");
  return r.status === 200 ? true : "Wikipedia answered " + r.status;
}
