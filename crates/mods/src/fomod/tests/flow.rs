//! The wizard: defaults, group rules, flags, visibility and the final copies.

use super::*;
use crate::{FomodChoices, FomodGroupChoice, FomodStepChoice};

fn plugin(name: &str, ty: &str, flags: &[(&str, &str)], files: &str) -> String {
    let flags: String = flags.iter().map(|(n, v)| format!(r#"<flag name="{n}">{v}</flag>"#)).collect();
    format!(
        r#"<plugin name="{name}"><description>about {name}</description><conditionFlags>{flags}</conditionFlags><files>{files}</files>
            <typeDescriptor><type name="{ty}"/></typeDescriptor></plugin>"#
    )
}

/// A plugin whose type is `default` unless `when` holds, then `then`.
fn dynamic(name: &str, default: &str, when: &str, then: &str, flags: &[(&str, &str)]) -> String {
    let flags: String = flags.iter().map(|(n, v)| format!(r#"<flag name="{n}">{v}</flag>"#)).collect();
    format!(
        r#"<plugin name="{name}"><conditionFlags>{flags}</conditionFlags><typeDescriptor><dependencyType><defaultType name="{default}"/>
            <patterns><pattern><dependencies>{when}</dependencies><type name="{then}"/></pattern></patterns></dependencyType></typeDescriptor></plugin>"#
    )
}

fn file(source: &str) -> String {
    format!(r#"<file source="{source}"/>"#)
}

fn group(name: &str, kind: &str, plugins: &[String]) -> String {
    format!(r#"<group name="{name}" type="{kind}"><plugins order="Explicit">{}</plugins></group>"#, plugins.concat())
}

fn step(name: &str, visible: &str, groups: &[String]) -> String {
    let visible = if visible.is_empty() { String::new() } else { format!("<visible>{visible}</visible>") };
    format!(
        r#"<installStep name="{name}">{visible}<optionalFileGroups order="Explicit">{}</optionalFileGroups></installStep>"#,
        groups.concat()
    )
}

fn config(before: &str, steps: &[String], after: &str) -> String {
    format!(r#"<config><moduleName>T</moduleName>{before}<installSteps order="Explicit">{}</installSteps>{after}</config>"#, steps.concat())
}

fn flag_is(flag: &str, value: &str) -> String {
    format!(r#"<flagDependency flag="{flag}" value="{value}"/>"#)
}

fn file_is(file: &str, state: &str) -> String {
    format!(r#"<fileDependency file="{file}" state="{state}"/>"#)
}

#[test]
fn skyui_like_single_step() {
    let xml = config(
        r#"<requiredInstallFiles><file source="SkyUI_SE.esp"/><folder source="interface"/></requiredInstallFiles>"#,
        &[step(
            "Options",
            "",
            &[group(
                "Config",
                "SelectExactlyOne",
                &[plugin("Default", "Recommended", &[], &file("def.ini")), plugin("Alt", "Optional", &[], &file("alt.ini"))],
            )],
        )],
        "",
    );
    let mut w = wizard(&xml);
    let view = w.step();
    assert_eq!(view.name, "Options");
    assert_eq!(view.groups[0].name, "Config");
    assert_eq!(view.groups[0].kind, GroupKind::SelectExactlyOne);
    assert_eq!(view.groups[0].plugins[0].state, PluginType::Recommended);
    assert_eq!(view.groups[0].plugins[0].description, "about Default");
    assert_eq!(picked(&w), vec![vec![true, false]]);
    assert!(w.can_next() && !w.has_next() && !w.has_back());
    assert!(!w.next());
    assert_eq!(w.step_index(), (1, 1));
    assert_eq!(
        copies(&w),
        vec![
            ("SkyUI_SE.esp".into(), "SkyUI_SE.esp".into()),
            ("interface".into(), "interface".into()),
            ("def.ini".into(), "def.ini".into())
        ]
    );

    w.select(0, 1, true);
    assert_eq!(picked(&w), vec![vec![false, true]]);
    assert_eq!(copies(&w).last().unwrap().0, "alt.ini");
    // A radio group cannot be cleared by clicking its pick.
    w.select(0, 1, false);
    assert_eq!(picked(&w), vec![vec![false, true]]);
    assert!(w.module_dependencies_met());
}

#[test]
fn group_rules() {
    let xml = config(
        "",
        &[step(
            "S",
            "",
            &[
                group(
                    "exactly",
                    "SelectExactlyOne",
                    &[plugin("e1", "NotUsable", &[], ""), plugin("e2", "Optional", &[], ""), plugin("e3", "Optional", &[], "")],
                ),
                group("atmost", "SelectAtMostOne", &[plugin("m1", "Optional", &[], ""), plugin("m2", "Optional", &[], "")]),
                group("atleast", "SelectAtLeastOne", &[plugin("l1", "Optional", &[], ""), plugin("l2", "Optional", &[], "")]),
                group("all", "SelectAll", &[plugin("a1", "Optional", &[], ""), plugin("a2", "Optional", &[], "")]),
                group("any", "SelectAny", &[plugin("n1", "Optional", &[], ""), plugin("n2", "Optional", &[], "")]),
            ],
        )],
        "",
    );
    let mut w = wizard(&xml);
    // Exactly-one takes the first usable one, at-most/at-least/any start empty, all is on.
    assert_eq!(picked(&w), vec![vec![false, true, false], vec![false, false], vec![false, false], vec![true, true], vec![false, false]]);
    assert!(!w.can_next(), "at-least-one is empty");

    w.select(1, 0, true);
    w.select(1, 1, true);
    assert_eq!(picked(&w)[1], vec![false, true]);
    w.select(1, 1, false);
    assert_eq!(picked(&w)[1], vec![false, false], "at-most-one can be cleared");

    w.select(2, 0, true);
    assert!(w.can_next());
    w.select(2, 1, true);
    w.select(2, 0, false);
    assert_eq!(picked(&w)[2], vec![false, true]);
    w.select(2, 1, false);
    assert_eq!(picked(&w)[2], vec![false, true], "the last one of at-least-one stays");

    w.select(3, 0, false);
    assert_eq!(picked(&w)[3], vec![true, true], "select-all is locked");
    assert!(w.step().groups[3].plugins.iter().all(|p| !p.enabled));

    w.select(4, 0, true);
    w.select(4, 1, true);
    assert_eq!(picked(&w)[4], vec![true, true]);
    w.select(4, 0, false);
    assert_eq!(picked(&w)[4], vec![false, true]);

    // Out-of-range indexes are ignored.
    w.select(9, 0, true);
    w.select(0, 9, true);
}

#[test]
fn required_and_not_usable_are_locked() {
    let xml = config(
        "",
        &[step(
            "S",
            "",
            &[group(
                "G",
                "SelectAny",
                &[
                    plugin("req", "Required", &[], ""),
                    plugin("rec", "Recommended", &[], ""),
                    plugin("no", "NotUsable", &[], ""),
                    plugin("could", "CouldBeUsable", &[], ""),
                ],
            )],
        )],
        "",
    );
    let mut w = wizard(&xml);
    assert_eq!(picked(&w), vec![vec![true, true, false, false]]);
    let enabled: Vec<bool> = w.step().groups[0].plugins.iter().map(|p| p.enabled).collect();
    assert_eq!(enabled, vec![false, true, false, true]);
    w.select(0, 0, false);
    w.select(0, 2, true);
    assert_eq!(picked(&w), vec![vec![true, true, false, false]]);
    w.select(0, 3, true);
    w.select(0, 1, false);
    assert_eq!(picked(&w), vec![vec![true, false, false, true]]);
}

#[test]
fn a_required_option_wins_a_radio_group() {
    let xml = config(
        "",
        &[step("S", "", &[group("G", "SelectExactlyOne", &[plugin("a", "Recommended", &[], ""), plugin("b", "Required", &[], "")])])],
        "",
    );
    let mut w = wizard(&xml);
    assert_eq!(picked(&w), vec![vec![false, true]]);
    w.select(0, 0, true);
    assert_eq!(picked(&w), vec![vec![false, true]]);
}

#[test]
fn flags_show_and_hide_later_steps() {
    let xml = config(
        "",
        &[
            step(
                "Mode",
                "",
                &[group(
                    "Mode",
                    "SelectExactlyOne",
                    &[
                        plugin("Simple", "Recommended", &[("mode", "simple")], &file("simple.esp")),
                        plugin("Advanced", "Optional", &[("mode", "advanced")], ""),
                    ],
                )],
            ),
            step(
                "Tweaks",
                &flag_is("mode", "advanced"),
                &[group("Tweaks", "SelectAny", &[plugin("Hard", "Optional", &[("hard", "yes")], &file("hard.esp"))])],
            ),
            step("Finish", "", &[group("Finish", "SelectAny", &[plugin("Readme", "Recommended", &[], &file("readme.txt"))])]),
        ],
        "",
    );
    let mut w = wizard(&xml);
    assert_eq!(w.step().name, "Mode");
    assert_eq!(w.flags().get("mode").map(String::as_str), Some("simple"));
    assert!(w.has_next());
    assert_eq!(w.step_index(), (1, 2));

    assert!(w.next());
    assert_eq!(w.step().name, "Finish");
    assert_eq!(w.step_index(), (2, 2));
    assert!(!w.has_next());
    assert_eq!(copies(&w), vec![("simple.esp".into(), "simple.esp".into()), ("readme.txt".into(), "readme.txt".into())]);
    assert!(w.back() && !w.back());

    w.select(0, 1, true);
    assert_eq!(w.flags().get("mode").map(String::as_str), Some("advanced"), "the radio swapped the flag");
    assert_eq!(w.step_index(), (1, 3));
    assert!(w.next());
    assert_eq!(w.step().name, "Tweaks");
    w.select(0, 0, true);
    assert_eq!(w.flags().get("hard").map(String::as_str), Some("yes"));
    assert_eq!(w.step_index(), (2, 3));
    assert!(w.next());
    assert_eq!(w.step().name, "Finish");
    assert_eq!(copies(&w), vec![("hard.esp".into(), "hard.esp".into()), ("readme.txt".into(), "readme.txt".into())]);
    assert!(!w.next());
}

#[test]
fn going_back_unsets_the_flags_of_later_steps() {
    let xml = config(
        "",
        &[
            step("One", "", &[group("G", "SelectAny", &[plugin("a", "Recommended", &[("one", "1")], "")])]),
            step("Two", "", &[group("G", "SelectAny", &[plugin("b", "Recommended", &[("two", "2")], &file("b.esp"))])]),
        ],
        "",
    );
    let mut w = wizard(&xml);
    assert!(w.next());
    assert_eq!(w.flags().len(), 2);
    assert_eq!(copies(&w).len(), 1);
    assert!(w.back());
    assert_eq!(w.flags().keys().collect::<Vec<_>>(), vec!["one"]);
    assert!(copies(&w).is_empty(), "files of a step left are not installed");
    assert_eq!(w.choices().steps.len(), 1);
    // The picks of the step left are kept for when the user comes back.
    assert!(w.next());
    assert_eq!(picked(&w), vec![vec![true]]);
    assert_eq!(w.flags().len(), 2);
}

#[test]
fn a_hidden_step_is_shown_when_a_flag_is_cleared_by_going_back() {
    let xml = config(
        "",
        &[
            step("Pick", "", &[group("G", "SelectAtMostOne", &[plugin("a", "Optional", &[("x", "1")], "")])]),
            step("Extra", &flag_is("x", "1"), &[group("G", "SelectAny", &[plugin("b", "Optional", &[], "")])]),
        ],
        "",
    );
    let mut w = wizard(&xml);
    assert!(!w.has_next());
    w.select(0, 0, true);
    assert!(w.has_next());
    w.select(0, 0, false);
    assert!(!w.has_next());
}

#[test]
fn types_follow_the_flags_of_earlier_steps() {
    let xml = config(
        "",
        &[
            step("Hardware", "", &[group("GPU", "SelectAtMostOne", &[plugin("High", "Optional", &[("gpu", "high")], "")])]),
            step(
                "Textures",
                "",
                &[group(
                    "Res",
                    "SelectExactlyOne",
                    &[
                        dynamic("4K", "Optional", &flag_is("gpu", "high"), "Recommended", &[]),
                        dynamic("2K", "Recommended", &flag_is("gpu", "high"), "Optional", &[]),
                    ],
                )],
            ),
        ],
        "",
    );
    let mut w = wizard(&xml);
    assert!(w.next());
    let types: Vec<PluginType> = w.step().groups[0].plugins.iter().map(|p| p.state).collect();
    assert_eq!(types, vec![PluginType::Optional, PluginType::Recommended]);
    assert_eq!(picked(&w), vec![vec![false, true]]);

    // Back, set the flag: the same step now suggests the other one, but the
    // pick the user left is kept, only the types move.
    assert!(w.back());
    w.select(0, 0, true);
    assert!(w.next());
    let types: Vec<PluginType> = w.step().groups[0].plugins.iter().map(|p| p.state).collect();
    assert_eq!(types, vec![PluginType::Recommended, PluginType::Optional]);
}

#[test]
fn types_follow_the_flags_of_the_same_step() {
    let xml = config(
        "",
        &[step(
            "S",
            "",
            &[
                group("A", "SelectAny", &[plugin("lib", "Optional", &[("lib", "on")], "")]),
                group("B", "SelectAny", &[dynamic("needs lib", "NotUsable", &flag_is("lib", "on"), "Required", &[("dep", "1")])]),
            ],
        )],
        "",
    );
    let mut w = wizard(&xml);
    assert_eq!(w.step().groups[1].plugins[0].state, PluginType::NotUsable);
    assert_eq!(picked(&w)[1], vec![false]);
    w.select(0, 0, true);
    assert_eq!(w.step().groups[1].plugins[0].state, PluginType::Required);
    assert_eq!(picked(&w)[1], vec![true]);
    assert_eq!(w.flags().get("dep").map(String::as_str), Some("1"), "the forced pick sets its flags");
    w.select(0, 0, false);
    assert_eq!(picked(&w)[1], vec![false], "not usable again: off");
    assert!(w.flags().get("dep").is_none());
}

#[test]
fn types_follow_the_state_of_files() {
    let xml = config(
        "",
        &[step(
            "S",
            "",
            &[group(
                "G",
                "SelectAny",
                &[
                    dynamic("needs skse", "NotUsable", &file_is("skse64_loader.exe", "Active"), "Recommended", &[]),
                    dynamic("no patch", "Recommended", &file_is("Patch.esp", "Active"), "NotUsable", &[]),
                ],
            )],
        )],
        "",
    );
    let both = wizard_with(&xml, &[("skse64_loader.exe", FileState::Active), ("patch.esp", FileState::Active)], None);
    assert_eq!(picked(&both), vec![vec![true, false]]);
    let none = wizard_with(&xml, &[], None);
    assert_eq!(picked(&none), vec![vec![false, true]]);
    let inactive = wizard_with(&xml, &[("skse64_loader.exe", FileState::Inactive)], None);
    assert_eq!(picked(&inactive), vec![vec![false, true]], "installed but off is not active");
}

#[test]
fn file_dependencies_in_visible_use_the_lookup_once_per_file() {
    let xml = config("", &[step("S", &file_is("Mod.esp", "Active"), &[group("G", "SelectAny", &[plugin("p", "Optional", &[], "")])])], "");
    let (_d, installer) = load_xml(&xml);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let w = Wizard::new(
        installer,
        Box::new(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            FileState::Active
        }),
        None,
    );
    for _ in 0..5 {
        let _ = (w.step(), w.has_next(), w.step_index());
    }
    assert_eq!(w.step().name, "S");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn a_step_with_unmet_visibility_at_the_start_is_skipped() {
    let xml = config(
        "",
        &[
            step("Hidden", &flag_is("never", "1"), &[group("G", "SelectAny", &[plugin("p", "Optional", &[], "")])]),
            step("Shown", "", &[group("G", "SelectAny", &[plugin("q", "Optional", &[], "")])]),
        ],
        "",
    );
    let w = wizard(&xml);
    assert_eq!(w.step().name, "Shown");
    assert_eq!(w.step_index(), (1, 1));
}

#[test]
fn an_installer_without_steps_only_has_required_files() {
    let w = wizard(&config(r#"<requiredInstallFiles><file source="a.esp"/></requiredInstallFiles>"#, &[], ""));
    assert_eq!(w.step(), StepView::default());
    assert!(w.can_next() && !w.has_next() && !w.has_back());
    assert_eq!(w.step_index(), (0, 0));
    assert_eq!(copies(&w), vec![("a.esp".into(), "a.esp".into())]);
}

#[test]
fn conditional_installs_follow_the_final_flags() {
    let pattern = |when: &str, source: &str| {
        format!(r#"<pattern><dependencies operator="And">{when}</dependencies><files>{}</files></pattern>"#, file(source))
    };
    let after = format!(
        "<conditionalFileInstalls><patterns>{}{}{}</patterns></conditionalFileInstalls>",
        pattern(&flag_is("a", "1"), "with_a.esp"),
        pattern(&flag_is("b", "1"), "with_b.esp"),
        pattern(&format!("{}{}", flag_is("a", "1"), file_is("Base.esm", "Active")), "a_and_base.esp"),
    );
    let xml = config(
        "",
        &[step(
            "S",
            "",
            &[group(
                "G",
                "SelectAny",
                &[plugin("A", "Recommended", &[("a", "1")], &file("a.esp")), plugin("B", "Optional", &[("b", "1")], "")],
            )],
        )],
        &after,
    );
    let mut w = wizard_with(&xml, &[("Base.esm", FileState::Active)], None);
    let sources: Vec<String> = w.files().into_iter().map(|r| r.source).collect();
    assert_eq!(sources, vec!["a.esp", "with_a.esp", "a_and_base.esp"]);
    w.select(0, 0, false);
    w.select(0, 1, true);
    let sources: Vec<String> = w.files().into_iter().map(|r| r.source).collect();
    assert_eq!(sources, vec!["with_b.esp"]);

    let without_base = wizard(&xml);
    let sources: Vec<String> = without_base.files().into_iter().map(|r| r.source).collect();
    assert_eq!(sources, vec!["a.esp", "with_a.esp"]);
}

#[test]
fn results_are_ordered_by_priority_and_then_by_position() {
    let prio = |source: &str, p: i32| format!(r#"<file source="{source}" priority="{p}"/>"#);
    let xml = config(
        &format!(r#"<requiredInstallFiles>{}{}</requiredInstallFiles>"#, prio("req_hi", 10), prio("req_lo", 0)),
        &[step(
            "S",
            "",
            &[group(
                "G",
                "SelectAny",
                &[
                    plugin("A", "Recommended", &[], &format!("{}{}", prio("a_mid", 5), prio("a_low", -1))),
                    plugin("B", "Recommended", &[], &prio("b_zero", 0)),
                ],
            )],
        )],
        &format!(
            "<conditionalFileInstalls><patterns><pattern><dependencies/><files>{}</files></pattern></patterns></conditionalFileInstalls>",
            prio("cond_zero", 0)
        ),
    );
    let w = wizard(&xml);
    let order: Vec<(String, i64)> = w.files().into_iter().map(|r| (r.source, r.priority)).collect();
    let order: Vec<&str> = order.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(order, vec!["a_low", "req_lo", "b_zero", "cond_zero", "a_mid", "req_hi"]);
}

#[test]
fn always_and_if_usable_files_install_without_the_pick() {
    let files = r#"<file source="plain"/><file source="always" alwaysInstall="true"/><file source="usable" installIfUsable="true"/>"#;
    let xml = config(
        "",
        &[step("S", "", &[group("G", "SelectAny", &[plugin("off", "Optional", &[], files), plugin("blocked", "NotUsable", &[], files)])])],
        "",
    );
    let mut w = wizard(&xml);
    let sources: Vec<String> = w.files().into_iter().map(|r| r.source).collect();
    assert_eq!(sources, vec!["always", "usable", "always"]);
    w.select(0, 0, true);
    let sources: Vec<String> = w.files().into_iter().map(|r| r.source).collect();
    assert_eq!(sources, vec!["plain", "always", "usable", "always"]);
}

#[test]
fn files_of_steps_never_shown_are_left_out() {
    let xml = config(
        "",
        &[
            step("One", "", &[group("G", "SelectAny", &[plugin("a", "Recommended", &[], &file("one.esp"))])]),
            step(
                "Two",
                &flag_is("never", "1"),
                &[group(
                    "G",
                    "SelectAny",
                    &[plugin("b", "Recommended", &[], &format!("{}<file source=\"always\" alwaysInstall=\"true\"/>", file("two.esp")))],
                )],
            ),
        ],
        "",
    );
    let w = wizard(&xml);
    let sources: Vec<String> = w.files().into_iter().map(|r| r.source).collect();
    assert_eq!(sources, vec!["one.esp"]);
}

#[test]
fn choices_are_named_and_preselect_a_new_wizard() {
    let xml = config(
        "",
        &[
            step(
                "Look",
                "",
                &[
                    group(
                        "Style",
                        "SelectExactlyOne",
                        &[plugin("Plain", "Recommended", &[], ""), plugin("Fancy", "Optional", &[("fancy", "1")], "")],
                    ),
                    group("Extras", "SelectAny", &[plugin("Glow", "Optional", &[], ""), plugin("Sparks", "Recommended", &[], "")]),
                ],
            ),
            step(
                "Fancy",
                &flag_is("fancy", "1"),
                &[group("Colour", "SelectExactlyOne", &[plugin("Red", "Recommended", &[], ""), plugin("Blue", "Optional", &[], "")])],
            ),
        ],
        "",
    );
    let mut w = wizard(&xml);
    w.select(0, 1, true);
    w.select(1, 0, true);
    w.select(1, 1, false);
    assert!(w.next());
    w.select(0, 1, true);
    let saved = w.choices();
    assert_eq!(
        saved,
        FomodChoices {
            steps: vec![
                FomodStepChoice {
                    step: "Look".into(),
                    groups: vec![
                        FomodGroupChoice { group: "Style".into(), plugins: vec!["Fancy".into()] },
                        FomodGroupChoice { group: "Extras".into(), plugins: vec!["Glow".into()] }
                    ],
                },
                FomodStepChoice {
                    step: "Fancy".into(),
                    groups: vec![FomodGroupChoice { group: "Colour".into(), plugins: vec!["Blue".into()] }]
                },
            ]
        }
    );

    let mut again = wizard_with(&xml, &[], Some(&saved));
    assert_eq!(picked(&again), vec![vec![false, true], vec![true, false]]);
    assert!(again.next());
    assert_eq!(picked(&again), vec![vec![false, true]]);
    assert_eq!(again.choices(), saved);
}

#[test]
fn previous_choices_fall_back_to_defaults_where_names_do_not_match() {
    let xml = config(
        "",
        &[
            step(
                "Look",
                "",
                &[
                    group("Style", "SelectExactlyOne", &[plugin("Plain", "Recommended", &[], ""), plugin("Fancy", "Optional", &[], "")]),
                    group("New group", "SelectAny", &[plugin("Default on", "Recommended", &[], "")]),
                ],
            ),
            step("Brand new", "", &[group("G", "SelectExactlyOne", &[plugin("x", "Optional", &[], ""), plugin("y", "Optional", &[], "")])]),
        ],
        "",
    );
    let previous = FomodChoices {
        steps: vec![FomodStepChoice {
            step: "Look".into(),
            groups: vec![FomodGroupChoice { group: "Style".into(), plugins: vec!["Gone".into()] }],
        }],
    };
    let mut w = wizard_with(&xml, &[], Some(&previous));
    // "Gone" matches nothing in an exactly-one group: the first usable stands in.
    assert_eq!(picked(&w), vec![vec![true, false], vec![true]]);
    assert!(w.next());
    assert_eq!(picked(&w), vec![vec![true, false]]);

    let fancy = FomodChoices {
        steps: vec![FomodStepChoice {
            step: "Look".into(),
            groups: vec![FomodGroupChoice { group: "Style".into(), plugins: vec!["Fancy".into()] }],
        }],
    };
    let w = wizard_with(&xml, &[], Some(&fancy));
    assert_eq!(picked(&w), vec![vec![false, true], vec![true]]);
}

#[test]
fn previous_choices_cannot_break_locks() {
    let xml = config(
        "",
        &[step(
            "S",
            "",
            &[group(
                "G",
                "SelectAny",
                &[plugin("req", "Required", &[], ""), plugin("no", "NotUsable", &[], ""), plugin("opt", "Optional", &[], "")],
            )],
        )],
        "",
    );
    let previous = FomodChoices {
        steps: vec![FomodStepChoice { step: "S".into(), groups: vec![FomodGroupChoice { group: "G".into(), plugins: vec!["no".into()] }] }],
    };
    let w = wizard_with(&xml, &[], Some(&previous));
    assert_eq!(picked(&w), vec![vec![true, false, false]]);
}

#[test]
fn unmet_module_dependencies_are_explained() {
    let xml = config(
        r#"<moduleDependencies operator="And"><fileDependency file="Skyrim.esm" state="Active"/><fileDependency file="SKSE\Plugins\x.dll" state="Active"/><gameDependency version="1.6"/></moduleDependencies>"#,
        &[],
        "",
    );
    let ok = wizard_with(&xml, &[("Skyrim.esm", FileState::Active), ("SKSE/Plugins/x.dll", FileState::Active)], None);
    assert!(ok.module_dependencies_met() && ok.missing_module_dependencies().is_empty());
    let missing = wizard_with(&xml, &[("Skyrim.esm", FileState::Active)], None);
    assert!(!missing.module_dependencies_met());
    assert_eq!(missing.missing_module_dependencies(), vec!["SKSE/Plugins/x.dll is not installed"]);
}

#[test]
fn or_dependencies_in_visibility() {
    let when = format!(r#"<dependencies operator="Or">{}{}</dependencies>"#, flag_is("a", "1"), file_is("X.esp", "Active"));
    let xml = config("", &[step("S", &when, &[group("G", "SelectAny", &[plugin("p", "Optional", &[], "")])])], "");
    assert_eq!(wizard_with(&xml, &[("X.esp", FileState::Active)], None).step().name, "S");
    assert_eq!(wizard(&xml).step().name, "");
}
