use super::*;
use crate::games::{find, Kind, KNOWN};
use crate::model::GameLinks;

fn tree(paths: &[&str]) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    for p in paths {
        let full = t.path().join(p);
        if p.ends_with('/') {
            std::fs::create_dir_all(&full).unwrap();
        } else {
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, b"x").unwrap();
        }
    }
    t
}

fn profile(steam_id: &str) -> Profile {
    Profile::of(&GameLinks::default(), find(Some(steam_id), ""))
}

fn map(rules: &[CopyRule]) -> Vec<String> {
    let mut v: Vec<String> = rules.iter().map(|r| format!("{} -> {}", r.source, r.destination)).collect();
    v.sort();
    v
}

const SKYRIM: &str = "489830";

#[test]
fn skyrim_texture_mod_in_a_wrapper_goes_under_data() {
    let c = tree(&["Cool Textures-123-1/textures/a.dds", "Cool Textures-123-1/meshes/b.nif"]);
    let rules = auto(c.path(), &profile(SKYRIM), None, &ModSource::File);
    assert_eq!(map(&rules), ["Cool Textures-123-1/meshes -> Data/meshes", "Cool Textures-123-1/textures -> Data/textures"]);
    assert!(rules.iter().all(|r| r.is_folder));
}

#[test]
fn script_extender_goes_to_the_game_folder() {
    let c = tree(&["skse64_loader.exe", "skse64_1_6_1170.dll", "Data/Scripts/x.pex", "src/readme.txt"]);
    let rules = auto(c.path(), &profile(SKYRIM), None, &ModSource::File);
    assert_eq!(
        map(&rules),
        ["Data -> Data", "skse64_1_6_1170.dll -> skse64_1_6_1170.dll", "skse64_loader.exe -> skse64_loader.exe", "src -> src"]
    );
}

#[test]
fn a_lone_plugin_goes_under_data_and_a_data_folder_in_a_wrapper_to_the_root() {
    let c = tree(&["MyMod.esp"]);
    assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["MyMod.esp -> Data/MyMod.esp"]);
    let c = tree(&["Some Mod v2/data/MyMod.esp", "Some Mod v2/data/textures/a.dds"]);
    assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["Some Mod v2/data -> Data"]);
}

#[test]
fn executables_alone_go_to_the_root_and_odd_files_under_data() {
    let c = tree(&["ENBSeries.dll", "d3d11.dll"]);
    assert_eq!(
        map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)),
        ["ENBSeries.dll -> ENBSeries.dll", "d3d11.dll -> d3d11.dll"]
    );
    let c = tree(&["config.ini"]);
    assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["config.ini -> Data/config.ini"]);
}

#[test]
fn the_installer_folder_is_ignored() {
    let c = tree(&["fomod/ModuleConfig.xml", "Textures/a.dds"]);
    assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["Textures -> Data/Textures"]);
    let c = tree(&["Wrapper/fomod/info.xml", "Wrapper/meshes/a.nif"]);
    assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["Wrapper/meshes -> Data/meshes"]);
}

fn ts(namespace: &str, name: &str) -> ModSource {
    ModSource::Thunderstore { community: "valheim".into(), namespace: namespace.into(), name: name.into() }
}

fn valheim() -> Profile {
    profile("892970")
}

#[test]
fn bepinex_pack_goes_to_the_game_folder() {
    let c = tree(&["manifest.json", "icon.png", "README.md", "BepInExPack_Valheim/BepInEx/core/a.dll", "BepInExPack_Valheim/winhttp.dll"]);
    let rules = auto(c.path(), &valheim(), None, &ts("denikson", "BepInExPack_Valheim"));
    assert_eq!(map(&rules), ["BepInExPack_Valheim -> "]);
    let c = tree(&["manifest.json", "BepInEx/core/a.dll", "winhttp.dll", "doorstop_config.ini"]);
    let rules = auto(c.path(), &valheim(), None, &ModSource::File);
    assert_eq!(map(&rules), ["BepInEx -> BepInEx", "doorstop_config.ini -> doorstop_config.ini", "winhttp.dll -> winhttp.dll"]);
}

