# DLsite (unofficial)

A metadata source for Heronite that reads doujin and indie games from
DLsite (dlsite.com). **It is unofficial.** DLsite offers no API to other
apps, so this reads the same data DLsite's own product pages load. DLsite
may change or block it at any time. Heronite is not affiliated with DLsite.

**Maintained by:** lurebat (<https://github.com/lurebat>). Report problems
on Heronite's issue tracker and mention `@lurebat`. If it stays broken and
nobody looks after it, it may be dropped from the Heronite list.

It is asked as a fallback. That means it is asked only when:
- no other source found the game for sure;
- the game was already matched with DLsite;
- you ask it by name: the search in a game's Metadata, "Review matches", or
  a job of DLsite alone.

- **Finds a game:** only by its DLsite product code. A code is `RJ`, `RE`,
  `BJ` or `VJ` followed by 6 to 8 digits, for example `RJ01234567`.
  - The code must be in the game's title or the name of its folder.
  - Heronite finds it with the manifest's `[ids] pattern` and passes it to
    the script. A product read by its code is a sure match, even when its
    name is not the game's title.
  - A game with no code, or with two different codes, is not looked up.
  - To match a game by hand, type its code in the search.
- **Fills:**
  - description: DLsite's short introduction, in your language when DLsite
    has it;
  - release date;
  - developer: the circle or brand;
  - genre: DLsite's work type, such as "Role-playing" or "Adventure";
  - themes: DLsite's genre tags, only for products rated for all ages;
  - series;
  - the wide cover: DLsite's main picture;
  - the background: the first landscape sample picture.
  - Names and descriptions DLsite has not translated stay in the original
    (usually Japanese).
- **Adult pictures:** DLsite rates each product: all ages, R-15 or adult
  only.
  - Every picture of a product that is not rated for all ages is flagged
    adult. So is a picture of a product with no rating.
  - Heronite drops a flagged picture, and takes the picture from the next
    source, unless "Allow adult covers" is on.
  - The source as a whole is not marked `adult = true`, because the
    contract lets it flag each picture by its product's rating. That keeps
    the pictures of all-ages games.
- **Contacts:** `www.dlsite.com`, at most 20 requests a minute. A lookup is
  two requests. Pictures come from `img.dlsite.jp`.
  - When DLsite refuses (403, 429), has a server error, or answers with a
    page instead of data, the game is left for later. It is not marked as
    found in no source, and the extension is not stopped.
- **Needs:** nothing. No account and no key.

Try it:

```sh
cargo run -p heronite-metadata-ext --example run -- extensions/metadata/dlsite find "Drago Noka [RJ345679]"
cargo run -p heronite-metadata-ext --example run -- extensions/metadata/dlsite search "RJ345679"
cargo run -p heronite-metadata-ext --example run -- extensions/metadata/dlsite fetch RJ345679 --language ja
```

Product names, descriptions and pictures belong to DLsite and the works'
creators. Heronite keeps them only in your local library, as it does for
the other sources.
