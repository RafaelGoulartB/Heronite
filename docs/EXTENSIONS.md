# Writing a metadata extension

An extension adds a metadata source to Heronite: a site or a database that
gives games their description, dates, genres, covers or other facts. It is
listed in Settings → Metadata like the built-in sources, can be switched on
and off, ordered, configured, searched and pinned like them, and is asked
by the same fetches, updates and reviews.

An extension is a folder with two text files, shared as a zip of that
folder:

```text
mysource/
  source.toml    the manifest: what it is, what it fills, where it connects
  source.js      the script: how it finds a game and reads its record
  README.md      optional: what it does, for people
  icon.png       optional (or icon.svg): its icon
```

Nothing else may be in the package: no other files, no folders, no links.
Limits: `source.toml` 64 KB, `source.js` 512 KB, `README.md` 64 KB, the
icon 256 KB, the zip 2 MB. In a zip the files sit at the top or in one
folder at the top. Everything the app shows about an extension comes from
the manifest; no script runs to draw a screen.

Metadata extensions live under `extensions/metadata/` (in the repository
and in the data folder) because other kinds of extensions may come later,
each in a folder of its own.

The repository has two complete examples: `extensions/metadata/wikipedia`
(a details source asked as a fallback) and `extensions/metadata/vndb` (the
Visual Novel Database: a details source asked as a fallback for visual
novels,
with settings, names in several scripts, pictures flagged adult one by
one, and a service that asks clients to slow down).

## The manifest: `source.toml`

```toml
id = "exampledb"                 # lowercase letters, digits, single dashes; 2–40
name = "Example DB"              # shown as is; 40 characters at most
version = "1.2.0"                # semantic version
api = 1                          # the contract version this extension is written for
author = "Your name"
homepage = "https://example.org" # optional
kind = "details"                 # details | pictures | extras
adult = false                    # optional: a source about adult games
description = "Games from Example DB, the database of example games."   # the "about" line; 300 at most

[provides]
# The fields it fills, by the app's ids:
# description releaseDate developers publishers genres themes gameModes
# platforms series features criticScore ageRatings
# cover (the tall cover) coverSquare coverWide hero (the background)
fields = ["description", "releaseDate", "genres", "cover", "hero"]

[applies]                        # optional: every game, always asked
to = "all"                       # all | emulated | pc (not emulated) | steam (games with a Steam id)
# platforms = ["snes", "n64"]    # or: emulated games of these platforms
# stores = ["gog", "steam"]      # or: games of these stores
asked = "always"                 # always | fallback
# The kind of game it is about, for a fallback source:
# game_kind = { name = "Visual novels", signals = ["Visual Novel"] }

[network]
hosts = ["api.example.org", "*.cdn.example.org"]   # where the script may send requests
images = ["images.example.net"]                    # further hosts its pictures may come from
rate = "2/s"                                       # N/s or N/min; 1/s if left out; 10/s at most

[[settings]]                     # optional, one table per setting, 16 at most
key = "apiKey"                   # letters, digits and _, starting with a letter
label = "API key"
type = "secret"                  # text | secret | select | switch
required = true                  # the source is not asked while it is empty
help = "Get a key on your profile page."
link = "https://example.org/profile/api"

[[settings]]
key = "region"
label = "Region"
type = "select"
options = ["eu", { value = "us", label = "America" }]
default = "us"
```

- **id** must not be a built-in source's (`igdb`, `steam`, `store`,
  `lutris`, `playnite`, `launchbox`, `steamgriddb`, `libretro`,
  `webimages`, `howlongtobeat`, `steamtags`, `protondb`, `manual`, `keep`,
  `auto`). The
  user's settings for it are kept under this id: keep it for ever.
- **kind**: a *details* source takes part in deciding which game a game is
  (an unsure match of its sends the game to review). A *pictures* source
  provides pictures only (`cover`, `coverSquare`, `coverWide`, `hero`).
  An *extras* source fills facts beside the others; its match is used when
  it is sure, or the same game the details sources found, and it never
  sends a game to review.
- **platforms**: the app's platform ids (`nintendo_super_nes`,
  `sony_playstation2`), or the names and short names the app knows them
  by (`Super Nintendo Entertainment System`, `snes`, `ps2`); each is read
  as the app's id, which is what `game.platform` carries. A name the app
  does not know is left out with a line in the log; a list with none it
  knows is refused. An emulated game whose platform the app cannot tell
  matches `to = "emulated"` but no `platforms` list.