#[test]
fn thunderstore_plugin_with_a_loose_dll() {
    let c = tree(&["manifest.json", "icon.png", "README.md", "LICENSE", "CHANGELOG.md", "MyPlugin.dll"]);
    let rules = auto(c.path(), &valheim(), None, &ts("Author", "MyPlugin"));
    assert_eq!(map(&rules), ["MyPlugin.dll -> BepInEx/plugins/Author-MyPlugin/MyPlugin.dll"]);
}

#[test]
fn thunderstore_plugin_with_plugins_and_config_folders() {
    let c = tree(&["manifest.json", "plugins/a.dll", "config/a.cfg", "patchers/p.dll", "assets/x.bundle"]);
    let rules = auto(c.path(), &valheim(), None, &ts("Author", "Thing"));
    assert_eq!(
        map(&rules),
        [
            "assets -> BepInEx/plugins/Author-Thing/assets",
            "config -> BepInEx/config",
            "patchers -> BepInEx/patchers/Author-Thing",
            "plugins -> BepInEx/plugins/Author-Thing"
        ]
    );
    let c = tree(&["BepInEx/plugins/x/a.dll", "BepInEx/config/a.cfg"]);
    assert_eq!(map(&auto(c.path(), &valheim(), None, &ts("A", "B"))), ["BepInEx -> BepInEx"]);
}

#[test]
fn a_wrapper_around_a_plugin_is_unwrapped_and_names_the_folder() {
    let c = tree(&["CoolMod-1.2/CoolMod.dll"]);
    let rules = auto(c.path(), &valheim(), None, &ModSource::File);
    assert_eq!(map(&rules), ["CoolMod-1.2/CoolMod.dll -> BepInEx/plugins/CoolMod/CoolMod.dll"]);
}

fn cyberpunk() -> Profile {
    profile("1091500")
}

#[test]
fn cyberpunk_archive_with_r6_goes_to_the_root() {
    let game = tree(&["archive/pc/content/a.archive", "bin/x64/Cyberpunk2077.exe", "r6/config/a.xml"]);
    let c = tree(&["archive/pc/mod/m.archive", "r6/scripts/m.reds"]);
    let rules = auto(c.path(), &cyberpunk(), Some(game.path()), &ModSource::File);
    assert_eq!(map(&rules), ["archive -> archive", "r6 -> r6"]);
    let c = tree(&["My Mod 2.0/archive/pc/mod/m.archive"]);
    assert_eq!(map(&auto(c.path(), &cyberpunk(), Some(game.path()), &ModSource::File)), ["My Mod 2.0/archive -> archive"]);
}

#[test]
fn witcher_mod_folder_goes_under_mods() {
    let game = tree(&["bin/x64/witcher3.exe", "content/content0/a.bundle", "dlc/dlc1/a.w3strings", "Mods/"]);
    let witcher = profile("292030");
    let c = tree(&["modFoo/content/scripts/a.ws"]);
    assert_eq!(map(&auto(c.path(), &witcher, Some(game.path()), &ModSource::File)), ["modFoo -> Mods/modFoo"]);
    let c = tree(&["mods/modFoo/content/scripts/a.ws", "readme.txt"]);
    assert_eq!(map(&auto(c.path(), &witcher, Some(game.path()), &ModSource::File)), ["mods -> mods", "readme.txt -> Mods/readme.txt"]);
    let c = tree(&["dlc/dlcFoo/content/a.bundle"]);
    assert_eq!(map(&auto(c.path(), &witcher, Some(game.path()), &ModSource::File)), ["dlc -> dlc"]);
    let c = tree(&["Wrapper v1/mods/modFoo/content/a.bundle"]);
    assert_eq!(map(&auto(c.path(), &witcher, Some(game.path()), &ModSource::File)), ["Wrapper v1/mods -> mods"]);
}

#[test]
fn a_user_chosen_folder_is_used_for_unknown_games() {
    let links = GameLinks { mod_root: Some("Plugins".into()), ..Default::default() };
    let p = Profile::of(&links, None);
    let c = tree(&["foo.dll", "bar.txt"]);
    assert_eq!(map(&auto(c.path(), &p, None, &ModSource::File)), ["bar.txt -> Plugins/bar.txt", "foo.dll -> Plugins/foo.dll"]);
    assert!(auto(tree(&[]).path(), &p, None, &ModSource::File).is_empty());
}

