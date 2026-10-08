import json, sys

IMG = json.load(open(sys.argv[1]))
BG, BGB, COV = IMG["bg"], IMG["bgblur"], IMG["covers"]

# ---------------------------------------------------------------- icons
P = {
    "grid": '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
    "download": '<path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/>',
    "wine": '<path d="M8 22h8"/><path d="M12 15v7"/><path d="M6 3h12l-.6 5.5a5.4 5.4 0 0 1-10.8 0Z"/>',
    "user": '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>',
    "pad": '<rect x="2" y="6" width="20" height="12" rx="6"/><path d="M6.5 12h4M8.5 10v4"/><circle cx="15.5" cy="11" r=".9"/><circle cx="17.5" cy="13.2" r=".9"/>',
    "plus": '<path d="M12 5v14M5 12h14"/>',
    "gear": '<circle cx="12" cy="12" r="3.2"/><path d="M12 2.5v3M12 18.5v3M5.3 5.3l2.1 2.1M16.6 16.6l2.1 2.1M2.5 12h3M18.5 12h3M5.3 18.7l2.1-2.1M16.6 7.4l2.1-2.1"/>',
    "sliders": '<path d="M4 6h9M19 6h1M4 12h3M11 12h9M4 18h11M19 18h1"/><circle cx="16" cy="6" r="2.2"/><circle cx="9" cy="12" r="2.2"/><circle cx="17" cy="18" r="2.2"/>',
    "books": '<path d="M4 4h4v16H4zM10 4h4v16h-4z"/><path d="m16 5.2 3.8-1 2.6 14.6-3.8 1z"/>',
    "term": '<path d="m4 17 6-5-6-5"/><path d="M12 19h8"/>',
    "file": '<path d="M14 3H6a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8z"/><path d="M14 3v5h5M9 13h6M9 17h6"/>',
    "info": '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.6v.4"/>',
    "folder": '<path d="M3 7a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z"/>',
    "x": '<path d="M6 6l12 12M18 6 6 18"/>',
    "search": '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
    "image": '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="10" r="2"/><path d="m21 16-5-5-9 9"/>',
    "monitor": '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
    "cloud": '<path d="M7 18a5 5 0 1 1 1-9.9A6 6 0 0 1 19 10a4 4 0 0 1 0 8z"/>',
    "menu": '<path d="M4 7h16M4 12h16M4 17h16"/>',
    "chev": '<path d="m9 6 6 6-6 6"/>',
    "chevd": '<path d="m6 9 6 6 6-6"/>',
    "chevl": '<path d="m15 6-6 6 6 6"/>',
    "refresh": '<path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v7h-7"/>',
    "list": '<path d="M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01"/>',
    "box": '<path d="M21 8 12 3 3 8v8l9 5 9-5z"/><path d="m3 8 9 5 9-5M12 13v8"/>',
    "filter": '<path d="M3 5h18l-7 8v6l-4 2v-8z"/>',
    "sort": '<path d="M7 4v16M3 16l4 4 4-4M17 20V4M13 8l4-4 4 4"/>',
    "flag": '<path d="M5 21V4M5 4h11l-2 4 2 4H5"/>',
    "swap": '<path d="M4 8h14l-3-3M20 16H6l3 3"/>',
    "check": '<path d="m5 12 5 5 9-10"/>',
    "cmd": '<path d="M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3z"/>',
    "dot": '<circle cx="12" cy="12" r="4"/>',
    "clock": '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
    "play": '<path d="M7 4.5v15l12.5-7.5z" fill="currentColor"/>',
    "terminal": '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M12 15h5"/>',
}


def ic(name, size=18, sw=1.8, cls=""):
    return f'<svg class="ic" width="{size}" height="{size}"><use href="#i-{name}"/></svg>'


SPRITE = ('<svg width="0" height="0" style="position:absolute"><defs>' + "".join(
    f'<symbol id="i-{k}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" '
    f'stroke-linecap="round" stroke-linejoin="round">{v}</symbol>' for k, v in P.items()) + '</defs></svg>')


# ---------------------------------------------------------------- shared data
MENU = [("grid", "Collection"), ("download", "Downloads"), ("wine", "Wine Manager"),
        ("user", "Epic Games"), ("pad", "Emulators")]
SNAV = [("gear", "General"), ("sliders", "Game defaults"), ("books", "Library"), ("download", "Downloads"),
        ("term", "Advanced"), ("file", "Logs"), ("info", "About")]
GTABS = [("image", "Images"), ("list", "Details"), ("wine", "Wine"), ("box", "Other"), ("term", "Advanced"),
         ("cloud", "Cloud saves"), ("monitor", "Gamescope")]
