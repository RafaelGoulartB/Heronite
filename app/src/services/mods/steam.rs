//! Steam games start through Steam, so the environment of a mod loader
//! (`WINEDLLOVERRIDES=winhttp=n,b`) cannot be added to the launch: it goes in
//! the game's launch options in Steam. This reads them and adds it.

use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_mods::apply::merge_dll_overrides;
use heronite_mods::ModList;
use heronite_steam::{options, Access, Change};

use super::{ModsEvent, ModsService};
use gpui_kit::*;

const VAR: &str = "WINEDLLOVERRIDES";

/// The proxy DLLs the enabled mods need when the game plays with mods.
pub(super) fn wanted(list: &ModList) -> Vec<String> {
    if !list.active {
        return Vec::new();
    }
    let mut dlls: Vec<String> = list.enabled().flat_map(|m| m.dll_overrides.iter().map(|d| d.to_lowercase())).collect();
    dlls.sort();
    dlls.dedup();
    dlls
}

/// Where each word of the line starts and ends, quotes kept together.
fn words(line: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut start, mut quote) = (None, None::<char>);
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => {
                quote = Some(c);
                start.get_or_insert(i);
            }
            (None, c) if c.is_whitespace() => {
                if let Some(s) = start.take() {
                    out.push((s, i));
                }
            }
            (None, _) => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        out.push((s, line.len()));
    }
    out
}

/// The `WINEDLLOVERRIDES=` word of a launch options line: where it is and its value.
fn assignment(line: &str) -> Option<((usize, usize), String)> {
    words(line).into_iter().find_map(|(s, e)| {
        let word = &line[s..e];
        let value = word.strip_prefix(VAR)?.strip_prefix('=')?;
        let value = value.trim_matches(|c| c == '"' || c == '\'');
        Some(((s, e), value.to_string()))
    })
}

/// The DLLs of `wanted` the line does not already load as native.
fn missing(line: &str, wanted: &[String]) -> Vec<String> {
    let current = assignment(line).map(|(_, v)| v).unwrap_or_default();
    let native: Vec<String> = current
        .split(';')
        .filter_map(|entry| entry.split_once('='))
        .filter(|(_, mode)| mode.trim_start().starts_with('n'))
        .flat_map(|(modules, _)| modules.split(',').map(|m| m.trim().to_lowercase()))
        .collect();
    wanted.iter().filter(|d| !native.contains(d)).cloned().collect()
}

/// `winhttp=n,b`: what Steam's launch options still need, if anything.
fn value_to_add(line: &str, wanted: &[String]) -> Option<String> {
    let missing = missing(line, wanted);
    (!missing.is_empty()).then(|| merge_dll_overrides(None, &missing))
}

/// The line with the overrides in it: the variable joins the one already
/// there, or goes in front of the command (`%command%` is added to a line
/// that only has arguments).
fn with_overrides(line: &str, add: &[String]) -> String {
    let line = line.trim();
    if let Some(((s, e), old)) = assignment(line) {
        let merged = merge_dll_overrides(Some(&old), add);
        return format!("{}{VAR}=\"{merged}\"{}", &line[..s], &line[e..]);
    }
    let value = merge_dll_overrides(None, add);
    let command = if line.contains(options::COMMAND) {
        line.to_string()
    } else if line.is_empty() {
        options::COMMAND.to_string()
    } else {
        format!("{} {line}", options::COMMAND)
    };
    format!("{VAR}=\"{value}\" {command}")
}

/// The value to add to the Steam game's launch options, read from Steam.
/// Blocking (the client may take a moment).
pub(super) fn missing_overrides(paths: &DataPaths, app_id: &str, list: &ModList) -> Option<String> {
    let wanted = wanted(list);
    if wanted.is_empty() {
        return None;
    }
    let snapshot = crate::services::steam_library::steam(paths).read(app_id).ok()?;
    value_to_add(&snapshot.launch_options, &wanted)
}

impl ModsService {
    pub(super) fn write_steam_overrides(&mut self, game_key: &str, cx: &mut Context<Self>) {
        let (Some(game), Some(shared)) = (self.games.get(game_key), self.shared.get(game_key).cloned()) else { return };
        let Some(app_id) = game.info.steam_app_id.clone().filter(|_| game.info.through_steam) else { return };
        let (paths, key, title) = (self.data_paths.clone(), game_key.to_string(), game.info.title.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { add_to_steam(&paths, &app_id, &shared.list(), &title) }).await;
            this.update(cx, |s, cx| {
                let (message, error) = match result {
                    Ok(message) => (message, false),
                    Err(message) => (message, true),
                };
                cx.emit(ModsEvent::Notice { game_key: key.clone(), message, error });
                s.recheck(&key, cx);
            })
            .ok();
        })
        .detach();
    }
}