#[test]
fn a_game_folder_the_game_does_not_ship_is_not_peeled() {
    // Control's loose files go in `data_pc`, which the game folder lacks.
    let game = tree(&["data/x.rmdp", "Control_DX12.exe"]);
    let t = tree(&["data_pc/textures/videos/intro/startvideos.tex"]);
    let rules = auto(t.path(), &Profile::of(&Default::default(), None), Some(game.path()), &ModSource::File);
    assert_eq!(map(&rules), ["data_pc -> data_pc"]);
    // The same in a download-named wrapper: the wrapper goes, the tree stays.
    let t = tree(&["No Intro Screen-12-1-0-1576850747/data_pc/textures/videos/intro/startvideos.tex"]);
    let rules = auto(t.path(), &Profile::of(&Default::default(), None), Some(game.path()), &ModSource::File);
    assert_eq!(map(&rules), ["No Intro Screen-12-1-0-1576850747/data_pc -> data_pc"]);
}

#[test]
fn download_names() {
    assert!(named_like_a_download("No Intro Screen-12-1-0-1576850747"));
    assert!(named_like_a_download("SkyUI_5_2"));
    assert!(named_like_a_download("Mod v3"));
    assert!(!named_like_a_download("data_pc"));
    assert!(!named_like_a_download("textures"));
}

#[test]
fn symlinks_are_not_entries() {
    let c = tree(&["textures/a.dds"]);
    std::os::unix::fs::symlink("/etc", c.path().join("evil")).unwrap();
    assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["textures -> Data/textures"]);
}

#[test]
fn every_known_game_has_a_layout() {
    let c = tree(&["a.txt"]);
    for k in KNOWN {
        let rules = auto(c.path(), &Profile::of(&GameLinks::default(), Some(k)), None, &ModSource::File);
        assert_eq!(rules.len(), 1, "{}", k.name);
    }
}

fn auto_file(c: &tempfile::TempDir, p: &Profile, game: Option<&tempfile::TempDir>) -> Vec<String> {
    map(&auto(c.path(), p, game.map(|g| g.path()), &ModSource::File))
}

#[test]
fn docs_beside_a_lone_wrapper_do_not_block_it() {
    let c = tree(&["Cool Mod/Cool.esp", "readme.txt", "preview.png"]);
    assert_eq!(auto_file(&c, &profile(SKYRIM), None), ["Cool Mod/Cool.esp -> Data/Cool.esp"]);
    let game = tree(&["bin/x64/witcher3.exe", "Mods/"]);
    let c = tree(&["modFoo/content/a.ws", "Readme.TXT"]);
    assert_eq!(
        auto_file(&c, &profile("292030"), Some(&game)),
        ["Readme.TXT -> Mods/Readme.TXT", "modFoo -> Mods/modFoo"],
        "docs go along, they do not make modFoo something else"
    );
    let c = tree(&["Plugin-1.0/Plugin.dll", "README.txt"]);
    assert_eq!(auto_file(&c, &valheim(), None), ["Plugin-1.0/Plugin.dll -> BepInEx/plugins/Plugin/Plugin.dll"]);
}

#[test]
fn a_plugin_keeps_its_own_texts_and_pictures() {
    let c = tree(&["Plugin.dll", "logo.png", "notes.txt"]);
    assert_eq!(
        auto_file(&c, &valheim(), None),
        [
            "Plugin.dll -> BepInEx/plugins/mod/Plugin.dll",
            "logo.png -> BepInEx/plugins/mod/logo.png",
            "notes.txt -> BepInEx/plugins/mod/notes.txt"
        ]
    );
}

