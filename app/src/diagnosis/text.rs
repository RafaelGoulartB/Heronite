//! What the launch diagnosis says, in the interface language.

use heronite_i18n::{tr, trn};
use heronite_launch::diagnose::{AntiCheatStatus, Finding, Fix, Problem, RunKind, MAX_MAP_COUNT};

use super::Report;

/// A winetricks component as players know it (product names stay as they are).
pub fn component(verb: &str) -> String {
    let name = match verb {
        "vcrun2022" => "Visual C++ 2015-2022",
        "vcrun2013" => "Visual C++ 2013",
        "vcrun2012" => "Visual C++ 2012",
        "vcrun2010" => "Visual C++ 2010",
        "vcrun2008" => "Visual C++ 2008",
        "vcrun2005" => "Visual C++ 2005",
        "vcrun2003" => "Visual C++ 2003",
        "mfc42" => "MFC 4.2",
        "vb6run" => "Visual Basic 6",
        "d3dx9" => "DirectX 9 (d3dx9)",
        "d3dx10" => "DirectX 10 (d3dx10)",
        "d3dx11_43" => "DirectX 11 (d3dx11)",
        "d3dcompiler_43" | "d3dcompiler_47" => "D3DCompiler",
        "xact" => "XAudio (XACT)",
        "physx" => "PhysX",
        "openal" => "OpenAL",
        "xna40" => "XNA 4.0",
        "dotnet48" => ".NET Framework 4.8",
        other => other,
    };
    name.to_string()
}

fn components(verbs: &[String]) -> String {
    verbs.iter().map(|v| component(v)).collect::<Vec<_>>().join(", ")
}

fn anticheats(names: &[String]) -> String {
    names.join(" + ")
}

/// The problem in a few words: the dialog's heading and what the toast and
/// the notification say after the game's name.
pub fn headline(f: &Finding) -> String {
    match &f.problem {
        Problem::MissingDlls { verbs, .. } if !verbs.is_empty() => {
            trn!(verbs.len(), "{names} is missing", "{names} are missing", names = components(verbs))
        }
        Problem::MissingDlls { dlls, .. } => tr!("{file} is missing", file = dlls.first().cloned().unwrap_or_default()),
        Problem::DotNet => tr!(".NET Framework is missing").into(),
        Problem::Dx12WithoutVkd3d => tr!("DirectX 12 needs VKD3D-Proton").into(),
        Problem::Direct3dWithoutDxvk => tr!("Direct3D failed without DXVK").into(),
        Problem::AntiCheat { names, status } => match status {
            AntiCheatStatus::Works => tr!("{anticheat} needs its runtime", anticheat = anticheats(names)),
            AntiCheatStatus::Broken => tr!("{anticheat} does not work on Linux yet", anticheat = anticheats(names)),
            AntiCheatStatus::Denied => tr!("{anticheat} blocks Linux players", anticheat = anticheats(names)),
        },
        Problem::Vulkan => tr!("Vulkan could not start").into(),
        Problem::TooManyOpenFiles => tr!("Wine ran out of open files").into(),
        Problem::MaxMapCount { .. } => tr!("The memory map limit is low").into(),
        Problem::MissingLibs { bits32: true, .. } => tr!("32-bit libraries are missing").into(),
        Problem::MissingLibs { .. } => tr!("Linux libraries are missing").into(),
        Problem::ExecutableMissing => tr!("The game's program is missing").into(),
        Problem::Wineserver => tr!("Another Wine still runs on the prefix").into(),
        Problem::Crash if f.sure => tr!("The game crashed").into(),
        Problem::Crash => tr!("The game closed with an error").into(),
        Problem::LogsOff => tr!("The launch log is off").into(),
    }
}

