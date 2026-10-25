use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

const TOKEN: &str = "AbCdEfGhIj";
const MACHINE: &str = "0123456789abcdef0123456789abcdef\n";
const NOW: u64 = 1_700_000_000;

fn login(hardcore: bool) -> Login<'static> {
    Login { username: "Rafa", token: TOKEN, hardcore }
}

fn machine() -> Option<String> {
    Some(MACHINE.to_string())
}

fn sign_in(home: &Path, only: &[Emulator], hardcore: bool) -> Vec<Status> {
    run(home, only, Some(&login(hardcore)), &|_| false, NOW, &machine)
}

fn sign_out(home: &Path, only: &[Emulator]) -> Vec<Status> {
    run(home, only, None, &|_| false, NOW, &machine)
}

fn held(home: &Path, name: &str) -> Vec<Status> {
    check(home, Some(name), &machine)
}

fn state_of(statuses: &[Status], emulator: Emulator) -> State {
    let mut of = statuses.iter().filter(|s| s.emulator == emulator);
    let first = of.next().expect("a status for the emulator");
    assert!(of.all(|s| s.state == first.state));
    first.state.clone()
}

fn put(home: &Path, rel: &str, text: &str) -> PathBuf {
    let path = home.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    path
}

/// The lines of `before` that are not one of `ours` keys, in order, still
/// appear in `after` in the same order.
fn assert_others_kept(before: &str, after: &str, ours: &[&str]) {
    let is_ours = |l: &&str| ours.iter().any(|k| l.trim_start().starts_with(k));
    let kept: Vec<&str> = before.lines().filter(|l| !is_ours(l)).collect();
    let mut rest = after.lines().filter(|l| !is_ours(l));
    for line in kept {
        assert!(rest.any(|l| l == line), "line lost or moved: {line:?}");
    }
}

const RETROARCH: &str = "audio_driver = \"pulse\"\ncheevos_enable = \"false\"\ncheevos_username = \"\"\ncheevos_password = \"\"\ncheevos_token = \"\"\nvideo_fullscreen = \"true\"\n";
const DUCKSTATION: &str = "[Main]\nSettingsVersion = 3\n\n[Cheevos]\nEnabled = false\n\n[Display]\nAspectRatio = Auto\n";
const PCSX2: &str = "[UI]\nSettingsVersion = 1\n\n[Achievements]\nEnabled = false\nOverlays = true\n\n[Folders]\nBios = bios\n";
const PPSSPP: &str = "[General]\nFirstRun = False\n\n[Achievements]\nAchievementsEnable = False\nAchievementsHost = \n\n[Graphics]\nCardboardScreenSize = 50\n";
const DOLPHIN: &str = "[Achievements]\nHostUrl = \nEnabled = False\n; keep\nProgressEnabled = True\n";
const FLYCAST: &str = "[config]\nrend.Resolution = 480\n\n[achievements]\nEnabled = no\n\n[network]\nEnable = no\n";

struct Case {
    emulator: Emulator,
    rel: &'static str,
    sample: &'static str,
    ours: &'static [&'static str],
    signed_in: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            emulator: Emulator::RetroArch,
            rel: ".config/retroarch/retroarch.cfg",
            sample: RETROARCH,
            ours: &["cheevos_"],
            signed_in: &[
                "cheevos_enable = \"true\"",
                "cheevos_hardcore_mode_enable = \"true\"",
                "cheevos_username = \"Rafa\"",
                "cheevos_password = \"\"",
                "cheevos_token = \"AbCdEfGhIj\"",
            ],
        },
        Case {
            emulator: Emulator::Pcsx2,
            rel: ".config/PCSX2/inis/PCSX2.ini",
            sample: PCSX2,
            ours: &["Enabled", "ChallengeMode", "Username", "Token", "LoginTimestamp"],
            signed_in: &["Enabled = true", "ChallengeMode = true", "Username = Rafa", "LoginTimestamp = 1700000000", "Token = AbCdEfGhIj"],
        },
        Case {
            emulator: Emulator::Ppsspp,
            rel: ".config/ppsspp/PSP/SYSTEM/ppsspp.ini",
            sample: PPSSPP,
            ours: &["AchievementsEnable =", "AchievementsChallengeMode", "AchievementsUserName"],
            signed_in: &["AchievementsEnable = True", "AchievementsChallengeMode = True", "AchievementsUserName = Rafa"],
        },
        Case {
            emulator: Emulator::Dolphin,
            rel: ".config/dolphin-emu/RetroAchievements.ini",
            sample: DOLPHIN,
            ours: &["Enabled", "HardcoreEnabled", "Username", "ApiToken"],
            signed_in: &["Enabled = True", "HardcoreEnabled = True", "Username = Rafa", "ApiToken = AbCdEfGhIj"],
        },
        Case {
            emulator: Emulator::Flycast,
            rel: ".config/flycast/emu.cfg",
            sample: FLYCAST,
            ours: &["Enabled", "HardcoreMode", "UserName", "Token"],
            signed_in: &["Enabled = yes", "HardcoreMode = yes", "UserName = Rafa", "Token = AbCdEfGhIj"],
        },
    ]
}