#[test]
fn markers_of_the_game_keep_its_tree_and_unwrap_around_it() {
    let control = profile("870780");
    let game = tree(&["data/x.rmdp", "Control_DX12.exe"]);
    let c = tree(&["data_pc/textures/videos/intro/startvideos.tex"]);
    assert_eq!(auto_file(&c, &control, Some(&game)), ["data_pc -> data_pc"]);
    let c = tree(&["Intro Pack/data_pc/textures/a.tex"]);
    assert_eq!(auto_file(&c, &control, Some(&game)), ["Intro Pack/data_pc -> data_pc"], "a wrapper around the tree goes");
    let re4 = profile("2050650");
    let game = tree(&["re4.exe", "re_chunk_000.pak"]);
    let c = tree(&["Cool Mod/natives/STM/art/a.tex.1"]);
    assert_eq!(auto_file(&c, &re4, Some(&game)), ["Cool Mod/natives -> natives"]);
    let mhw = profile("582010");
    let c = tree(&["Some Patch/nativePC/common/a.mrl2"]);
    assert_eq!(auto_file(&c, &mhw, None), ["Some Patch/nativePC -> nativePC"]);
}

#[test]
fn graphics_injectors_go_to_the_game_folder_not_data() {
    let enb = tree(&["d3d11.dll", "d3dcompiler_46e.dll", "enbseries/enbbloom.fx", "enbseries.ini", "enblocal.ini"]);
    assert_eq!(
        auto_file(&enb, &profile(SKYRIM), None),
        [
            "d3d11.dll -> d3d11.dll",
            "d3dcompiler_46e.dll -> d3dcompiler_46e.dll",
            "enblocal.ini -> enblocal.ini",
            "enbseries -> enbseries",
            "enbseries.ini -> enbseries.ini"
        ]
    );
    let reshade = tree(&["Wrapper/dxgi.dll", "Wrapper/ReShade.ini", "Wrapper/reshade-shaders/Shaders/a.fx", "Wrapper/Data/Textures/x.dds"]);
    assert_eq!(
        auto_file(&reshade, &profile(SKYRIM), None),
        [
            "Wrapper/Data -> Data",
            "Wrapper/ReShade.ini -> ReShade.ini",
            "Wrapper/dxgi.dll -> dxgi.dll",
            "Wrapper/reshade-shaders -> reshade-shaders"
        ]
    );
    let preset = tree(&["d3d11.dll", "textures/a.dds", "settings.ini"]);
    assert_eq!(
        auto_file(&preset, &profile(SKYRIM), None),
        ["d3d11.dll -> d3d11.dll", "settings.ini -> settings.ini", "textures -> Data/textures"],
        "an .ini beside a proxy DLL is the injector's"
    );
}

#[test]
fn a_mods_folder_game_decides_per_entry() {
    let game = tree(&["bin/x64/witcher3.exe", "content/content0/a.bundle", "dlc/dlc1/a.w3strings", "Mods/"]);
    let witcher = profile("292030");
    let c = tree(&["modFoo/content/scripts/a.ws", "bin/config/r4game/user_config_matrix/pc/dx11filelist.txt"]);
    assert_eq!(
        auto_file(&c, &witcher, Some(&game)),
        ["bin -> bin", "modFoo -> Mods/modFoo"],
        "the game's folder stays, the mod folder goes under Mods"
    );
    let c = tree(&["modA/content/a.ws", "modB/content/b.ws"]);
    assert_eq!(auto_file(&c, &witcher, Some(&game)), ["modA -> Mods/modA", "modB -> Mods/modB"]);
}