TOGGLES_GENERAL = [
    ("Minimize after game launch", "", False),
    ("Exit to system tray", "Closing the window keeps Heronite running in the tray.", True),
    ("Start minimized", "", False),
    ("Hide tray icon", "Requires restart.", False),
    ("Hide the window for heronite-rs:// links", "Keeps the window hidden when a game starts from a shortcut.", False),
]
TOGGLES_SHORTCUTS = [
    ("Add desktop shortcuts automatically", "", False),
    ("Add games to the start menu automatically", "", True),
    ("Add games to Steam automatically", "", False),
]
WINE_TOGGLES = [
    ("DXVK", "Direct3D 9–11 over Vulkan", True),
    ("VKD3D-Proton", "Direct3D 12 over Vulkan", True),
    ("DXVK-NVAPI", "DLSS and Reflex on NVIDIA", False),
    ("Esync / Fsync", "Faster synchronization", True),
    ("Wine-Wayland driver", "Experimental", False),
]


def collection(dim=False, blur=False):
    return f'<div class="coll-slot" data-cls="{' dim' if dim else ''}{' blur' if blur else ''}"></div>'


def collection_real():
    covers = "".join(f'<div class="cv" style="background-image:url({c})"><span>{t}</span></div>'
                     for c, t in zip(COV, ["3h 20m", "16h 1m", "47h 15m", "70h 53m", "6h 16m", "26m", "5m", "9m", "2h 4m", "55m"]))
    return f'''
<div class="coll">
  <div class="cbg" style="background-image:url({BG})"></div>
  <div class="chdr">
    <div class="cl">{ic("menu",18)}<span class="sep"></span>{ic("pad",18)}</div>
    <div class="csearch">{ic("search",14)}<span>Search for Games</span>{ic("filter",14)}{ic("sort",14)}{ic("flag",14)}</div>
    <div class="cr">{ic("refresh",16)}{ic("swap",16)}{ic("gear",16)}</div>
  </div>
  <div class="clist">
    <div class="cgh"><i style="background:#3ecf8e"></i>2. Playing <b>2</b></div>
    <div class="cgrid">{covers[:0]}{"".join(covers.split("</div>")[i] + "</div>" for i in range(2))}</div>
    <div class="cgh"><i style="background:#2ec4b6"></i>3. On Going <b>4</b></div>
    <div class="cgrid">{"".join(covers.split("</div>")[i] + "</div>" for i in range(2, 6))}</div>
    <div class="cgh"><i style="background:#6cb6ff"></i>4. Plan to Play <b>1</b></div>
    <div class="cgrid">{"".join(covers.split("</div>")[i] + "</div>" for i in range(6, 9))}</div>
  </div>
  <div class="cfocus">
    <h1>Control: AWE</h1>
    <p class="cmeta">8/27/2020 · Google Stadia · Shooter · Adventure</p>
    <div class="cshots"><i></i><i></i><i></i><i></i></div>
    <div class="cdet"><div></div><div></div><div></div><div></div><div></div><div></div></div>
    <div class="cplay">{ic("play",14)} Play Now</div>
  </div>
</div>'''


def board(opt, title, inner, note=""):
    return f'''
<figure class="frame">
  <figcaption><b>{title}</b>{f" — {note}" if note else ""}</figcaption>
  <div class="scaler"><div class="board {opt}">{inner}</div></div>
</figure>'''


# ================================================================ OPTION 1 — Vidro
def o1_menu():
    meta = ["210 games", "1 active · 42%", "Proton-CachyOS 10", "Signed in", "6 emulators"]
    tiles = "".join(f'<div class="tile{" on" if i == 0 else ""}">{ic(n,22)}<b>{t}</b><small>{m}</small></div>'
                    for i, ((n, t), m) in enumerate(zip(MENU, meta)))
    return collection() + f'''
<div class="g-pop" style="left:10px;top:48px">
  <div class="tiles">{tiles}<div class="tile add">{ic("plus",22)}<b>Add game</b><small>Sideload or ROM</small></div></div>
  <div class="g-row">{ic("gear",17)}<span>Settings</span><kbd>Ctrl ,</kbd></div>
</div>'''


def o1_settings():
    nav = "".join(f'<div class="pill{" on" if i == 0 else ""}">{ic(n,16)}{t}</div>' for i, (n, t) in enumerate(SNAV))
    rows = lambda items: "".join(
        f'<div class="row"><div><b>{t}</b>{f"<small>{d}</small>" if d else ""}</div><span class="tg{" on" if v else ""}"><i></i></span></div>'
        for t, d, v in items)
    return collection(blur=True) + f'''
<div class="g-sheet">
  <div class="g-side"><div class="g-title">{ic("gear",18)} Settings</div>{nav}</div>
  <div class="g-main">
    <h2>General</h2>
    <p class="lead">Saved automatically to the shared Heroic settings.</p>
    <div class="card">
      <div class="fld"><label>Default install folder</label><div class="in">{ic("folder",15)}<span>/home/rafa/Games/Heroic</span><em>Change</em></div></div>
      <div class="fld"><label>New Wine prefixes</label><div class="in">{ic("folder",15)}<span>/home/rafa/Games/Heroic/Prefixes</span><em>Change</em></div></div>
    </div>
    <div class="cap">Window &amp; tray</div>
    <div class="card">{rows(TOGGLES_GENERAL[:3])}</div>
    <div class="cap">Shortcuts</div>
    <div class="card">{rows(TOGGLES_SHORTCUTS)}</div>
  </div>
  <div class="g-x">{ic("x",16)}</div>
</div>'''


