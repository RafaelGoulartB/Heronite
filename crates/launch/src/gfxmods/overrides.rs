//! `WINEDLLOVERRIDES` lists (`a,b=n,b;c=d`): under Wine and Proton a game
//! prefers a DLL in its folder over Wine's own only when the list says
//! `<dll>=n,b`. Mods add their DLLs at launch and take them out of the
//! user's own list when they are removed; the user's entries always win.

/// The DLL names of an override list, lowercase, without `.dll`, with the
/// entry each belongs to.
fn override_names(value: &str) -> Vec<(String, usize)> {
    value
        .split(';')
        .enumerate()
        .flat_map(|(i, entry)| {
            let names = entry.split_once('=').map_or(entry, |(n, _)| n);
            names.split(',').map(move |n| (n.trim().to_ascii_lowercase().trim_end_matches(".dll").to_string(), i))
        })
        .filter(|(n, _)| !n.is_empty())
        .collect()
}

fn stem(dll: &str) -> String {
    dll.trim().to_ascii_lowercase().trim_end_matches(".dll").to_string()
}

/// `current` with `<dll>=n,b` added for each of `dlls` it does not name.
/// Gives the names the user already sets: their value is kept.
pub fn merge_overrides(current: &str, dlls: &[&str]) -> (String, Vec<String>) {
    let named: Vec<String> = override_names(current).into_iter().map(|(n, _)| n).collect();
    let mut entries: Vec<String> = current.split(';').map(str::trim).filter(|e| !e.is_empty()).map(str::to_string).collect();
    let mut kept = Vec::new();
    for dll in dlls {
        let s = stem(dll);
        if named.contains(&s) {
            kept.push(s);
        } else if !entries.iter().any(|e| e == &format!("{s}=n,b")) {
            entries.push(format!("{s}=n,b"));
        }
    }
    (entries.join(";"), kept)
}

/// `value` without the names in `dlls` (an entry left with no name goes).
pub fn strip_overrides(value: &str, dlls: &[&str]) -> String {
    let drop: Vec<String> = dlls.iter().map(|d| stem(d)).collect();
    value
        .split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            let (names, val) = match entry.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (entry, None),
            };
            let left: Vec<&str> = names.split(',').map(str::trim).filter(|n| !n.is_empty() && !drop.contains(&stem(n))).collect();
            match (left.is_empty(), val) {
                (true, _) => None,
                (false, Some(v)) => Some(format!("{}={v}", left.join(","))),
                (false, None) => Some(left.join(",")),
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The names of `dlls` that the user's `WINEDLLOVERRIDES` sets.
pub fn user_overrides(env_options: &[(String, String)], dlls: &[&str]) -> Vec<String> {
    let Some((_, v)) = env_options.iter().find(|(k, _)| k.eq_ignore_ascii_case("WINEDLLOVERRIDES")) else { return Vec::new() };
    let named: Vec<String> = override_names(v).into_iter().map(|(n, _)| n).collect();
    dlls.iter().map(|d| stem(d)).filter(|d| named.contains(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SP: &str = "ddraw,steam_api64,dxgi,d3dcompiler_47=n,b;lsteamclient=d";

    #[test]
    fn overrides_merge_without_duplicates_and_keep_the_users() {
        assert_eq!(merge_overrides("", &["dxgi.dll"]), ("dxgi=n,b".to_string(), vec![]));
        assert_eq!(
            merge_overrides("winemenubuilder.exe=d", &["dxgi.dll", "d3dcompiler_47.dll"]).0,
            "winemenubuilder.exe=d;dxgi=n,b;d3dcompiler_47=n,b"
        );
        let (sp, kept) = merge_overrides(SP, &["dxgi.dll", "d3dcompiler_47.dll"]);
        assert_eq!((sp.as_str(), kept), (SP, vec!["dxgi".to_string(), "d3dcompiler_47".to_string()]));
        assert_eq!(merge_overrides("dxgi=n,b", &["dxgi.dll"]).0, "dxgi=n,b");
        assert_eq!(merge_overrides("a=n;", &["d3d9.dll"]).0, "a=n;d3d9=n,b");
    }

    #[test]
    fn the_users_overrides_can_be_cleaned_up() {
        assert_eq!(strip_overrides(SP, &["dxgi.dll", "d3dcompiler_47.dll"]), "ddraw,steam_api64=n,b;lsteamclient=d");
        assert_eq!(strip_overrides("dxgi=n,b", &["dxgi.dll"]), "");
        let env = vec![("WINEDLLOVERRIDES".to_string(), SP.to_string())];
        assert_eq!(user_overrides(&env, &["dxgi.dll", "d3d9.dll"]), ["dxgi"]);
        assert!(user_overrides(&[], &["dxgi.dll"]).is_empty());
    }
}