- **asked = "fallback"**: asked only when no other online details source
  found the game for sure, when the record or the game's Steam tags carry
  its `game_kind` signal, when the game was already matched with it, or
  when the user asks it by name. The VNDB example is one, for the games
  whose genres or Steam tags say "Visual Novel".
- An extension's fields are taken after the sources already in a field's
  order; the user can move it like any source. It never goes ahead of
  them by itself: unlike a built-in source such as the LaunchBox Games
  Database, which goes first for an emulated game it found surely, an
  extension has no such rule, and no database to download.
- **hosts**: a name (`api.example.org`), or `*.example.org` for its
  subdomains (not `example.org` itself: list both if needed). Requests are
  `https` only; write `http://old.example.org` to allow plain `http` to
  that host. No IP addresses, ports or paths.
- **adult = true**: every picture it gives counts as adult and is dropped
  unless the user allows adult covers.
- **name**, **author**, a setting's **label** and the other short texts are
  one line: no line breaks (only `description` may have them), no control
  characters and no invisible formatting characters (direction overrides,
  zero-width characters); a manifest with them is refused.
- Unknown keys are refused in `[network]` and for `api`; anywhere else they
  are left out with a line in the log, so a manifest for a later minor
  version of the contract still loads.

## The script: `source.js`

An ES module that exports functions the app calls. Each may be `async`;
the app waits for its promise.

```js
export async function find(game, ctx) { /* -> [hit, ...] */ }
export async function fetch(id, ctx) { /* -> record | null */ }
export async function test(ctx) { /* optional -> true | "what went wrong" */ }
```

### `find(game, ctx)` → hits

Called to find a game automatically (the app takes the best hit and reads
it with `fetch`) and for the search by name in the per-game dialog and the
review (then `game` holds only the typed `title`). Return the games it may
be, best first, at most 25 (more are left out):

```js
{
  id: "12345",              // its id in your source; passed back to fetch, saved when pinned
  title: "Portal 2",        // its name as your source has it
  year: 2011,               // optional
  platform: "PC",           // optional
  thumb: "https://images.example.net/12345.jpg", // optional, on your hosts
  adult: true,              // optional: the thumb is adult (see below)
  altTitle: "ポータル2",     // optional: another name of it (another script, an alias)
  confidence: 1,            // optional, 0..1
  matchedBy: { id: "steam" } // optional, see below
}
```

`altTitle` is shown beside the title in the search results. `adult: true`
flags the thumbnail as adult: it is not shown unless the user allows
adult covers (as with a picture of the record, below).

How sure a match is decides whether it is saved or sent to review:

- By title (the default): sure only when `title` or `altTitle` is the
  game's title once normalised the app's way (`ctx.normalize`), and
  `confidence`, when given, is at least 0.75. A script cannot claim more.
  The record `fetch` returns should carry one of those two names as its
  title; a different one makes the match a guess. A source that knows a
  game by many names (VNDB: titles in several scripts, aliases) puts the
  one that is the game's title, when it is not the title it shows, in
  `altTitle`.
  Several hits with the game's title are different games of one name
  ("Doom" of 1993 and of 2016): they are guesses, unless the game's record
  has a year and exactly one of them has that `year`.
- `matchedBy: { id: "steam" }`: you found it by one of `game.ids` (name
  the key you used). Sure, if the game has that id.
- `matchedBy: { hash: "md5" }`: by one of `game.hashes`. Sure, if the game
  has that hash.

The `game` it gets:

```js
{
  title: "Portal 2",
  platform: null,            // an emulated game's platform id ("nintendo_super_nes"), else null
  year: 2011,                // when the record already knows it, else null
  fileName: "Portal 2",      // see below; never a path; else null
  store: "steam",            // steam, epic, gog, amazon, xbox, emulator, ... or null
  ids: { steam: "620", igdb: "72", lutris: "portal-2" },  // only those it has
  hashes: {},                // ROM hashes by kind (crc32, md5, sha1), when the app has them (none yet)
  fields: null               // for a pictures source: the fields asked, else null
}
```

- `platform` is the app's id of an emulated game's platform: the one its
  entry names, else what its ROM's extension, its emulator or its folders
  (`…/PS2/…`) say, else the one its record names. `null` for a PC game
  and when nothing says it.
- `fileName` is, for an emulated game, the name of its ROM with the
  extension, as the dump is named (`"Super Metroid (Japan, USA)
  (En,Ja).sfc"`; for a game started from a boot file such as `EBOOT.BIN`,
  the folder that names it); for another game, the name of its install
  folder.