def o1_game():
    seg = "".join(f'<span class="{"on" if t == "Wine" else ""}">{t}</span>' for _, t in GTABS)
    rows = "".join(
        f'<div class="row"><div><b>{t}</b><small>{d}</small></div><span class="tg{" on" if v else ""}"><i></i></span></div>'
        for t, d, v in WINE_TOGGLES[:4])
    return collection(dim=True) + f'''
<div class="g-drawer">
  <div class="dh">
    <div class="dcover" style="background-image:url({COV[0]})"></div>
    <div><div class="kick">Game settings</div><h2>Control</h2><small>Epic · Installed · 41.9 GB</small></div>
    <div class="g-x s">{ic("x",16)}</div>
  </div>
  <div class="seg">{seg}</div>
  <div class="dbody">
    <div class="card">
      <div class="fld"><label>Wine / Proton version</label><div class="in sel"><span class="chip">Proton</span><span>Proton-CachyOS 10.0-20250930</span>{ic("chevd",16)}</div></div>
      <div class="fld"><label>Prefix</label><div class="in">{ic("folder",15)}<span>~/Games/Heroic/Prefixes/Control</span><em>Change</em></div></div>
    </div>
    <div class="cap">Graphics</div>
    <div class="card">{rows}</div>
    <div class="acts"><span class="btn">Winecfg</span><span class="btn">Winetricks</span><span class="btn">Run .exe in prefix</span></div>
  </div>
  <div class="dfoot"><span class="ghost">Use defaults</span><span class="btn pri">Done</span></div>
</div>'''


def o1_kit():
    return f'''<div class="kit">
<span class="btn pri">Install</span><span class="btn">Change</span><span class="ghost">Reset</span>
<div class="seg sm"><span class="on">Images</span><span>Details</span><span>Wine</span></div>
<span class="tg on"><i></i></span><span class="tg"><i></i></span>
<div class="in sel" style="width:260px"><span>Proton-CachyOS 10.0</span>{ic("chevd",16)}</div>
</div>'''


# ================================================================ OPTION 2 — Console
def o2_menu():
    items = "".join(f'<div class="ri{" on" if i == 1 else ""}">{ic(n,26)}<span>{t}</span>{"<em>1 active</em>" if t == "Downloads" else ""}</div>'
                    for i, (n, t) in enumerate(MENU))
    return collection(dim=True) + f'''
<div class="k-rail">
  <div class="k-brand">HERONITE</div>
  {items}
  <div class="k-sep"></div>
  <div class="ri">{ic("plus",26)}<span>Add game</span></div>
  <div class="ri">{ic("gear",26)}<span>Settings</span></div>
  <div class="k-hint"><span><b class="a">A</b>Select</span><span><b class="b">B</b>Back</span></div>
</div>'''


def o2_settings():
    cats = "".join(f'<div class="kc{" on" if i == 0 else ""}">{ic(n,22)}{t}</div>' for i, (n, t) in enumerate(SNAV))
    def row(t, val, focus=False, kind="tg"):
        if kind == "tg":
            v = f'<span class="kv {"on" if val else ""}">{"On" if val else "Off"}</span>'
        else:
            v = f'<span class="kcyc">{ic("chevl",18)}<b>{val}</b>{ic("chev",18)}</span>'
        return f'<div class="kr{" focus" if focus else ""}"><span>{t}</span>{v}</div>'
    return f'''<div class="k-full" style="background-image:linear-gradient(90deg,#0b0e13 35%,rgba(11,14,19,.86)),url({BGB})">
  <div class="k-top"><span class="k-brand">SETTINGS</span><span class="k-tabs"><b class="lb">LB</b><span class="on">App</span><span>Game defaults</span><span>Library</span><span>Downloads</span><b class="lb">RB</b></span><span class="k-clock">21:04</span></div>
  <div class="k-body">
    <div class="k-cats">{cats}</div>
    <div class="k-rows">
      <h2>General</h2>
      {row("Default install folder", "~/Games/Heroic", kind="cyc")}
      {row("Exit to system tray", True, focus=True)}
      {row("Start minimized", False)}
      {row("Minimize after game launch", False)}
      {row("Add games to Steam automatically", False)}
      {row("Add games to the start menu", True)}
      {row("Language", "English", kind="cyc")}
      <p class="k-help">Closing the window keeps Heronite running in the tray so games and downloads keep going.</p>
    </div>
  </div>
  <div class="k-foot"><span><b class="a">A</b>Toggle</span><span><b class="b">B</b>Back</span><span><b class="x">X</b>Reset to default</span><span><b class="lb">LB</b><b class="lb">RB</b>Switch tab</span></div>
</div>'''