#[test]
fn unreal_paks_go_to_the_mods_folder_of_paks() {
    let game = tree(&["Palworld.exe", "Pal/Content/Paks/pakchunk0-Windows.pak", "Pal/Binaries/Win64/Palworld-Win64-Shipping.exe"]);
    let palworld = profile("1623730");
    let c = tree(&["Cool.pak", "Cool.ucas", "Cool.utoc"]);
    assert_eq!(
        auto_file(&c, &palworld, Some(&game)),
        [
            "Cool.pak -> Pal/Content/Paks/~mods/Cool.pak",
            "Cool.ucas -> Pal/Content/Paks/~mods/Cool.ucas",
            "Cool.utoc -> Pal/Content/Paks/~mods/Cool.utoc"
        ]
    );
    let c = tree(&["Cool Mod v2/Cool.pak"]);
    assert_eq!(auto_file(&c, &palworld, Some(&game)), ["Cool Mod v2/Cool.pak -> Pal/Content/Paks/~mods/Cool.pak"]);
    let c = tree(&["~mods/Cool.pak"]);
    assert_eq!(auto_file(&c, &palworld, Some(&game)), ["~mods -> Pal/Content/Paks/~mods"], "the folder is the destination, not peeled");
    let c = tree(&["Cool Mod/~mods/Cool.pak"]);
    assert_eq!(auto_file(&c, &palworld, Some(&game)), ["Cool Mod/~mods -> Pal/Content/Paks/~mods"]);
    let c = tree(&["Pal/Content/Paks/Cool.pak"]);
    assert_eq!(auto_file(&c, &palworld, Some(&game)), ["Pal -> Pal"]);
    let hogwarts = tree(&["Phoenix/Content/Paks/a.pak"]);
    let c = tree(&["x.pak"]);
    assert_eq!(auto_file(&c, &profile("990080"), Some(&hogwarts)), ["x.pak -> Phoenix/Content/Paks/~mods/x.pak"]);
    assert_eq!(auto_file(&c, &palworld, None), ["x.pak -> x.pak"], "no game folder to look at");
}

#[test]
fn loaders_go_next_to_the_only_executable() {
    let elden = profile("1245620");
    assert_eq!(elden.mod_root, "Game");
    let game = tree(&["Game/eldenring.exe", "Game/regulation.bin"]);
    let c = tree(&["dinput8.dll", "modloader.ini", "regulation.bin"]);
    assert_eq!(
        auto_file(&c, &elden, Some(&game)),
        ["dinput8.dll -> Game/dinput8.dll", "modloader.ini -> Game/modloader.ini", "regulation.bin -> Game/regulation.bin"]
    );
    let c = tree(&["Game/regulation.bin"]);
    assert_eq!(auto_file(&c, &elden, Some(&game)), ["Game -> Game"]);
    // Unreal: the executable is deep, the game folder has none of its own.
    let unreal = tree(&["Pal/Binaries/Win64/Palworld-Win64-Shipping.exe", "Pal/Content/Paks/a.pak"]);
    let palworld = profile("1623730");
    let c = tree(&["xinput1_3.dll", "mod.asi", "mod.ini", "Mods/Cool/main.lua", "notes.txt"]);
    assert_eq!(
        auto_file(&c, &palworld, Some(&unreal)),
        [
            "Mods -> Mods",
            "mod.asi -> Pal/Binaries/Win64/mod.asi",
            "mod.ini -> Pal/Binaries/Win64/mod.ini",
            "notes.txt -> notes.txt",
            "xinput1_3.dll -> Pal/Binaries/Win64/xinput1_3.dll"
        ]
    );
    // With an executable at the root it stays there.
    let with_root_exe = tree(&["Palworld.exe", "Pal/Binaries/Win64/Palworld-Win64-Shipping.exe"]);
    let c = tree(&["xinput1_3.dll"]);
    assert_eq!(auto_file(&c, &palworld, Some(&with_root_exe)), ["xinput1_3.dll -> xinput1_3.dll"]);
    // Two executables: no guess.
    let two = tree(&["a/x.exe", "b/y.exe"]);
    assert_eq!(auto_file(&c, &palworld, Some(&two)), ["xinput1_3.dll -> xinput1_3.dll"]);
}

#[test]
fn cyberpunk_loose_archives_go_to_the_mod_archive_folder() {
    let c = tree(&["foo.archive", "foo.xl"]);
    assert_eq!(auto_file(&c, &cyberpunk(), None), ["foo.archive -> archive/pc/mod/foo.archive", "foo.xl -> archive/pc/mod/foo.xl"]);
    let c = tree(&["Foo 1.2/foo.archive"]);
    assert_eq!(auto_file(&c, &cyberpunk(), None), ["Foo 1.2/foo.archive -> archive/pc/mod/foo.archive"]);
    let c = tree(&["red4ext/plugins/x/x.dll", "r6/scripts/x.reds"]);
    assert_eq!(auto_file(&c, &cyberpunk(), None), ["r6 -> r6", "red4ext -> red4ext"]);
}