/// Changes the launch options in Steam. The error says what to paste instead.
fn add_to_steam(paths: &DataPaths, app_id: &str, list: &ModList, title: &str) -> Result<String, String> {
    let wanted = wanted(list);
    let steam = crate::services::steam_library::steam(paths);
    let snapshot = steam.read(app_id)?;
    let line = snapshot.launch_options;
    let Some(value) = value_to_add(&line, &wanted) else {
        return Ok(tr!("The launch options of {game} in Steam already have what the mods need", game = title).to_string());
    };
    let add = missing(&line, &wanted);
    let new_line = with_overrides(&line, &add);
    let paste = tr!("In Steam, open the game's Properties and put this in the launch options: {line}", line = new_line);
    if !options::parse(&line).editable {
        return Err(tr!(
            "The launch options of {game} use shell syntax, which Heronite does not change. {paste}",
            game = title,
            paste = paste
        ));
    }
    match steam.apply(app_id, &Change { launch_options: Some(new_line), compat_tool: None }) {
        Ok(Access::Client) => Ok(tr!("Added {value} to the launch options of {game} in Steam", value = value, game = title)),
        Ok(_) => Ok(tr!(
            "Added {value} to the launch options of {game} in Steam; Steam reads them the next time it starts",
            value = value,
            game = title
        )),
        Err(e) => Err(tr!("Could not change the launch options in Steam: {error} {paste}", error = e, paste = paste)),
    }
}

#[cfg(test)]
mod tests {
    use super::{assignment, missing, value_to_add, wanted, with_overrides};
    use heronite_mods::{ModEntry, ModList};

    fn dlls(names: &[&str]) -> Vec<String> {
        names.iter().map(|d| d.to_string()).collect()
    }

    #[test]
    fn the_variable_is_read_with_or_without_quotes() {
        let line = "FOO=1 WINEDLLOVERRIDES=\"winhttp=n,b;d3d9=b\" gamemoderun %command% -x";
        assert_eq!(assignment(line).map(|(_, v)| v).as_deref(), Some("winhttp=n,b;d3d9=b"));
        assert_eq!(assignment("WINEDLLOVERRIDES=version=n %command%").map(|(_, v)| v).as_deref(), Some("version=n"));
        assert_eq!(assignment("MYWINEDLLOVERRIDES=x %command%"), None);
        assert_eq!(assignment(""), None);
    }

    #[test]
    fn only_dlls_not_loaded_as_native_are_missing() {
        let want = dlls(&["winhttp", "version"]);
        assert_eq!(missing("", &want), want);
        assert_eq!(missing("WINEDLLOVERRIDES=\"winhttp=n,b\" %command%", &want), dlls(&["version"]));
        assert_eq!(missing("WINEDLLOVERRIDES=\"winhttp,version=n,b\" %command%", &want), Vec::<String>::new());
        assert_eq!(missing("WINEDLLOVERRIDES=\"winhttp=b\" %command%", &want), want);
        assert_eq!(value_to_add("", &want).as_deref(), Some("winhttp=n,b;version=n,b"));
        assert_eq!(value_to_add("WINEDLLOVERRIDES=\"winhttp,version=n,b\" %command%", &want), None);
    }

    #[test]
    fn the_variable_goes_in_front_of_the_command() {
        let add = dlls(&["winhttp"]);
        assert_eq!(with_overrides("", &add), "WINEDLLOVERRIDES=\"winhttp=n,b\" %command%");
        assert_eq!(
            with_overrides("gamemoderun %command% -windowed", &add),
            "WINEDLLOVERRIDES=\"winhttp=n,b\" gamemoderun %command% -windowed"
        );
        assert_eq!(with_overrides("-windowed", &add), "WINEDLLOVERRIDES=\"winhttp=n,b\" %command% -windowed");
    }

    #[test]
    fn an_existing_variable_is_joined_not_repeated() {
        let add = dlls(&["winhttp"]);
        let line = with_overrides("FOO=1 WINEDLLOVERRIDES=\"d3d9=b;winhttp=b\" %command%", &add);
        assert_eq!(line, "FOO=1 WINEDLLOVERRIDES=\"d3d9=b;winhttp=n,b\" %command%");
        assert_eq!(missing(&line, &add), Vec::<String>::new());
    }

    #[test]
    fn mods_that_are_off_need_nothing() {
        let mut list = ModList::new("g", "G");
        list.mods.push(ModEntry { id: "a".into(), enabled: true, dll_overrides: dlls(&["Winhttp"]), ..Default::default() });
        list.mods.push(ModEntry { id: "b".into(), enabled: false, dll_overrides: dlls(&["version"]), ..Default::default() });
        assert_eq!(wanted(&list), dlls(&["winhttp"]));
        list.active = false;
        assert!(wanted(&list).is_empty());
    }
}