def o2_game():
    tabs = "".join(f'<span class="{"on" if t == "Wine" else ""}">{t}</span>' for _, t in GTABS)
    def r(t, v, f=False, cyc=False):
        val = f'<span class="kcyc">{ic("chevl",18)}<b>{v}</b>{ic("chev",18)}</span>' if cyc else f'<span class="kv {"on" if v else ""}">{"On" if v else "Off"}</span>'
        return f'<div class="kr{" focus" if f else ""}"><span>{t}</span>{val}</div>'
    return f'''<div class="k-full game">
  <div class="k-hero" style="background-image:url({BG})"></div>
  <div class="k-gt"><div class="k-cover" style="background-image:url({COV[0]})"></div><div><small>GAME SETTINGS</small><h1>Control</h1><p>Epic Games · Installed · 3h 20m played</p></div></div>
  <div class="k-tabs2"><b class="lb">LB</b>{tabs}<b class="lb">RB</b></div>
  <div class="k-rows wide">
    {r("Wine / Proton", "Proton-CachyOS 10.0", True, True)}
    {r("DXVK", True)}{r("VKD3D-Proton", True)}{r("DXVK-NVAPI", False)}{r("Esync / Fsync", True)}
  </div>
  <div class="k-foot"><span><b class="a">A</b>Change</span><span><b class="b">B</b>Back</span><span><b class="y">Y</b>Winetricks</span><span><b class="x">X</b>Use defaults</span></div>
</div>'''


def o2_kit():
    return f'''<div class="kit">
<span class="kbtn on">Play</span><span class="kbtn">Install</span>
<span class="kv on">On</span><span class="kv">Off</span>
<span class="kcyc">{ic("chevl",18)}<b>Proton-CachyOS 10</b>{ic("chev",18)}</span>
<span class="k-hint inline"><span><b class="a">A</b>Select</span><span><b class="b">B</b>Back</span><span><b class="lb">LB</b>Tab</span></span>
</div>'''


# ================================================================ OPTION 3 — Comando
def o3_menu():
    def it(i, t, k, on=False, sub=""):
        return f'<div class="ci{" on" if on else ""}">{ic(i,16)}<span>{t}</span>{f"<small>{sub}</small>" if sub else ""}<kbd>{k}</kbd></div>'
    return collection(dim=True) + f'''
<div class="c-pal">
  <div class="c-in">{ic("search",16)}<span class="typed">wi<i class="caret"></i></span><span class="ph">Type a command, page or game…</span><kbd>Esc</kbd></div>
  <div class="c-grp">Go to</div>
  {it("wine","Wine Manager","Ctrl 3",True,"Download and manage Proton / Wine")}
  {it("grid","Collection","Ctrl 1")}
  {it("download","Downloads","Ctrl 2")}
  {it("user","Epic Games","Ctrl 4")}
  {it("pad","Emulators","Ctrl 5")}
  <div class="c-grp">Actions</div>
  {it("wine","Open Wine settings of Control","↵",sub="Game settings › Wine")}
  {it("plus","Add game…","Ctrl N")}
  {it("refresh","Refresh library and scan ROMs","F5")}
  {it("gear","Settings","Ctrl ,")}
  <div class="c-foot"><span><kbd>↑</kbd><kbd>↓</kbd> move</span><span><kbd>↵</kbd> open</span><span><kbd>Tab</kbd> filter by type</span></div>
</div>'''


def o3_settings():
    nav = "".join(f'<div class="cn{" on" if i == 0 else ""}">{ic(n,15)}<span>{t}</span><small>{c}</small></div>'
                  for i, ((n, t), c) in enumerate(zip(SNAV, [14, 22, 9, 6, 11, "", ""])))
    def s(t, d, ctrl, mod=False):
        return f'<div class="cs">{"<i class=mod></i>" if mod else ""}<div><b>{t}</b><small>{d}</small></div>{ctrl}</div>'
    sw = lambda on: f'<span class="csw{" on" if on else ""}"><i></i></span>'
    return f'''<div class="c-app">
  <div class="c-top">{ic("menu",16)}<span class="crumb">Settings <i>›</i> <b>General</b></span><div class="c-filter">{ic("search",14)}<span>Filter settings</span><kbd>/</kbd></div><span class="c-saved">{ic("check",14)} Saved</span></div>
  <div class="c-split">
    <div class="c-nav">{nav}<div class="c-navfoot">config.json · GamesConfig/</div></div>
    <div class="c-list">
      <div class="c-sec">Folders</div>
      {s("Default install folder", "Where new games are installed.", '<span class="cpath">~/Games/Heroic</span>')}
      {s("Wine prefixes folder", "Parent folder for new prefixes.", '<span class="cpath">~/Games/Heroic/Prefixes</span>')}
      {s("Steam folder", "Used to find Valve Proton and to add non-Steam games.", '<span class="cpath">~/.steam/steam</span>')}
      <div class="c-sec">Window and tray</div>
      {s("Exit to system tray", "Closing the window keeps the app running.", sw(True), True)}
      {s("Start minimized", "Open straight into the tray.", sw(False))}
      {s("Minimize after game launch", "", sw(False))}
      <div class="c-sec">Shortcuts</div>
      {s("Add to start menu automatically", "Creates a .desktop entry after each install.", sw(True), True)}
      {s("Add to Steam automatically", "Writes shortcuts.vdf (Steam must be closed).", sw(False))}
    </div>
  </div>
  <div class="c-status"><span>{ic("dot",10)} 2 settings differ from Heroic defaults</span><span>Ctrl K commands</span></div>
</div>'''