#[test]
fn melonloader_content_is_laid_out_at_the_game_folder() {
    let c = tree(&["manifest.json", "icon.png", "README.md", "MelonLoader/net6/MelonLoader.dll", "version.dll", "dobby.dll"]);
    let source = ts("LavaGang", "MelonLoader");
    assert_eq!(
        map(&auto(c.path(), &valheim(), None, &source)),
        ["MelonLoader -> MelonLoader", "dobby.dll -> dobby.dll", "version.dll -> version.dll"]
    );
    let c = tree(&["manifest.json", "Mods/CoolMod.dll", "UserLibs/lib.dll"]);
    assert_eq!(map(&auto(c.path(), &valheim(), None, &ts("Author", "CoolMod"))), ["Mods -> Mods", "UserLibs -> UserLibs"]);
    let c = tree(&["version.dll", "dobby.dll"]);
    assert_eq!(map(&auto(c.path(), &valheim(), None, &ts("A", "Loader"))), ["dobby.dll -> dobby.dll", "version.dll -> version.dll"]);
    // A BepInEx package with a plugins folder is still BepInEx's.
    let c = tree(&["manifest.json", "plugins/a.dll"]);
    assert_eq!(map(&auto(c.path(), &valheim(), None, &ts("A", "B"))), ["plugins -> BepInEx/plugins/A-B"]);
}

#[test]
fn hades_two_follows_returnofmodding() {
    let hades = profile("1145350");
    assert_eq!(hades.kind, Kind::Generic);
    let game = tree(&["Ship/Hades2.exe", "Content/x.pkg"]);
    // Hell2Modding's real package.
    let c = tree(&["manifest.json", "icon.png", "README.md", "ReturnOfModdingPack/d3d12.dll"]);
    let rules = auto(c.path(), &hades, Some(game.path()), &ts("Hell2Modding", "Hell2Modding"));
    assert_eq!(map(&rules), ["ReturnOfModdingPack/d3d12.dll -> Ship/d3d12.dll"]);
    // A plugin (SGG_Modding-Chalk's real package).
    let c = tree(&["manifest.json", "icon.png", "README.md", "CHANGELOG.md", "LICENSE", "main.lua", "def.lua"]);
    let rules = auto(c.path(), &hades, Some(game.path()), &ts("SGG_Modding", "Chalk"));
    assert_eq!(
        map(&rules),
        [
            "def.lua -> Ship/ReturnOfModding/plugins/SGG_Modding-Chalk/def.lua",
            "main.lua -> Ship/ReturnOfModding/plugins/SGG_Modding-Chalk/main.lua"
        ]
    );
    let c = tree(&["manifest.json", "plugins/a/main.lua", "config/a.cfg"]);
    let rules = auto(c.path(), &hades, None, &ts("A", "B"));
    assert_eq!(map(&rules), ["config -> ReturnOfModding/config/A-B", "plugins -> ReturnOfModding/plugins/A-B"]);
}

#[test]
fn only_license_files_are_dropped_from_a_package() {
    let c = tree(&["manifest.json", "LICENSE", "license.txt", "License-MIT.md", "LicenseManager.dll"]);
    assert_eq!(map(&auto(c.path(), &valheim(), None, &ts("A", "B"))), ["LicenseManager.dll -> BepInEx/plugins/A-B/LicenseManager.dll"]);
    assert!(is_license("license") && is_license("licence.md") && !is_license("licensemanager.dll") && !is_license("license.dll"));
}

#[test]
fn versions_are_taken_out_of_names() {
    assert_eq!(without_version("MyPlugin-1.2.0"), "MyPlugin");
    assert_eq!(without_version("No Intro Screen-12-1-0-1576850747"), "No Intro Screen");
    assert_eq!(without_version("SkyUI_5_2"), "SkyUI");
    assert_eq!(without_version("Mod v3"), "Mod");
    assert_eq!(without_version("Plugin2"), "Plugin2");
    assert_eq!(without_version("Mod-2-Electric"), "Mod-2-Electric");
    assert_eq!(without_version("1.2.0"), "1.2.0");
}
