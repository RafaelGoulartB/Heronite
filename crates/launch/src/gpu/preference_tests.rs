use serde_json::json;

use super::*;
use crate::plan::{Handoff, ProcessPlan};

const GAME: &str = r"C:\Games\Hades\Hades.exe";

fn ledger() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("gpu-preferences.json");
    (dir, file)
}

#[test]
fn ids_round_trip_and_unknown_ones_are_the_systems_choice() {
    for p in Preference::ALL {
        assert_eq!(Preference::parse(p.id()), p);
    }
    assert_eq!(Preference::parse("fastest"), Preference::System);
    assert_eq!(Preference::System.value(), None);
    assert_eq!(Preference::PowerSaving.value(), Some(1));
    assert_eq!(Preference::HighPerformance.value(), Some(2));
    let g = Gpu::from_map(json!({ KEY_PREFERENCE: "high-performance" }).as_object().unwrap());
    assert_eq!(g.preference, Preference::HighPerformance);
}

#[test]
fn only_the_gpu_field_changes_and_the_others_stay_in_order() {
    assert_eq!(with_preference(None, 2), "GpuPreference=2;");
    assert_eq!(with_preference(Some(""), 1), "GpuPreference=1;");
    assert_eq!(with_preference(Some("GpuPreference=0;"), 2), "GpuPreference=2;");
    assert_eq!(
        with_preference(Some("AutoHDREnable=0;SwapEffectUpgradeEnable=1;"), 2),
        "AutoHDREnable=0;SwapEffectUpgradeEnable=1;GpuPreference=2;"
    );
    assert_eq!(
        with_preference(Some("SwapEffectUpgradeEnable=1;gpupreference=1;AutoHDREnable=1"), 2),
        "SwapEffectUpgradeEnable=1;GpuPreference=2;AutoHDREnable=1;"
    );
    assert_eq!(preference_of("AutoHDREnable=1;GpuPreference=2;"), Some(2));
    assert_eq!(preference_of("AutoHDREnable=1;"), None);
}