def o3_game():
    secs = "".join(f'<div class="cn{" on" if t == "Wine" else ""}">{ic(n,15)}<span>{t}</span>{"<small>3</small>" if t == "Wine" else ""}</div>' for n, t in GTABS)
    sw = lambda on: f'<span class="csw{" on" if on else ""}"><i></i></span>'
    rows = "".join(f'<div class="cs">{"<i class=mod></i>" if t in ("DXVK-NVAPI",) else ""}<div><b>{t}</b><small>{d}</small></div>{sw(v)}</div>' for t, d, v in WINE_TOGGLES)
    return collection(dim=True) + f'''
<div class="c-modal">
  <div class="c-mh"><span class="crumb">Control <i>›</i> Settings <i>›</i> <b>Wine</b></span><div class="c-filter sm">{ic("search",14)}<span>Filter</span></div><span class="c-x">{ic("x",15)}</span></div>
  <div class="c-split">
    <div class="c-nav">{secs}</div>
    <div class="c-list">
      <div class="c-sec">Runtime</div>
      <div class="cs"><i class="mod"></i><div><b>Wine / Proton version</b><small>Overrides the default (GE-Proton10-25).</small></div><span class="csel">Proton-CachyOS 10.0-20250930 {ic("chevd",14)}</span></div>
      <div class="cs"><div><b>Prefix</b><small>Created on first launch.</small></div><span class="cpath">~/Games/Heroic/Prefixes/Control</span></div>
      <div class="c-sec">Graphics</div>
      {rows}
      <div class="c-tools"><span class="cbtn">Winecfg</span><span class="cbtn">Winetricks</span><span class="cbtn">Run .exe…</span><span class="cbtn ghost">Open prefix folder</span></div>
    </div>
  </div>
  <div class="c-status"><span>GamesConfig/Control.json · 2 overrides</span><span class="lnk">Reset all to defaults</span></div>
</div>'''


def o3_kit():
    return f'''<div class="kit">
<span class="cbtn pri">Install</span><span class="cbtn">Change</span><span class="cbtn ghost">Reset</span>
<span class="csw on"><i></i></span><span class="csw"><i></i></span>
<span class="csel">Proton-CachyOS 10.0 {ic("chevd",14)}</span><span class="cpath">~/Games/Heroic</span>
<span><kbd>Ctrl</kbd> <kbd>K</kbd></span>
</div>'''


# ================================================================ OPTION 4 — Editorial
def o4_menu():
    def it(n, t, k="", on=False):
        return f'<div class="ei{" on" if on else ""}">{ic(n,17)}<span>{t}</span><kbd>{k}</kbd></div>'
    return collection() + f'''
<div class="e-pop">
  <div class="e-lbl">Library</div>
  {it("grid","Collection","Ctrl 1",True)}{it("download","Downloads","Ctrl 2")}
  <div class="e-lbl">Manage</div>
  {it("wine","Wine Manager")}{it("user","Epic Games")}{it("pad","Emulators")}
  <div class="e-hr"></div>
  {it("plus","Add game…","Ctrl N")}{it("gear","Settings…","Ctrl ,")}
</div>'''


def o4_group(title, rows):
    return f'<div class="e-gt">{title}</div><div class="e-grp">{rows}</div>'


def o4_tg(t, d, v):
    return f'<div class="e-r"><div><span>{t}</span>{f"<small>{d}</small>" if d else ""}</div><span class="e-tg{" on" if v else ""}"><i></i></span></div>'


def o4_settings():
    nav = "".join(f'<div class="en{" on" if i == 0 else ""}">{ic(n,16)}{t}</div>' for i, (n, t) in enumerate(SNAV))
    folders = "".join(f'<div class="e-r"><div><span>{t}</span></div><span class="e-val">{v} {ic("chev",15)}</span></div>' for t, v in
                      [("Install folder", "~/Games/Heroic"), ("Wine prefixes", "~/Games/Heroic/Prefixes"), ("Steam", "~/.steam/steam")])
    return f'''<div class="e-app">
  <div class="e-side">
    <div class="e-back">{ic("chevl",16)} Collection</div>
    <div class="e-search">{ic("search",14)} Search settings</div>
    {nav}
  </div>
  <div class="e-main">
    <h1>General</h1>
    {o4_group("Folders", folders)}
    {o4_group("Window and tray", "".join(o4_tg(*t) for t in TOGGLES_GENERAL[:3]))}
    {o4_group("Shortcuts", "".join(o4_tg(*t) for t in TOGGLES_SHORTCUTS))}
  </div>
</div>'''