Nothing else about the user or their library is passed.

### `fetch(id, ctx)` → record

Return the game's record, or `null` when your source has no such game.
Every key is optional; what the manifest does not provide is left out.

```js
{
  title: "Portal 2",
  description: "Plain text; HTML tags are removed. 2000 characters at most.",
  releaseDate: "2011-04-18",   // YYYY, YYYY-MM or YYYY-MM-DD (ctx.date.parse helps)
  developers: ["Valve"], publishers: [], genres: [], themes: [],
  gameModes: [], platforms: [], features: [],  // 12 names each at most, 100 characters each
  series: "Portal",
  score: 95,                   // 0..100
  ageRatings: [{ system: "esrb", rating: "E10+", descriptors: ["Fantasy Violence"] }],
  websites: ["https://www.thinkwithportals.com"],
  images: [
    { kind: "cover", url: "https://images.example.net/c.jpg" },               // the tall cover
    { kind: "cover", shape: "square", url: "https://images.example.net/s.jpg" },
    { kind: "cover", shape: "wide", url: "https://images.example.net/w.jpg" },
    { kind: "background", url: "https://images.example.net/b.jpg", adult: true }
  ],
  extras: {}                   // reserved for a later version of the contract; ignored
}
```

Pictures must be `https` addresses (or `http` where the manifest allows it)
on the manifest's `hosts` or `images`; others are left out. The first
picture of each kind and shape is used. `adult: true` flags one picture.
The app keeps an extension's picture (and a hit's `thumb`) with the
fragment `#heronite-extension`, any fragment it had replaced: whenever it
downloads it, now or later, it reaches public addresses only and checks
every redirect (see below).

### `test(ctx)`

Optional: Settings shows a "Test" button when it exists. Check that the
source can be asked with the user's settings; return `true`, or text that
says what is wrong.

### `ctx`: what the script may use

```js
const r = await ctx.http.get("https://api.example.org/search", {
  query: { q: game.title },              // added to the address
  headers: { Authorization: "Bearer " + ctx.settings.get("apiKey") },
});
r.status;   // 200
r.url;      // the address after redirects
r.headers;  // { "content-type": "...", ... } (lowercase names)
r.text;     // the body as text
r.json();   // the body parsed as JSON

await ctx.http.post(url, { json: { ... } });        // or form: { ... }, or body: "..."

const doc = ctx.html.parse(r.text);    // a read-only HTML document
doc.text("h1.title");                  // text of the first match (spaces squashed), or null
doc.attr("img.cover", "src");          // an attribute of the first match, or null
doc.all("ul.genres li").map((li) => li.text());   // each match: text(), attr(name), all(sel), html()

ctx.settings.get("region");    // this extension's own settings only
ctx.date.parse("April 18, 2011");   // "2011-04-18"; also "2011-04", "18 Apr 2011", "April 2011"...; or null
ctx.normalize("Pokémon: Red!");     // the app's title key, to compare titles like the app does
ctx.language;                  // the interface language: "en", "pt-BR", "de"...
ctx.log("found 3 pages");      // a line in the app's log, tagged with the extension id
throw ctx.unreachable("busy"); // the service cannot be asked now: not a failure (see below)
```

`ctx.http` requests carry the app's user agent (`Heronite/<version>`; a
script cannot set `User-Agent`, `Host` and the like, nor a header name or
value with line breaks), go out at the manifest's rate, and follow
redirects only to the manifest's hosts. An HTTP error status is not an
exception: check `r.status`. A request that cannot reach the service
(offline, DNS, timeout, even one that used up the call's time) throws; if
the call ends with that error, the game's lookup counts as "unreachable",
not as a failure of the script. An error of the script's own after it
(it caught the request's error and threw another, or ran until the time
limit) is a failure. A request whose turn at the manifest's rate would
come after the call's time is not sent: the call ends as "unreachable"
and its turn is left for the next call, so a slow rate (`2/min`) makes
calls wait their turn, never failures.

When the service answers but cannot be asked now (it asks to slow down
with a 429, or has a server error: 503 and the like), throw
`ctx.unreachable(message)`: the call ends as "unreachable", exactly like
a request that got no answer. The game is not marked as found in no
source and is asked again later, a fetch where every source was
unreachable counts as offline, and it never counts towards "Stopped
after errors". `message` (optional; kept to one line of 300 characters)
is what the log and the "Test" button show. Only the error it returns,
thrown and not caught, counts this way: an error of the script's own,
even with the same words, is a failure. `r.headers["retry-after"]` says
how long the service asked to wait; a script has no timers, but a second
try waits its turn at the manifest's rate (the VNDB example tries once
more when the pause asked is no longer than that).

