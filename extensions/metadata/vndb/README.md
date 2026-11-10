# VNDB

A metadata source for Heronite that reads visual novels from VNDB, the
Visual Novel Database (vndb.org), through its public API
(<https://api.vndb.org/kana>). No key or account.

It is asked as a fallback: only when no other source found the game for
sure, when the game's record or its Steam tags say it is a visual novel,
when the game was already matched with VNDB, or when you ask it by name
(the search in a game's Metadata, "Review matches", a job of VNDB alone).

- Finds a game: VNDB's own search of the title, which knows each entry by
  its Japanese, romaji and English titles and its aliases. A name that is
  the game's title, in any of them, is a sure match; a close one is a
  guess, and an unsure match waits for review. In a search, a VNDB id
  (`v17`) or address (`https://vndb.org/v17`) finds that entry.
- Fills: description (VNDB's formatting and spoilers removed), release
  date (as precise as VNDB knows it: a year, a month or a day), developers,
  themes (the content tags VNDB's users rate 2 or more, without spoilers,
  12 at most), genre "Visual Novel", score (VNDB's rating), the tall cover,
  and a screenshot as the background.
- Adult pictures: VNDB's users vote on how suggestive or violent each
  picture is. A cover or screenshot that VNDB would not show as safe (an
  average vote above 0.4, or no vote) is flagged adult: Heronite drops it,
  and takes the picture from the next source, unless "Allow adult covers"
  is on. The background is the safe landscape screenshot with the most
  votes.
- Settings: "Title to show" (romaji / English, or the original title, for
  the name of the match; your game keeps its own) and "Use a screenshot as
  background" (on).
- Contacts: `api.vndb.org`, at most 36 requests a minute (VNDB allows 200
  per 5 minutes); pictures from `t.vndb.org`. When VNDB asks to slow down
  (429) or has a server error, the game is left for later: it is not
  marked as found in no source, and the extension is not stopped.
- Needs: nothing.

Try it:

```sh
cargo run -p heronite-metadata-ext --example run -- extensions/metadata/vndb find "Steins;Gate"
cargo run -p heronite-metadata-ext --example run -- extensions/metadata/vndb search "v2002"
cargo run -p heronite-metadata-ext --example run -- extensions/metadata/vndb fetch v97 --set titleOrder=original
```

VNDB's API is free for non-commercial use; its data is under VNDB's data
license (<https://vndb.org/d17#4>). Heronite is not affiliated with VNDB.