def o4_game():
    tabs = "".join(f'<span class="{"on" if t == "Wine" else ""}">{t}</span>' for _, t in GTABS)
    return collection(dim=True) + f'''
<div class="e-sheet">
  <div class="e-sh">
    <div class="e-cv" style="background-image:url({COV[0]})"></div>
    <div><h2>Control</h2><small>Epic Games · Installed · 41.9 GB</small></div>
    <span class="e-x">{ic("x",16)}</span>
  </div>
  <div class="e-tabs">{tabs}</div>
  <div class="e-sb">
    {o4_group("Runtime", f'<div class="e-r"><div><span>Wine / Proton</span></div><span class="e-val">Proton-CachyOS 10.0-20250930 {ic("chevd",15)}</span></div><div class="e-r"><div><span>Prefix</span></div><span class="e-val">~/Games/Heroic/Prefixes/Control {ic("chev",15)}</span></div>')}
    {o4_group("Graphics", "".join(o4_tg(*t) for t in WINE_TOGGLES[:3]))}
    <div class="e-tools"><span class="ebtn">Winecfg</span><span class="ebtn">Winetricks</span><span class="ebtn">Run program…</span></div>
  </div>
  <div class="e-foot"><span class="elink">Use default settings</span><span class="ebtn pri">Done</span></div>
</div>'''


def o4_kit():
    return f'''<div class="kit">
<span class="ebtn pri">Install</span><span class="ebtn">Change</span><span class="elink">Reset</span>
<div class="e-tabs sm"><span class="on">Images</span><span>Details</span><span>Wine</span></div>
<span class="e-tg on"><i></i></span><span class="e-tg"><i></i></span>
<div class="e-grp" style="width:300px"><div class="e-r"><div><span>Wine / Proton</span></div><span class="e-val">CachyOS 10 {ic("chevd",15)}</span></div></div>
</div>'''


# ================================================================ OPTION 5 — Imersivo
def o5_menu():
    return collection(dim=True) + f'''
<div class="i-mega">
  <div class="i-card big"><div class="i-ico">{ic("grid",22)}</div><b>Collection</b><small>210 games · 4 playing</small><div class="i-strip">{"".join(f'<i style="background-image:url({c})"></i>' for c in COV[:6])}</div></div>
  <div class="i-card"><div class="i-ico">{ic("download",22)}</div><b>Downloads</b><small>Alan Wake 2 · 42% · 18 MB/s</small><div class="i-bar"><i style="width:42%"></i></div></div>
  <div class="i-card"><div class="i-ico">{ic("wine",22)}</div><b>Wine Manager</b><small>Proton-CachyOS 10 · update available</small><span class="i-badge">1</span></div>
  <div class="i-card"><div class="i-ico">{ic("user",22)}</div><b>Epic Games</b><small>Signed in · 312 games</small></div>
  <div class="i-card"><div class="i-ico">{ic("pad",22)}</div><b>Emulators</b><small>6 emulators · 1,204 ROMs</small></div>
  <div class="i-actions"><span class="ibtn pri">{ic("plus",16)} Add game</span><span class="ibtn">{ic("gear",16)} Settings</span></div>
</div>'''


def o5_settings():
    chips = "".join(f'<span class="{"on" if i == 0 else ""}">{ic(n,15)}{t}</span>' for i, (n, t) in enumerate(SNAV))
    tg = lambda t, d, v: f'<div class="i-r"><div><b>{t}</b>{f"<small>{d}</small>" if d else ""}</div><span class="i-tg{" on" if v else ""}"><i></i></span></div>'
    return f'''<div class="i-app">
  <div class="i-banner" style="background-image:linear-gradient(180deg,rgba(9,11,15,.2),#0c0f14),url({BG})"><span class="i-back">{ic("chevl",16)} Collection</span><h1>Settings</h1><div class="i-chips">{chips}</div></div>
  <div class="i-cards">
    <div class="i-grp"><div class="i-gh">{ic("folder",18)}<b>Folders</b></div>
      <div class="i-path"><small>Install folder</small><span>~/Games/Heroic</span></div>
      <div class="i-path"><small>Wine prefixes</small><span>~/Games/Heroic/Prefixes</span></div>
      <div class="i-path"><small>Steam</small><span>~/.steam/steam</span></div></div>
    <div class="i-grp"><div class="i-gh">{ic("monitor",18)}<b>Window and tray</b></div>{"".join(tg(*t) for t in TOGGLES_GENERAL[:4])}</div>
    <div class="i-grp"><div class="i-gh">{ic("plus",18)}<b>Shortcuts</b></div>{"".join(tg(*t) for t in TOGGLES_SHORTCUTS)}</div>
    <div class="i-grp"><div class="i-gh">{ic("wine",18)}<b>Wine and Proton</b></div>{tg("Allow Valve Proton builds","Custom forks are recommended.",False)}{tg("Download GE-Proton to Steam","",False)}</div>
  </div>
</div>'''