## Limits

Each call of an exported function (a `find`, a `fetch`, a `test`) may use:

| What | Limit |
| --- | --- |
| Time, requests and waits included | 10 s |
| Memory of the script's engine | 32 MB |
| Stack (recursion) | 1 MB |
| Requests (each redirect counts) | 12 |
| One answer's size | 2 MB |
| All the answers of the call | 6 MB |
| Time of one request | 8 s (and never more than the call has left) |
| Log lines | 20 (more are dropped) |
| Hits of `find` | 25 |
| Data returned | 100,000 values, 8 MB of text (keys included), 32 levels deep |
| A request's address | 8 KB |
| A request's headers, query and form together | 64 KB |
| HTML documents parsed (`ctx.html.parse`) | 8, 8 MB and 200,000 tags together; 4 MB each |
| An array-like's `length` for the methods of `Array.prototype` | 2^24 |

Going over a limit ends the call: the source answers "not found" for that
game, and the fetch goes on with the other sources. After 5 failures in a
row (errors and limits; not an unreachable network nor
`ctx.unreachable`) the extension is
stopped: Settings shows "Stopped after errors" with the last error, and it
is not asked until the user switches it on again or installs an update.
Messages, the log and the stored error show addresses without their query
and fragment, which may hold a key.

The time limit holds wherever the script is: at its module's top level
(when it is inspected, installed or loaded) and in each exported function.
Most code is stopped at the limit. Some work cannot be interrupted: a loop
inside one of the engine's built-in functions, or the host's own work on
what the script gave it (parsing a document nested tens of thousands of
levels deep, or a tag with a hundred thousand attributes, takes far longer
than the call). The app waits for such a call 2 s past its time limit at
most, then leaves its thread running: the extension is stopped at once,
Settings says that it ran past its time limit and could not be stopped,
and it is not asked until the user switches it on again, installs an
update, or restarts the app. That thread keeps a processor core busy, and
its memory (32 MB at most), until it ends by itself or the app quits. At
most one such thread per extension, and three in all, are left running:
past that, the extension (or, at three, every extension) is not started
again until the app restarts.

## What an extension can and cannot do

The script runs in QuickJS, an embedded JavaScript engine, in a sandbox of
its own:

- It can send `GET` and `POST` requests to the hosts its manifest lists,
  over `https` (or `http` only where listed), at the manifest's rate. It
  cannot reach any other host, an IP address, a port, a name that leads to
  your computer or local network, or an address with a user and password.
  A name is resolved once and the request goes to the address checked;
  a name with any address that is not public (this computer, a private or
  local network, link-local, unique-local, carrier-grade NAT, multicast,
  documentation, benchmarking and the other special-purpose ranges of
  IANA's IPv4 and IPv6 registries) is refused, and so is a proxy. The
  pictures and thumbnails it names are downloaded the same way, every
  redirect checked again. The installer shows the hosts before you
  install.
- It can read its own settings. It cannot read the app's settings, other
  sources' keys or settings, your library, your files, or anything about
  you beyond the game it is asked about (its title, platform, year, file
  name — never a path —, store, ids and hashes).
- It has no file system, no processes, no sockets, no timers, no `eval` or
  `Function` constructor, no `import` (static or dynamic), no `fetch` or
  `XMLHttpRequest`; only the standard JavaScript objects and `ctx`.
- Each lookup runs in a fresh engine that is thrown away afterwards: what a
  script changes (globals, prototypes) does not last, and two lookups never
  share anything.
- What it returns is checked like the built-in sources' answers: plain
  text, capped lengths and lists, real dates, pictures only from its hosts,
  and only the fields its manifest provides.
- It cannot decide by itself that a match is sure by title: the app
  compares the titles.

What it can still do, and what a careful user should know:

- It sends what it is given (the game's title, ids...) and what you enter
  in its settings, keys included, to its hosts: whoever runs those hosts
  sees them. A host written as `*.example.org` is every subdomain, which
  may belong to people other than the author on a shared service.
- The addresses of the pictures it names can carry data to its picture
  hosts (`images`) when the app downloads them, and a link in its
  `websites` when you open it.
- Its author can change what it does in an update; the installer says so
  when an update is made by someone else or connects to new hosts, and
  asks before your saved settings go to it. The author's name is the
  author's own word: nothing proves who wrote a package.
- A script that keeps the engine busy where it cannot be interrupted costs
  a processor core until the app restarts (see Limits); the app stops the
  extension and starts no more than a few such threads.

Extensions are installed only by you, from a file you choose or from a
list (below), after the installer has shown what the extension is, what
it fills, who it is for, where it connects, the settings it asks for,
whether it is about adult games, where it comes from, and the package's
SHA-256. An installed extension starts switched on.

## Installing, updating, removing

Installed extensions live in the data folder, in
`extensions/metadata/<id>/`: the package's files as they were and
`install.json` (when, from where, the package's SHA-256, the address of the
list it came from, if any, and the last error while stopped). The SHA-256
is the zip file's, or for a folder the digest of its files (each name, a
zero byte, its length as 8 little-endian bytes and its bytes, in name
order).

Installing a newer version replaces the files and keeps what the user set
(on or off, settings, order). The same version again, or an older one, asks
first. A package with the id of one installed (or removed with its
settings kept) whose author differs, or that connects to hosts the other
did not, is shown as such ("Made by someone other than the one installed",
"Connects to new hosts: …"), and the installer asks whether what the user
entered in its settings (keys included) goes to it: not by default. The
app installs the package the installer showed, by its SHA-256: a file
that changed in between is refused. Removing an extension keeps what the
user set for it, and who made it and where it connected, for a later
install, unless they choose to forget it.

In the app, Settings → Metadata lists an installed extension with the
built-in sources ("Extension · 1.2.0" beside its name). Its open row says
who made it, where it came from, where it connects, and has "Update from a
file…" and "Remove". Under the sources, "Get more sources" opens the
gallery of the lists, and "Install from a file…" takes a `.zip` or the
`source.toml` of an extension's folder.

## Lists of extensions

The gallery ("Get more sources") reads lists: small JSON documents that
name extensions and where their two files are. Heronite always reads its
own list, `extensions/metadata/index.json` on the repository's main
branch; you can add others by their address ("Lists…"), and remove them
(what you installed from a list stays installed). The last good copy of
each list is kept in `store_cache/extensions/metadata/`, so the gallery
opens offline with it and says how old it is.

A list can also be a file on this computer: the path of an `index.json`
(or a `file://` address). It is read as strictly as a downloaded one, and
each entry's files are read from the folder beside it,
`<the list's folder>/<id>/source.toml` and `source.js`, ignoring their
`url`s; their SHA-256 and sizes are checked all the same, and nothing
outside that folder is read. No copy of it is kept: it is read each time.
What is installed from it is shown as "From a list on this computer",
never as from the Heronite list, and is offered updates only by that same
list (the Heronite list does not update it, even once it lists the same
extension: remove it and install it from there).

Installing from a list downloads the entry's `source.toml` and
`source.js`, checks that the SHA-256 and the size of each are the ones the
list gives, then shows the same installer as a file. The manifest must be
what the entry says: the same id, version and kind, no field the entry
does not list, no host the entry does not name (a picture host may be in
`hosts` or `images`), and adult only if the entry says so; otherwise
nothing is installed. Nothing updates by itself: an installed extension
whose list offers a newer version is flagged in the gallery ("Updates")
and in its row, and you install the update like any other.

What this checks, and what it does not: the files are the ones the list
names. Nobody reviews an extension because it is on a list; Heronite's own
list holds what was merged into the repository by a pull request, and a
list you add is kept by someone Heronite does not know. An extension from
a file was checked by nobody.

### The format

```json
{
  "index": 1,
  "type": "metadata",
  "name": "Heronite",
  "extensions": [
    {
      "id": "wikipedia",
      "name": "Wikipedia",
      "version": "1.0.0",
      "api": 1,
      "author": "Heronite",
      "homepage": "https://www.wikipedia.org",
      "kind": "details",
      "adult": false,
      "description": "The first paragraphs of the game's Wikipedia article, in your language when there is one.",
      "fields": ["description", "releaseDate"],
      "applies": { "to": "all", "asked": "fallback" },
      "hosts": ["*.wikipedia.org"],
      "images": [],
      "settings": [{ "label": "API key", "required": true }],
      "files": {
        "source.toml": {
          "url": "https://raw.githubusercontent.com/RafaelGoulartB/Heronite/main/extensions/metadata/wikipedia/source.toml",
          "sha256": "5d12d1646140a78c7e9614f028aa98178db81d45df879e3f0e26248841e71c54",
          "size": 659
        },
        "source.js": {
          "url": "https://raw.githubusercontent.com/RafaelGoulartB/Heronite/main/extensions/metadata/wikipedia/source.js",
          "sha256": "382556b0935ac6b7c20478b4fe0f1d55c4a039cc331f659d5c4482e3b1c4218e",
          "size": 3199
        }
      }
    }
  ]
}
```

- `index` is the version of the format; a list of a later one is refused
  ("made for a newer Heronite"). `type` says what kind of extensions the
  list holds; only `"metadata"` exists today, and a list of another type
  is refused ("This list is not a list of metadata sources."). It is
  optional, so a list written by hand without it is read as a list of
  metadata extensions; the script writes it. `name` is optional.
- Each entry copies its manifest: `id`, `name`, `version`, `api`, `author`,
  `homepage` (optional), `kind`, `adult` (optional), `description`,
  `fields` (`[provides] fields`), `applies` (the `[applies]` table, as is;
  optional), `hosts` and `images` (`[network]`), `settings` (each
  setting's `label` and `required`; optional). An entry whose `api` is
  higher than the app's is listed but cannot be installed.
- `files` gives `source.toml` and `source.js`: an `https` address, the
  SHA-256 in lowercase hex, and the size in bytes (64 KB and 512 KB at
  most).
- The list is read strictly: every address must be `https`, ids must be
  valid and not a built-in source's, an id appears once, at most 500
  entries and 1 MB. One wrong entry refuses the whole list, with what is
  wrong. Keys the app does not know are left out.

### Getting listed

Open a pull request on the repository that adds your extension's folder
under `extensions/metadata/` (`source.toml`, `source.js`, a `README.md`;
the folder named after the id) and regenerates the list:

```sh
python3 scripts/extensions-index.py           # rewrites extensions/metadata/index.json
python3 scripts/extensions-index.py --check   # what the tests check
```

The script reads each folder's `source.toml`, hashes the two files and
builds their addresses from a base (`--base`, the raw files of the main
branch by default). A test fails while `extensions/metadata/index.json` does not
match the folders. To publish a list of your own, run the script with
`--base <where your folders are served>` and `--out`, and serve the result
over `https`.

## Trying an extension

Run it against a game from the repository, with the real network and a
throw-away data folder:

```sh
cargo run -p heronite-metadata-ext --example run -- path/to/mysource inspect
cargo run -p heronite-metadata-ext --example run -- path/to/mysource find "Portal 2"
cargo run -p heronite-metadata-ext --example run -- path/to/mysource find "Portal 2" --steam 620 --language pt-BR
cargo run -p heronite-metadata-ext --example run -- path/to/mysource find "Super Metroid" --platform snes --file "Super Metroid (Japan, USA) (En,Ja).sfc"
cargo run -p heronite-metadata-ext --example run -- path/to/mysource search "Portal"
cargo run -p heronite-metadata-ext --example run -- path/to/mysource fetch 12345 --set apiKey=...
cargo run -p heronite-metadata-ext --example run -- path/to/mysource test
```

`find` prints the candidate the app would merge (the record after the
checks above, how the match was found). `--platform` (an id, name or short
name, read as the app's id) makes the game an emulated one, and `--file`
gives its `fileName`. Set `RUST_LOG=info` to see the
script's `ctx.log` lines and errors. The path may be the folder or its zip.

To try it in the app, through the gallery as users will install it: a
debug build (`cargo run -p heronite`) reads the repository's own
`extensions/metadata/index.json` as the app's list, so the folders of
`extensions/metadata/` install before they are merged (the gallery says
"From this copy of the repository"; a release build always reads the list
on GitHub). Add your
folder and run `python3 scripts/extensions-index.py` first, or the list
names other files and the install is refused. To read another list
instead, in any build, set `HERONITE_EXTENSIONS_INDEX` to its `https`
address or to the path of an `index.json` (one written with `--out`, its
folders beside it).

For tests in this repository, `crates/metadata-ext/tests/examples.rs` shows
how to serve recorded answers to an extension: the test HTTP client maps
an origin such as `https://en.wikipedia.org` to a local fixture server
(`Endpoints::origins`).

## Versions of the contract

`api` is the version of everything this page describes. A build reads
manifests of its version and older ones; a manifest with a higher `api` is
refused at install ("made for a newer Heronite") and, once installed, shows
as "Could not be loaded". Additions that old scripts do not notice (a new
`ctx` helper, a new optional key) do not change the number; a change that
would break a script does. `ctx.unreachable` and a hit's `altTitle` and
`adult` are such additions.

Version 1 is the first.