#[test]
fn every_emulator_round_trips_and_keeps_the_other_lines() {
    for case in cases() {
        let home = tempfile::tempdir().unwrap();
        let path = put(home.path(), case.rel, case.sample);

        let statuses = sign_in(home.path(), &[case.emulator], true);
        assert_eq!(statuses, vec![Status { emulator: case.emulator, config: Some(path.clone()), state: State::SignedIn }]);
        let text = fs::read_to_string(&path).unwrap();
        for line in case.signed_in {
            assert!(text.lines().any(|l| l == *line), "{:?}: missing {line:?} in\n{text}", case.emulator);
        }
        let others = case.ours;
        assert_others_kept(case.sample, &text, others);
        assert_eq!(state_of(&held(home.path(), "rafa"), case.emulator), State::SignedIn, "{:?}", case.emulator);
        assert_eq!(state_of(&held(home.path(), "someone"), case.emulator), State::SignedOut);
        assert_eq!(state_of(&check(home.path(), None, &machine), case.emulator), State::SignedOut);

        let statuses = sign_out(home.path(), &[case.emulator]);
        assert_eq!(statuses[0].state, State::SignedOut);
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains(TOKEN) && !text.contains("Rafa"), "{:?}:\n{text}", case.emulator);
        assert_others_kept(case.sample, &text, others);
        assert_eq!(state_of(&held(home.path(), "rafa"), case.emulator), State::SignedOut);
    }
}

#[test]
fn hardcore_follows_the_login() {
    let home = tempfile::tempdir().unwrap();
    let path = put(home.path(), ".config/flycast/emu.cfg", FLYCAST);
    sign_in(home.path(), &[Emulator::Flycast], false);
    assert!(fs::read_to_string(path).unwrap().contains("HardcoreMode = no"));
}

#[test]
fn sign_out_turns_achievements_off_and_empties_the_login() {
    let home = tempfile::tempdir().unwrap();
    let path = put(home.path(), ".config/retroarch/retroarch.cfg", RETROARCH);
    sign_in(home.path(), &[Emulator::RetroArch], true);
    sign_out(home.path(), &[Emulator::RetroArch]);
    let text = fs::read_to_string(path).unwrap();
    for line in ["cheevos_enable = \"false\"", "cheevos_username = \"\"", "cheevos_token = \"\""] {
        assert!(text.lines().any(|l| l == line), "{line}");
    }
}

#[test]
fn signing_out_a_file_without_a_login_does_not_touch_it() {
    let home = tempfile::tempdir().unwrap();
    let path = put(home.path(), ".config/PCSX2/inis/PCSX2.ini", "[UI]\r\nA = 1\r\n");
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let statuses = sign_out(home.path(), &[Emulator::Pcsx2]);
    assert_eq!(statuses[0].state, State::SignedOut);
    assert_eq!(fs::read_to_string(&path).unwrap(), "[UI]\r\nA = 1\r\n");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
}

#[test]
fn a_missing_config_is_not_set_up_and_is_not_created() {
    let home = tempfile::tempdir().unwrap();
    let statuses = sign_in(home.path(), &Emulator::ALL, true);
    assert_eq!(statuses.len(), 6);
    assert!(statuses.iter().all(|s| s.config.is_none() && s.state == State::NotSetUp));
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
    assert!(held(home.path(), "rafa").iter().all(|s| s.state == State::NotSetUp));
}