def o5_game():
    tabs = "".join(f'<div class="{"on" if t == "Wine" else ""}">{ic(n,17)}{t}</div>' for n, t in GTABS)
    tg = lambda t, d, v: f'<div class="i-r"><div><b>{t}</b><small>{d}</small></div><span class="i-tg{" on" if v else ""}"><i></i></span></div>'
    return f'''<div class="i-game" style="--acc:#ff5a4e">
  <div class="i-art" style="background-image:linear-gradient(90deg,rgba(12,15,20,0) 40%,#0c0f14 100%),linear-gradient(0deg,#0c0f14,rgba(12,15,20,0) 45%),url({BG})">
    <span class="i-back">{ic("chevl",16)} Back to Control</span>
    <div class="i-id"><div class="i-cv" style="background-image:url({COV[0]})"></div><h1>Control</h1><p>Epic Games · 3h 20m played · Proton-CachyOS</p></div>
  </div>
  <div class="i-panel">
    <div class="i-vt">{tabs}</div>
    <div class="i-pc">
      <h2>Wine</h2>
      <div class="i-selbig"><small>Wine / Proton version</small><b>Proton-CachyOS 10.0-20250930</b>{ic("chevd",18)}</div>
      <div class="i-path"><small>Prefix</small><span>~/Games/Heroic/Prefixes/Control</span></div>
      <div class="i-grp flat">{"".join(tg(*t) for t in WINE_TOGGLES[:4])}</div>
      <div class="i-tools"><span class="ibtn">Winecfg</span><span class="ibtn">Winetricks</span><span class="ibtn">Run .exe</span></div>
    </div>
  </div>
</div>'''


def o5_kit():
    return f'''<div class="kit" style="--acc:#ff5a4e">
<span class="ibtn pri">{ic("play",14)} Play</span><span class="ibtn">Change</span>
<span class="i-tg on"><i></i></span><span class="i-tg"><i></i></span>
<div class="i-selbig" style="width:300px;margin:0"><small>Wine / Proton</small><b>Proton-CachyOS 10</b>{ic("chevd",18)}</div>
<span class="i-note">Accent follows the game's art (Control → red)</span>
</div>'''


OPTIONS = [
    ("o1", "1 · Vidro", "Painéis translúcidos flutuando sobre a arte do jogo",
     "Tudo vira vidro fosco sobre a arte, combinando com a Collection, que já usa a arte borrada. O menu ☰ vira uma grade de blocos com informação ao vivo. As configurações do jogo abrem numa <b>gaveta lateral</b>, com o jogo ainda visível atrás. Usa interruptores no lugar de caixas de marcar, abas em pílula e cantos bem arredondados.",
     "Médio. O GPUI não borra o fundo atrás de um painel, então o efeito usa uma cópia já borrada da arte, como a Collection já faz.",
     [o1_menu, o1_settings, o1_game], o1_kit),
    ("o2", "2 · Console", "Modo TV / Steam Deck, pronto para controle",
     "Letras grandes, uma linha focada por vez com contorno forte, valores trocados com ◀ ▶ e dicas de botão do controle no rodapé (A, B, LB/RB). O menu vira uma <b>barra lateral de altura total</b>. As configurações do jogo ocupam a tela toda, com a arte no topo. Prepara o terreno para a navegação por controle.",
     "Médio-alto: exige foco e navegação por teclado e controle em todas as telas.",
     [o2_menu, o2_settings, o2_game], o2_kit),
    ("o3", "3 · Comando", "Denso e feito para teclado, como VS Code, Linear ou Raycast",
     "O menu ☰ vira uma <b>paleta de comandos</b> (Ctrl K) que procura páginas, ações e jogos. As configurações são uma lista densa com busca, descrição curta e um marcador nos valores diferentes do padrão. Uma barra de status no rodapé mostra onde está sendo salvo (GamesConfig/Control.json). Os caminhos aparecem em fonte monoespaçada.",
     "Baixo-médio: componentes simples, só a busca e a paleta são novas.",
     [o3_menu, o3_settings, o3_game], o3_kit),
    ("o4", "4 · Editorial", "Limpo e tipográfico, no estilo dos ajustes do macOS e iOS",
     "Superfícies sólidas e listas agrupadas com cantos arredondados: rótulo à esquerda, interruptor ou valor com › à direita. Títulos grandes em Rubik, cor de destaque usada com parcimônia e botões preenchidos suaves com texto em caixa normal. O menu vira um dropdown clássico com seções e atalhos. As configurações do jogo abrem num modal centralizado.",
     "Baixo: o mais direto de implementar e o mais fácil de manter consistente.",
     [o4_menu, o4_settings, o4_game], o4_kit),
    ("o5", "5 · Imersivo", "A arte e a cor do jogo comandam a interface",
     "O menu ☰ vira um <b>painel grande com cartões ao vivo</b>: progresso do download, conta Epic, atualização do Proton, número de ROMs. As configurações globais têm um banner e grupos em cartões. As configurações de um jogo viram uma <b>página inteira</b>, com a arte à esquerda e a cor de destaque tirada da arte (vermelho em Control).",
     "Alto: exige extrair a cor da arte e montar layouts maiores. É o mais \"marcante\".",
     [o5_menu, o5_settings, o5_game], o5_kit),
]

SCREEN_NAMES = ["Menu ☰", "Configurações globais", "Configurações do jogo (aba Wine)"]

CSS = open(sys.argv[2]).read()

tabs = "".join(f'<a href="#{k}" class="otab" data-k="{k}"><b>{n}</b><small>{s}</small></a>' for k, n, s, *_ in OPTIONS)
sections = []
for k, name, sub, desc, effort, screens, kit in OPTIONS:
    boards = "".join(board(k, SCREEN_NAMES[i], f()) for i, f in enumerate(screens))
    sections.append(f'''
<section id="{k}" class="opt">
  <header class="oh"><h2>{name} <span>{sub}</span></h2><p>{desc}</p><p class="eff"><b>Esforço no GPUI:</b> {effort}</p></header>
  <div class="kitwrap {k}"><div class="kitlbl">Componentes</div>{kit()}</div>
  {boards}
</section>''')