/// What happened and why the fix helps, in a sentence or two.
pub fn explanation(f: &Finding, r: &Report) -> String {
    match &f.problem {
        Problem::MissingDlls { verbs, .. } if !verbs.is_empty() => tr!(
            "Windows games expect these runtimes to be installed, and this game's prefix does not have them yet. Heronite installs them with winetricks, in this prefix only."
        )
        .into(),
        Problem::MissingDlls { dlls, .. } => tr!(
            "Wine could not find {files}. No runtime brings it: it comes with the game, so its files may be incomplete.",
            files = dlls.join(", ")
        ),
        Problem::DotNet => tr!(
            "The game runs on Microsoft .NET, and Wine Mono, Wine's own replacement, could not start it. Installing .NET Framework 4.8 takes a few minutes."
        )
        .into(),
        Problem::Dx12WithoutVkd3d => tr!(
            "The game draws with DirectX 12, which plain Wine runs through VKD3D-Proton. It is off for this game; Proton has it built in."
        )
        .into(),
        Problem::Direct3dWithoutDxvk => {
            tr!("Wine's own Direct3D failed for this game. DXVK runs DirectX 9, 10 and 11 through Vulkan and avoids most of these failures.")
                .into()
        }
        Problem::AntiCheat { names, status } => {
            let anticheat = anticheats(names);
            match status {
                AntiCheatStatus::Works if r.kind == RunKind::Wine => tr!(
                    "This game uses {anticheat}, which works on Linux with Proton and its anti-cheat runtime. The game runs with plain Wine now.",
                    anticheat
                ),
                AntiCheatStatus::Works => tr!(
                    "This game uses {anticheat}, which works on Linux through Proton's anti-cheat runtime. The runtime is off for this game.",
                    anticheat
                ),
                AntiCheatStatus::Broken => tr!(
                    "AreWeAntiCheatYet lists the game as broken: {anticheat} does not run under Proton for now. No setting changes that.",
                    anticheat
                ),
                AntiCheatStatus::Denied => tr!(
                    "The developer set {anticheat} to refuse Linux, so the game cannot run here whatever the settings. AreWeAntiCheatYet lists it as denied.",
                    anticheat
                ),
            }
        }
        Problem::Vulkan => tr!(
            "Proton, DXVK and VKD3D-Proton draw through Vulkan, and the graphics driver did not answer. The Vulkan driver, often its 32-bit part, is missing."
        )
        .into(),
        Problem::TooManyOpenFiles => {
            tr!("With esync, Wine needs more open files than this system allows. fsync does the same work without them.").into()
        }
        Problem::MaxMapCount { current } => tr!(
            "vm.max_map_count is {current} here, and many games (and Easy Anti-Cheat) need {needed}, which SteamOS, Fedora, Ubuntu and Arch use. Raising it asks for the administrator password.",
            current,
            needed = MAX_MAP_COUNT
        ),
        Problem::MissingLibs { libs, bits32 } => match (libs.is_empty(), bits32) {
            (true, _) => tr!("This Wine needs its 32-bit part, which is not installed. Proton brings its own.").into(),
            (false, true) => {
                tr!("The game needs the 32-bit version of {libs}, and only the 64-bit one is installed.", libs = libs.join(", "))
            }
            (false, false) => tr!("The game needs {libs}, which is not installed.", libs = libs.join(", ")),
        },
        Problem::ExecutableMissing => tr!("Wine could not find the program to start: the game's files may be incomplete or moved.").into(),
        Problem::Wineserver => tr!(
            "A wineserver of another Wine or Proton version still runs in this game's prefix, and the new one cannot start next to it."
        )
        .into(),
        Problem::Crash if f.sure => {
            tr!("Wine reported a crash, with no known cause in the log. Another Proton version often helps, and ProtonDB has what worked for other players.").into()
        }
        Problem::Crash => tr!(
            "The log shows no known cause. Another Proton version often helps, and ProtonDB has what worked for other players."
        )
        .into(),
        Problem::LogsOff => tr!("With the log off there is nothing to read the cause from. Turn it on and start the game again.").into(),
    }
}