#[test]
fn a_running_emulator_is_left_alone() {
    let home = tempfile::tempdir().unwrap();
    let native = put(home.path(), ".config/dolphin-emu/RetroAchievements.ini", DOLPHIN);
    let flatpak = put(home.path(), ".var/app/org.DolphinEmu.dolphin-emu/config/dolphin-emu/RetroAchievements.ini", DOLPHIN);
    let running = |e: Emulator| e == Emulator::Dolphin;
    let statuses = run(home.path(), &Emulator::ALL, Some(&login(true)), &running, NOW, &machine);
    assert_eq!(state_of(&statuses, Emulator::Dolphin), State::Running);
    assert_eq!(statuses.iter().filter(|s| s.emulator == Emulator::Dolphin).count(), 2);
    assert_eq!(fs::read_to_string(native).unwrap(), DOLPHIN);
    assert_eq!(fs::read_to_string(flatpak).unwrap(), DOLPHIN);
    let out = run(home.path(), &Emulator::ALL, None, &running, NOW, &machine);
    assert_eq!(state_of(&out, Emulator::Dolphin), State::Running);
}

#[test]
fn native_and_flatpak_are_both_written() {
    let home = tempfile::tempdir().unwrap();
    let native = put(home.path(), ".local/share/duckstation/settings.ini", DUCKSTATION);
    let flatpak = put(home.path(), ".var/app/org.duckstation.DuckStation/config/duckstation/settings.ini", DUCKSTATION);
    let statuses = sign_in(home.path(), &[Emulator::DuckStation], true);
    assert_eq!(statuses.len(), 2);
    assert!(statuses.iter().all(|s| s.state == State::SignedIn));
    for path in [native, flatpak] {
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("[Cheevos]\nEnabled = true\n"), "{text}");
        assert!(text.contains("LoginTimestamp = 1700000000"));
        assert!(text.contains("[Display]\nAspectRatio = Auto\n"));
    }
    assert!(held(home.path(), "Rafa").iter().filter(|s| s.emulator == Emulator::DuckStation).all(|s| s.state == State::SignedIn));
}

#[test]
fn duckstation_keeps_the_token_encrypted_for_this_pc() {
    let home = tempfile::tempdir().unwrap();
    let path = put(home.path(), ".local/share/duckstation/settings.ini", DUCKSTATION);
    sign_in(home.path(), &[Emulator::DuckStation], true);
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains(TOKEN));
    let stored = Ini::parse(&text).get("Cheevos", "Token").unwrap();
    assert_eq!(secret::decrypt(&stored, "Rafa", MACHINE).as_deref(), Some(TOKEN));
    // Another PC's login is not ours.
    let other = check(home.path(), Some("Rafa"), &|| Some("another pc".to_string()));
    assert_eq!(state_of(&other, Emulator::DuckStation), State::SignedOut);
    // Without a machine id there is nothing to encrypt with.
    let statuses = run(home.path(), &[Emulator::DuckStation], Some(&login(true)), &|_| false, NOW, &|| None);
    assert!(matches!(statuses[0].state, State::Failed(_)));
}

use ini::Ini;

#[test]
fn ppsspp_keeps_the_token_in_its_own_file() {
    let home = tempfile::tempdir().unwrap();
    let path = put(home.path(), ".config/ppsspp/PSP/SYSTEM/ppsspp.ini", PPSSPP);
    let token_file = path.with_file_name("ppsspp_retroachievements.dat");
    sign_in(home.path(), &[Emulator::Ppsspp], true);
    assert_eq!(fs::read_to_string(&token_file).unwrap(), TOKEN);
    assert_eq!(fs::metadata(&token_file).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!fs::read_to_string(&path).unwrap().contains(TOKEN));
    assert_eq!(state_of(&held(home.path(), "rafa"), Emulator::Ppsspp), State::SignedIn);
    sign_out(home.path(), &[Emulator::Ppsspp]);
    assert!(!token_file.exists());
    assert_eq!(state_of(&held(home.path(), "rafa"), Emulator::Ppsspp), State::SignedOut);
}

#[test]
fn pcsx2_uses_secrets_ini_when_it_exists() {
    let home = tempfile::tempdir().unwrap();
    let ini = put(home.path(), ".config/PCSX2/inis/PCSX2.ini", PCSX2);
    let secrets = put(home.path(), ".config/PCSX2/inis/secrets.ini", "[Achievements]\nToken = oldtoken\n");
    sign_in(home.path(), &[Emulator::Pcsx2], true);
    assert!(!fs::read_to_string(&ini).unwrap().contains(TOKEN));
    assert_eq!(fs::read_to_string(&secrets).unwrap(), "[Achievements]\nToken = AbCdEfGhIj\n");
    assert_eq!(state_of(&held(home.path(), "rafa"), Emulator::Pcsx2), State::SignedIn);
    sign_out(home.path(), &[Emulator::Pcsx2]);
    assert_eq!(fs::read_to_string(&secrets).unwrap(), "[Achievements]\nToken = \n");
    assert_eq!(state_of(&held(home.path(), "rafa"), Emulator::Pcsx2), State::SignedOut);
}