#[test]
fn only_full_paths_to_programs_name_a_value() {
    assert_eq!(target_exe(GAME).as_deref(), Some(GAME));
    assert_eq!(target_exe("C:/Games/Hades/Hades.EXE").as_deref(), Some(r"C:\Games\Hades\Hades.EXE"));
    assert_eq!(target_exe(r#""D:\A B\game.exe""#).as_deref(), Some(r"D:\A B\game.exe"));
    assert_eq!(target_exe(r"\\nas\games\x.exe").as_deref(), Some(r"\\nas\games\x.exe"));
    assert_eq!(target_exe("game.exe"), None);
    assert_eq!(target_exe(r"C:\Games\start.bat"), None);
    assert_eq!(target_exe("explorer.exe"), None);
    assert_eq!(target_exe("steam://rungameid/1"), None);
    assert_eq!(target_exe(r"C:\Games\..\Windows\x.exe"), None);
}

#[test]
fn a_new_value_is_removed_after_the_game() {
    let (_d, file) = ledger();
    let reg = FakeRegistry::default();
    let applied = apply(&reg, &file, GAME, Preference::HighPerformance).unwrap();
    assert_eq!(applied, Applied::Wrote { data: "GpuPreference=2;".into(), previous: None });
    assert_eq!(reg.get(GAME).as_deref(), Some("GpuPreference=2;"));
    assert_eq!(restore(&reg, &file, GAME).unwrap(), Restored::Removed);
    assert!(reg.is_empty());
    // Nothing left to do a second time.
    assert_eq!(restore(&reg, &file, GAME).unwrap(), Restored::Nothing);
}

#[test]
fn a_value_that_was_there_is_put_back_with_its_other_fields() {
    let (_d, file) = ledger();
    let before = "AutoHDREnable=1;GpuPreference=1;";
    let reg = FakeRegistry::default().with(GAME, before);
    apply(&reg, &file, GAME, Preference::HighPerformance).unwrap();
    assert_eq!(reg.get(GAME).as_deref(), Some("AutoHDREnable=1;GpuPreference=2;"));
    assert_eq!(restore(&reg, &file, GAME).unwrap(), Restored::PutBack);
    assert_eq!(reg.get(GAME).as_deref(), Some(before));
}

#[test]
fn a_change_made_in_windows_settings_meanwhile_is_never_overwritten() {
    let (_d, file) = ledger();
    let reg = FakeRegistry::default();
    apply(&reg, &file, GAME, Preference::HighPerformance).unwrap();
    // The user picks "Power saving" in Settings while the game runs.
    reg.set(GAME, "GpuPreference=1;");
    assert_eq!(restore(&reg, &file, GAME).unwrap(), Restored::LeftAlone);
    assert_eq!(reg.get(GAME).as_deref(), Some("GpuPreference=1;"));
    // And Heronite has forgotten it: the next run starts from the user's value.
    assert_eq!(restore(&reg, &file, GAME).unwrap(), Restored::Nothing);
}

#[test]
fn the_same_choice_already_there_writes_nothing() {
    let (_d, file) = ledger();
    let reg = FakeRegistry { read_only: true, ..Default::default() }.with(GAME, "GpuPreference=2;");
    assert_eq!(apply(&reg, &file, GAME, Preference::HighPerformance).unwrap(), Applied::AlreadySet);
    assert_eq!(restore(&reg, &file, GAME).unwrap(), Restored::Nothing);
    assert_eq!(reg.get(GAME).as_deref(), Some("GpuPreference=2;"));
}

#[test]
fn the_systems_choice_writes_nothing() {
    let (_d, file) = ledger();
    let reg = FakeRegistry::default();
    assert_eq!(apply(&reg, &file, GAME, Preference::System).unwrap(), Applied::AlreadySet);
    assert!(reg.is_empty() && !file.exists());
}

#[test]
fn a_refused_write_leaves_nothing_to_put_back() {
    let (_d, file) = ledger();
    let reg = FakeRegistry { read_only: true, ..Default::default() };
    assert!(apply(&reg, &file, GAME, Preference::HighPerformance).is_err());
    assert_eq!(restore(&reg, &file, GAME).unwrap(), Restored::Nothing);
}

#[test]
fn a_session_that_crashed_still_puts_back_what_was_there_first() {
    let (_d, file) = ledger();
    let reg = FakeRegistry::default().with(GAME, "GpuPreference=1;");
    apply(&reg, &file, GAME, Preference::HighPerformance).unwrap();
    // The app died with the game: the next launch must not take Heronite's
    // own value for the user's.
    let again = apply(&reg, &file, GAME, Preference::HighPerformance).unwrap();
    assert_eq!(again, Applied::Wrote { data: "GpuPreference=2;".into(), previous: Some("GpuPreference=1;".into()) });
    assert_eq!(restore(&reg, &file, GAME).unwrap(), Restored::PutBack);
    assert_eq!(reg.get(GAME).as_deref(), Some("GpuPreference=1;"));
}

#[test]
fn the_app_starting_puts_back_what_ended_without_it_but_not_running_games() {
    let (_d, file) = ledger();
    let other = r"D:\Games\Celeste\Celeste.exe";
    let reg = FakeRegistry::default().with(other, "AutoHDREnable=1;");
    apply(&reg, &file, GAME, Preference::HighPerformance).unwrap();
    apply(&reg, &file, other, Preference::PowerSaving).unwrap();
    let done = restore_all_except(&reg, &file, &|exe| exe.eq_ignore_ascii_case(GAME));
    assert_eq!(done, vec![(other.to_string(), Restored::PutBack)]);
    assert_eq!(reg.get(other).as_deref(), Some("AutoHDREnable=1;"));
    // The running game keeps its value until it ends.
    assert_eq!(reg.get(GAME).as_deref(), Some("GpuPreference=2;"));
    assert_eq!(restore(&reg, &file, GAME).unwrap(), Restored::Removed);
}

#[test]
fn names_differing_only_in_case_are_the_same_value() {
    let (_d, file) = ledger();
    let reg = FakeRegistry::default().with(&GAME.to_uppercase(), "GpuPreference=1;");
    apply(&reg, &file, GAME, Preference::HighPerformance).unwrap();
    assert_eq!(reg.len(), 1);
    assert_eq!(restore(&reg, &file, &GAME.to_lowercase()).unwrap(), Restored::PutBack);
    assert_eq!(reg.get(GAME).as_deref(), Some("GpuPreference=1;"));
}

#[test]
fn only_programs_heronite_starts_itself_get_the_preference() {
    let g = Gpu { preference: Preference::HighPerformance, ..Default::default() };
    let mut plan = LaunchPlan::Native(ProcessPlan::default());
    attach_on(&mut plan, &g, true);
    assert_eq!(plan.process().unwrap().gpu_preference, Some(Preference::HighPerformance));
    // Not on Linux, where the GPU tab picks the card through variables.
    let mut linux = LaunchPlan::Native(ProcessPlan::default());
    attach_on(&mut linux, &g, false);
    assert_eq!(linux.process().unwrap().gpu_preference, None);
    // A launcher that hands the game on: its program is not the game's.
    let mut handed = LaunchPlan::Native(ProcessPlan { handoff: Some(Handoff::default()), ..Default::default() });
    attach_on(&mut handed, &g, true);
    assert_eq!(handed.process().unwrap().gpu_preference, None);
    // Epic games: legendary names the program, which gets it.
    let mut epic = LaunchPlan::Legendary { process: ProcessPlan::default(), app_name: "x".into() };
    attach_on(&mut epic, &g, true);
    assert_eq!(epic.process().unwrap().gpu_preference, Some(Preference::HighPerformance));
    // An emulator runs every game of its system.
    let mut emulator = LaunchPlan::Emulator { process: ProcessPlan::default(), emulator: "x".into(), rom: None };
    attach_on(&mut emulator, &g, true);
    assert_eq!(emulator.process().unwrap().gpu_preference, None);
    // Steam starts its games itself.
    let mut steam = LaunchPlan::Unavailable(String::new());
    attach_on(&mut steam, &g, true);
    assert_eq!(steam, LaunchPlan::Unavailable(String::new()));
    // Windows' own choice: nothing to do.
    let mut system = LaunchPlan::Native(ProcessPlan::default());
    attach_on(&mut system, &Gpu::default(), true);
    assert_eq!(system.process().unwrap().gpu_preference, None);
}
