// PCGamingWiki through its Cargo API (api.php?action=cargoquery): the
// Infobox_game table finds the game (by a Steam app id it lists, else by
// its page name), the Input and Video tables say what it supports.
// The engine the wiki names has no field in Heronite and is left out.

const API = "https://www.pcgamingwiki.com/w/api.php";

// Cargo's where clause is SQL: keep the title to what a name needs.
function quoted(text) {
  return '"' + String(text).replace(/["\\]/g, "").trim() + '"';
}

async function cargo(ctx, query) {
  const r = await ctx.http.get(API, { query: { action: "cargoquery", format: "json", ...query } });
  if (r.status !== 200) throw new Error("PCGamingWiki answered " + r.status);
  const body = r.json();
  if (body.error) throw new Error("PCGamingWiki: " + (body.error.info || body.error.code));
  return (body.cargoquery || []).map((row) => row.title || {});
}

// "2011-04-18;2011-04-19" or "April 18, 2011" -> 2011.
function yearOf(released) {
  const m = /\b(19|20)\d\d\b/.exec(released || "");
  return m ? Number(m[0]) : undefined;
}

function hit(row, matchedBy) {
  const h = { id: row.Page, title: row.Page, year: yearOf(row.Released) };
  if (matchedBy) h.matchedBy = matchedBy;
  return h;
}

export async function find(game, ctx) {
  const fields = "Infobox_game._pageName=Page,Infobox_game.Released=Released";
  if (game.ids.steam) {
    const rows = await cargo(ctx, {
      tables: "Infobox_game",
      fields,
      where: `Infobox_game.Steam_AppID HOLDS ${quoted(game.ids.steam)}`,
      limit: 5,
    });
    if (rows.length) return rows.map((row) => hit(row, { id: "steam" }));
  }
  const rows = await cargo(ctx, {
    tables: "Infobox_game",
    fields,
    where: `Infobox_game._pageName LIKE ${quoted("%" + game.title + "%")}`,
    limit: 10,
  });
  return rows.map((row) => hit(row));
}

// PCGamingWiki's states: true, limited, false, hackable, n/a, unknown...
const yes = (value) => value === "true";

export async function fetch(id, ctx) {
  const rows = await cargo(ctx, {
    tables: "Infobox_game,Input,Video",
    join_on: "Infobox_game._pageID=Input._pageID,Infobox_game._pageID=Video._pageID",
    fields: [
      "Infobox_game._pageName=Page",
      "Infobox_game.Series=Series",
      "Infobox_game.Released=Released",
      "Input.Controller_support=Controller",
      "Input.Full_controller_support=FullController",
      "Video.Ultrawidescreen=Ultrawide",
      "Video.HDR=HDR",
      "Video.Ray_tracing=RayTracing",
    ].join(","),
    where: `Infobox_game._pageName=${quoted(id)}`,
    limit: 1,
  });
  const row = rows[0];
  if (!row) return null;
  const features = [];
  if (yes(row.FullController)) features.push("Full controller support");
  else if (yes(row.Controller)) features.push("Controller support");
  else if (row.Controller === "limited") features.push("Partial controller support");
  if (yes(row.Ultrawide)) features.push("Ultra-widescreen");
  if (yes(row.HDR)) features.push("HDR");
  if (yes(row.RayTracing)) features.push("Ray tracing");
  ctx.log(`${id}: ${features.length} features`);
  return {
    title: row.Page,
    // A list on the wiki ("Portal,Half-Life"): the first is the game's own.
    series: (row.Series || "").split(",")[0].trim() || undefined,
    features,
  };
}