#[test]
fn pcsx2_without_secrets_ini_writes_the_token_in_the_ini() {
    let home = tempfile::tempdir().unwrap();
    let ini = put(home.path(), ".config/PCSX2/inis/PCSX2.ini", PCSX2);
    sign_in(home.path(), &[Emulator::Pcsx2], true);
    assert!(fs::read_to_string(&ini).unwrap().contains("Token = AbCdEfGhIj"));
    assert!(!ini.with_file_name("secrets.ini").exists());
}

#[test]
fn only_the_listed_emulators_are_touched() {
    let home = tempfile::tempdir().unwrap();
    let flycast = put(home.path(), ".config/flycast/emu.cfg", FLYCAST);
    let dolphin = put(home.path(), ".config/dolphin-emu/RetroAchievements.ini", DOLPHIN);
    sign_in(home.path(), &Emulator::ALL, true);
    let statuses = sign_out(home.path(), &[Emulator::Flycast]);
    assert_eq!(statuses.len(), 1);
    assert!(!fs::read_to_string(flycast).unwrap().contains("Rafa"));
    assert!(fs::read_to_string(dolphin).unwrap().contains("Username = Rafa"));
}

#[test]
fn another_account_is_not_ours() {
    let home = tempfile::tempdir().unwrap();
    put(home.path(), ".config/flycast/emu.cfg", "[achievements]\nEnabled = yes\nUserName = Other\nToken = zzz\n");
    assert_eq!(state_of(&held(home.path(), "Rafa"), Emulator::Flycast), State::SignedOut);
    assert_eq!(state_of(&held(home.path(), "other"), Emulator::Flycast), State::SignedIn);
}

#[test]
fn a_login_with_achievements_off_or_no_token_is_not_signed_in() {
    let home = tempfile::tempdir().unwrap();
    let path = put(home.path(), ".config/flycast/emu.cfg", "[achievements]\nEnabled = no\nUserName = Rafa\nToken = zzz\n");
    assert_eq!(state_of(&held(home.path(), "Rafa"), Emulator::Flycast), State::SignedOut);
    fs::write(&path, "[achievements]\nEnabled = yes\nUserName = Rafa\nToken =\n").unwrap();
    assert_eq!(state_of(&held(home.path(), "Rafa"), Emulator::Flycast), State::SignedOut);
}

#[test]
fn permissions_and_line_endings_are_kept() {
    let home = tempfile::tempdir().unwrap();
    let path = put(home.path(), ".config/PCSX2/inis/PCSX2.ini", "[Achievements]\r\nEnabled = false\r\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    sign_in(home.path(), &[Emulator::Pcsx2], false);
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.ends_with("\r\n") && !text.replace("\r\n", "").contains('\n'), "{text:?}");
}

#[test]
fn a_symlinked_config_stays_a_symlink() {
    let home = tempfile::tempdir().unwrap();
    let real = put(home.path(), "dotfiles/emu.cfg", FLYCAST);
    let link = home.path().join(".config/flycast/emu.cfg");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    sign_in(home.path(), &[Emulator::Flycast], true);
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert!(fs::read_to_string(&real).unwrap().contains("UserName = Rafa"));
}

#[test]
fn the_public_functions_find_nothing_in_an_empty_home() {
    let home = tempfile::tempdir().unwrap();
    assert!(apply(home.path(), Some(&login(true))).iter().all(|s| s.state == State::NotSetUp));
    assert!(apply_to(home.path(), &[Emulator::Pcsx2], None).iter().all(|s| s.state == State::NotSetUp));
    assert!(status(home.path(), Some("Rafa")).iter().all(|s| s.state == State::NotSetUp));
}

#[test]
fn names_are_the_ones_users_know() {
    let names: Vec<&str> = Emulator::ALL.iter().map(|e| e.name()).collect();
    assert_eq!(names, ["RetroArch", "DuckStation", "PCSX2", "PPSSPP", "Dolphin", "Flycast"]);
}