/// The settings a [`Fix::Settings`] turns on, as one label.
fn settings_label(changes: &[(&'static str, serde_json::Value)]) -> &'static str {
    let has = |k: &str| changes.iter().any(|(key, _)| *key == k);
    if has("autoInstallVkd3d") {
        tr!("Turn on VKD3D-Proton and play")
    } else if has("autoInstallDxvk") {
        tr!("Turn on DXVK and play")
    } else if has("eacRuntime") || has("battlEyeRuntime") {
        tr!("Turn on the anti-cheat runtime and play")
    } else if has("enableEsync") {
        tr!("Use fsync and play")
    } else if has("verboseLogs") {
        tr!("Turn on the log and play")
    } else {
        tr!("Change it and play")
    }
}

/// The button that applies `fix`.
pub fn fix_label(fix: &Fix, r: &Report) -> String {
    let proton = |p: &Option<String>| p.clone().unwrap_or_else(|| "Proton".into());
    match fix {
        Fix::Winetricks(verbs) => tr!("Install {names} and play", names = components(verbs)),
        Fix::Settings(changes) => settings_label(changes).into(),
        Fix::UseProton(_) => tr!("Use {proton} and play", proton = proton(&r.best_proton)),
        Fix::OtherProton => tr!("Try {proton}", proton = proton(&r.other_proton)),
        Fix::StopWineserver => tr!("Stop it and play").into(),
        Fix::RaiseMaxMapCount => tr!("Raise it and play").into(),
        Fix::Terminal(_) => tr!("Copy the command").into(),
        Fix::Repair => tr!("Verify the files").into(),
    }
}

/// What the dialog says while `fix` runs.
pub fn working(fix: &Fix) -> String {
    match fix {
        Fix::Winetricks(verbs) => tr!("Installing {verb}\u{2026}", verb = components(verbs)),
        Fix::RaiseMaxMapCount => tr!("Waiting for the administrator password\u{2026}").into(),
        Fix::StopWineserver => tr!("Ending the wineserver of the prefix\u{2026}").into(),
        _ => tr!("Changing the game's settings\u{2026}").into(),
    }
}

/// Under a [`Fix::Terminal`]: what the command does.
pub fn terminal_note(f: &Finding) -> &'static str {
    match f.problem {
        Problem::Vulkan => tr!("Installs the Vulkan driver for 64- and 32-bit programs. Run it in a terminal, then play again."),
        _ => tr!("Finds the packages that have these files. Install the one it names, then play again."),
    }
}

/// The toast once `fix` worked and the game starts again.
pub fn fixed(fix: &Fix, r: &Report) -> String {
    let title = r.title.as_str();
    match fix {
        Fix::Winetricks(verbs) => tr!("{names} installed. Starting {title}\u{2026}", names = components(verbs), title),
        Fix::UseProton(_) | Fix::OtherProton => {
            let proton = if matches!(fix, Fix::OtherProton) { &r.other_proton } else { &r.best_proton };
            tr!("{title} now runs with {proton}. Starting it\u{2026}", title, proton = proton.clone().unwrap_or_default())
        }
        Fix::RaiseMaxMapCount => tr!("vm.max_map_count raised. Starting {title}\u{2026}", title),
        Fix::StopWineserver => tr!("The old wineserver stopped. Starting {title}\u{2026}", title),
        _ => tr!("Settings changed. Starting {title}\u{2026}", title),
    }
}

/// What the toast or the notification says after a launch.
pub fn summary(r: &Report) -> String {
    let title = r.title.as_str();
    match r.main() {
        Some(f) => tr!("{title} closed right after starting: {cause}", title, cause = headline(f)),
        None => tr!("{title} closed right after starting", title),
    }
}

/// How the run ended, for the dialog's first line.
pub fn how_it_ended(r: &Report) -> String {
    let title = r.title.as_str();
    let Some(ended) = r.ended else { return tr!("From the last launch of {title}.", title) };
    let seconds = ended.elapsed.as_secs().max(1);
    match ended.exit.map(|e| (e.code, e.signal)) {
        Some((Some(code), _)) if code != 0 => {
            trn!(
                seconds,
                "{title} closed {n} second after starting (exit code {code}).",
                "{title} closed {n} seconds after starting (exit code {code}).",
                title,
                code
            )
        }
        Some((None, Some(signal))) => {
            trn!(
                seconds,
                "{title} crashed {n} second after starting (signal {signal}).",
                "{title} crashed {n} seconds after starting (signal {signal}).",
                title,
                signal
            )
        }
        _ => trn!(seconds, "{title} closed {n} second after starting.", "{title} closed {n} seconds after starting.", title),
    }
}

/// The note on a fix that was applied already this run.
pub fn tried_note() -> &'static str {
    tr!("Heronite already tried this, and the game still closes.")
}