html = f'''<!doctype html><html lang="pt-BR"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>HeroniteRust — 5 propostas de design</title>
<link rel="preconnect" href="https://fonts.googleapis.com"><link href="https://fonts.googleapis.com/css2?family=Cabin:wght@400;600&family=Rubik:wght@400;500;600&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet">
<style>{CSS}</style></head><body>{SPRITE}<template id="coll-t">{collection_real()}</template><div class="wrap">
<div class="top">
  <div class="intro"><h1>HeroniteRust · nova aparência</h1>
  <p>Cinco direções para os <b>menus, páginas e modais</b>. A Collection (lista, capas e painel do jogo) fica como está e aparece ao fundo só como contexto. Cada proposta mostra as mesmas três telas e os componentes básicos dela. Dá para escolher uma ou misturar partes.</p></div>
  <nav class="otabs">{tabs}</nav>
</div>
{"".join(sections)}
<section class="opt cmp"><header class="oh"><h2>Comparação rápida</h2></header>
<table>
<tr><th></th><th>1 · Vidro</th><th>2 · Console</th><th>3 · Comando</th><th>4 · Editorial</th><th>5 · Imersivo</th></tr>
<tr><td>Personalidade</td><td>Leve, moderno, combina com a arte</td><td>Sala / TV</td><td>Ferramenta de power user</td><td>Sóbrio, “nativo”</td><td>Marcante, cada jogo com sua cor</td></tr>
<tr><td>Densidade</td><td>Média</td><td>Baixa</td><td>Alta</td><td>Média</td><td>Baixa-média</td></tr>
<tr><td>Teclado e controle</td><td>Bom</td><td>Excelente</td><td>Excelente (teclado)</td><td>Bom</td><td>Bom</td></tr>
<tr><td>Menu ☰ vira</td><td>Grade de blocos com info</td><td>Barra lateral de altura total</td><td>Paleta de comandos</td><td>Dropdown com seções</td><td>Painel com cartões ao vivo</td></tr>
<tr><td>Configurações do jogo</td><td>Gaveta lateral</td><td>Tela cheia</td><td>Modal denso com busca</td><td>Modal centralizado</td><td>Página inteira com a arte</td></tr>
<tr><td>Esforço</td><td>Médio</td><td>Médio-alto</td><td>Baixo-médio</td><td>Baixo</td><td>Alto</td></tr>
</table></section>
<script>
document.querySelectorAll('.coll-slot').forEach(s=>{{const n=document.getElementById('coll-t').content.firstElementChild.cloneNode(true);n.className+=s.dataset.cls;s.replaceWith(n);}});
function fit(){{document.querySelectorAll('.scaler').forEach(s=>{{const b=s.firstElementChild;const k=s.clientWidth/1280;b.style.transform='scale('+k+')';s.style.height=(800*k)+'px';}});}}
addEventListener('resize',fit);fit();
const obs=new IntersectionObserver(es=>es.forEach(e=>{{if(e.isIntersecting)document.querySelectorAll('.otab').forEach(t=>t.classList.toggle('on',t.dataset.k===e.target.id));}}),{{rootMargin:'-40% 0px -55% 0px'}});
document.querySelectorAll('section.opt[id]').forEach(s=>obs.observe(s));
</script></div></body></html>'''
VARS = {"bg": BG, "bgb": BGB, **{f"c{i}": c for i, c in enumerate(COV)}}
root = ":root{" + "".join(f"--{k}:url({v});" for k, v in VARS.items()) + "}"
for k, v in VARS.items():
    html = html.replace(f"url({v})", f"var(--{k})")
html = html.replace("<style>", "<style>" + root, 1)
open(sys.argv[3], "w").write(html)
print(len(html) // 1024, "KB")

# ---- individual boards (for screenshots)
import os
os.makedirs("/tmp/design-gen/boards", exist_ok=True)
head = html.split("<body>")[0]
tpl = f'{SPRITE}<template id="coll-t">{collection_real()}</template>'
js = "<script>document.querySelectorAll('.coll-slot').forEach(s=>{const n=document.getElementById('coll-t').content.firstElementChild.cloneNode(true);n.className+=s.dataset.cls;s.replaceWith(n);});</script>"
for k, name, sub, desc, effort, screens, kit in OPTIONS:
    for i, f in enumerate(screens):
        page = head.replace("html,body{background:transparent}", "") + f'<body style="margin:0;background:#07090c">{tpl}<div class="board {k}" style="position:relative">{f()}</div>{js}</body></html>'
        open(f"/tmp/design-gen/boards/{k}-{i}.html", "w").write(page)
    page = head + f'<body style="margin:0;background:#0a0c10;width:1100px">{tpl}<div class="kitwrap {k}" style="margin:0">{kit()}</div></body></html>'
    open(f"/tmp/design-gen/boards/{k}-kit.html", "w").write(page)
