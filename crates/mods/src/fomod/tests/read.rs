//! Finding, decoding and parsing installers.

use super::*;

const MINIMAL: &str = r#"<config><moduleName>Mini</moduleName></config>"#;

fn utf16(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if bom {
        out.extend(if big_endian { [0xFE, 0xFF] } else { [0xFF, 0xFE] });
    }
    for u in text.encode_utf16() {
        out.extend(if big_endian { u.to_be_bytes() } else { u.to_le_bytes() });
    }
    out
}

fn load_bytes(bytes: &[u8]) -> Result<Installer, String> {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "fomod/ModuleConfig.xml", bytes);
    load(dir.path())
}

#[test]
fn finds_the_fomod_folder_at_the_root_or_in_wrappers() {
    let dir = archive(MINIMAL, &[]);
    assert_eq!(find(dir.path()), Some(dir.path().to_path_buf()));

    let one = TempDir::new().unwrap();
    write(one.path(), "Wrapper/fomod/ModuleConfig.xml", MINIMAL.as_bytes());
    assert_eq!(find(one.path()), Some(one.path().join("Wrapper")));

    let two = TempDir::new().unwrap();
    write(two.path(), "a/b/fomod/ModuleConfig.xml", MINIMAL.as_bytes());
    write(two.path(), "readme.txt", b"hi");
    assert_eq!(find(two.path()), Some(two.path().join("a/b")));

    let three = TempDir::new().unwrap();
    write(three.path(), "a/b/c/fomod/ModuleConfig.xml", MINIMAL.as_bytes());
    assert_eq!(find(three.path()), None);
}

#[test]
fn does_not_pick_between_several_folders() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "one/fomod/ModuleConfig.xml", MINIMAL.as_bytes());
    write(dir.path(), "two/Data/x.esp", b"");
    assert_eq!(find(dir.path()), None);
}

#[test]
fn names_are_matched_ignoring_case() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "FOMod/MODULECONFIG.xml", MINIMAL.as_bytes());
    write(dir.path(), "FOMod/Info.XML", b"<fomod><name>From info</name></fomod>");
    assert_eq!(find(dir.path()), Some(dir.path().to_path_buf()));
    assert_eq!(load(dir.path()).unwrap().info.name, "From info");
}

#[test]
fn reads_utf16_with_and_without_bom() {
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<config><moduleName>Pé ão</moduleName></config>";
    for (be, bom) in [(false, true), (true, true), (false, false), (true, false)] {
        let installer = load_bytes(&utf16(xml, be, bom)).unwrap_or_else(|e| panic!("be={be} bom={bom}: {e}"));
        assert_eq!(installer.info.name, "Pé ão", "be={be} bom={bom}");
    }
}

#[test]
fn utf16_declared_as_utf8_still_reads() {
    let xml = "<?xml version=\"1.0\" encoding=\"utf-8\"?><config><moduleName>X</moduleName></config>";
    assert_eq!(load_bytes(&utf16(xml, false, true)).unwrap().info.name, "X");
}

#[test]
fn reads_utf8_with_bom_and_latin1() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend("<config><moduleName>Ação</moduleName></config>".as_bytes());
    assert_eq!(load_bytes(&bytes).unwrap().info.name, "Ação");

    let mut latin = b"<?xml version=\"1.0\" encoding=\"iso-8859-1\"?><config><moduleName>Caf".to_vec();
    latin.push(0xE9);
    latin.extend(b" \x93quoted\x94</moduleName></config>");
    assert_eq!(load_bytes(&latin).unwrap().info.name, "Café \u{201C}quoted\u{201D}");
}

#[test]
fn tolerates_junk_before_the_declaration_and_bare_ampersands() {
    let xml = "junk\n  <?xml version=\"1.0\"?><config><moduleName>Fire & Ice &amp; &#33; &bogus</moduleName></config>";
    assert_eq!(load_bytes(xml.as_bytes()).unwrap().info.name, "Fire & Ice & ! &bogus");
}

#[test]
fn namespaces_and_schema_attributes_are_ignored() {
    let xml = r#"<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
        xsi:noNamespaceSchemaLocation="http://qconsulting.ca/fo3/ModConfig5.0.xsd"><moduleName>NS</moduleName></config>"#;
    assert_eq!(load_bytes(xml.as_bytes()).unwrap().info.name, "NS");
}

#[test]
fn invalid_xml_is_an_error() {
    let err = load_bytes(b"<config><moduleName>x</config>").unwrap_err();
    assert!(err.contains("not valid"), "{err}");
}

#[test]
fn info_xml_fills_the_details() {
    let dir = archive(
        r#"<config><moduleName>Module name</moduleName><moduleImage path="fomod\Cover.PNG"/></config>"#,
        &[
            ("fomod/cover.png", b"png"),
            (
                "fomod/info.xml",
                br#"<fomod><Name>Nice Mod</Name><AUTHOR> Someone </AUTHOR><Version MachineVersion="1.2">1.2.0</Version>
                    <Website>https://example.com/mod</Website><description>Does things.</description></fomod>"#,
            ),
        ],
    );
    let info = load(dir.path()).unwrap().info;
    assert_eq!(
        (info.name.as_str(), info.author.as_str(), info.version.as_str(), info.website.as_str(), info.description.as_str()),
        ("Nice Mod", "Someone", "1.2.0", "https://example.com/mod", "Does things.")
    );
    assert_eq!(info.image, Some(dir.path().join("fomod/cover.png")));
}

#[test]
fn module_name_stands_in_without_info_xml() {
    assert_eq!(load_bytes(MINIMAL.as_bytes()).unwrap().info.name, "Mini");
}

#[test]
fn a_script_installer_is_refused_with_a_reason() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "fomod/script.cs", b"class Script {}");
    assert_eq!(find(dir.path()), None);
    assert_eq!(find_scripted(dir.path()), Some(dir.path().to_path_buf()));
    let err = load(dir.path()).unwrap_err();
    assert!(err.contains("scripted installer") && err.contains("script.cs"), "{err}");

    let none = TempDir::new().unwrap();
    assert!(load(none.path()).is_err());
}

#[test]
fn resolve_ignores_case_and_accepts_both_separators() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "Textures/Armor/Body.DDS", b"x");
    let want = Some(dir.path().join("Textures/Armor/Body.DDS"));
    assert_eq!(resolve(dir.path(), "textures\\armor\\body.dds"), want);
    assert_eq!(resolve(dir.path(), "/TEXTURES/armor/BODY.dds"), want);
    assert_eq!(resolve(dir.path(), "./textures/armor/body.dds"), want);
    assert_eq!(resolve(dir.path(), "textures/missing.dds"), None);
    assert_eq!(resolve(dir.path(), "../outside"), None);
    assert_eq!(resolve(dir.path(), ""), Some(dir.path().to_path_buf()));
}

#[test]
fn images_are_found_ignoring_case_and_missing_ones_are_none() {
    let dir = archive(
        r#"<config><installSteps><installStep name="S"><optionalFileGroups><group name="G" type="SelectAny"><plugins>
            <plugin name="A"><image path="Images\Pic.JPG"/><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
            <plugin name="B"><image path="nope.png"/></plugin>
            </plugins></group></optionalFileGroups></installStep></installSteps></config>"#,
        &[("images/pic.jpg", b"jpg")],
    );
    let w = Wizard::new(load(dir.path()).unwrap(), states(&[]), None);
    let plugins = &w.step().groups[0].plugins;
    assert_eq!(plugins[0].image, Some(dir.path().join("images/pic.jpg")));
    assert_eq!(plugins[1].image, None);
}

fn first_files(config: &str) -> Vec<FileItem> {
    load_xml(config).1.required
}

#[test]
fn destination_defaults() {
    let files = first_files(
        r#"<config><requiredInstallFiles>
            <file source="a.esp"/>
            <file source="meshes\b.nif" destination=""/>
            <file source="c.esp" destination="Data\sub\"/>
            <file source="d.esp" destination="Data\renamed.esp"/>
            <folder source="textures"/>
            <folder source="loose" destination=""/>
            <folder source="x/y" destination="z"/>
            </requiredInstallFiles></config>"#,
    );
    let got: Vec<(&str, &str, bool)> = files.iter().map(|f| (f.source.as_str(), f.destination.as_str(), f.is_folder)).collect();
    assert_eq!(
        got,
        vec![
            ("a.esp", "a.esp", false),
            ("meshes/b.nif", "b.nif", false),
            ("c.esp", "Data/sub/c.esp", false),
            ("d.esp", "Data/renamed.esp", false),
            ("textures", "textures", true),
            ("loose", "", true),
            ("x/y", "z", true),
        ]
    );
}

#[test]
fn file_flags_and_priority_are_read() {
    let files = first_files(
        r#"<config><requiredInstallFiles>
            <file source="a" priority="5" alwaysInstall="true"/>
            <file source="b" installIfUsable="True" priority="-2"/>
            <file source="c" priority="oops"/>
            </requiredInstallFiles></config>"#,
    );
    assert_eq!((files[0].priority, files[0].always_install, files[0].install_if_usable), (5, true, false));
    assert_eq!((files[1].priority, files[1].always_install, files[1].install_if_usable), (-2, false, true));
    assert_eq!(files[2].priority, 0);
}

fn names(installer: &Installer) -> (Vec<String>, Vec<String>, Vec<String>) {
    let steps = installer.steps.iter().map(|s| s.name.clone()).collect();
    let groups = installer.steps[0].groups.iter().map(|g| g.name.clone()).collect();
    let plugins = installer.steps[0].groups[0].plugins.iter().map(|p| p.name.clone()).collect();
    (steps, groups, plugins)
}

fn ordered(steps: Option<&str>, groups: Option<&str>, plugins: Option<&str>) -> Installer {
    let attr = |o: Option<&str>| o.map(|o| format!(" order=\"{o}\"")).unwrap_or_default();
    let plugin = |n: &str| format!(r#"<plugin name="{n}"/>"#);
    let group = |n: &str| {
        format!(
            r#"<group name="{n}" type="SelectAny"><plugins{}>{}{}{}</plugins></group>"#,
            attr(plugins),
            plugin("b"),
            plugin("C"),
            plugin("a")
        )
    };
    let step = |n: &str| {
        format!(
            r#"<installStep name="{n}"><optionalFileGroups{}>{}{}{}</optionalFileGroups></installStep>"#,
            attr(groups),
            group("y"),
            group("Z"),
            group("x")
        )
    };
    let xml = format!(r#"<config><installSteps{}>{}{}{}</installSteps></config>"#, attr(steps), step("beta"), step("Gamma"), step("alpha"));
    load_xml(&xml).1
}

#[test]
fn ascending_is_the_default_and_ignores_case() {
    let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    for attr in [None, Some("Ascending")] {
        assert_eq!(names(&ordered(attr, attr, attr)), (s(&["alpha", "beta", "Gamma"]), s(&["x", "y", "Z"]), s(&["a", "b", "C"])));
    }
}

#[test]
fn descending_and_explicit_orders() {
    let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        names(&ordered(Some("Descending"), Some("Descending"), Some("Descending"))),
        (s(&["Gamma", "beta", "alpha"]), s(&["Z", "y", "x"]), s(&["C", "b", "a"]))
    );
    assert_eq!(
        names(&ordered(Some("Explicit"), Some("Explicit"), Some("Explicit"))),
        (s(&["beta", "Gamma", "alpha"]), s(&["y", "Z", "x"]), s(&["b", "C", "a"]))
    );
}

#[test]
fn types_and_group_kinds_are_read() {
    let (_d, installer) = load_xml(
        r#"<config><installSteps order="Explicit"><installStep name="S"><optionalFileGroups order="Explicit">
            <group name="G" type="SelectExactlyOne"><plugins order="Explicit">
              <plugin name="req"><typeDescriptor><type name="Required"/></typeDescriptor></plugin>
              <plugin name="dyn"><typeDescriptor><dependencyType><defaultType name="NotUsable"/><patterns>
                  <pattern><dependencies operator="Or"><fileDependency file="A.esp" state="Active"/><flagDependency flag="f" value="1"/></dependencies>
                  <type name="Recommended"/></pattern></patterns></dependencyType></typeDescriptor></plugin>
              <plugin name="none"/>
            </plugins></group>
            <group name="H" type="Bogus"><plugins/></group>
            </optionalFileGroups></installStep></installSteps></config>"#,
    );
    let g = &installer.steps[0].groups;
    assert_eq!((g[0].kind, g[1].kind), (GroupKind::SelectExactlyOne, GroupKind::SelectAny));
    assert!(matches!(g[0].plugins[0].kind, TypeDescriptor::Static(PluginType::Required)));
    assert!(matches!(&g[0].plugins[1].kind, TypeDescriptor::Dynamic { default: PluginType::NotUsable, patterns } if patterns.len() == 1));
    assert!(matches!(g[0].plugins[2].kind, TypeDescriptor::Static(PluginType::Optional)));
}

#[test]
fn description_and_flags_are_read() {
    let (_d, installer) = load_xml(
        r#"<config><installSteps><installStep name="S"><optionalFileGroups><group name="G" type="SelectAny"><plugins>
            <plugin name="P"><description>
               Line one
               <![CDATA[Line <two>]]>
            </description>
            <conditionFlags><flag name="A">On</flag><flag name="B"> x </flag></conditionFlags></plugin>
            </plugins></group></optionalFileGroups></installStep></installSteps></config>"#,
    );
    let p = &installer.steps[0].groups[0].plugins[0];
    assert!(p.description.starts_with("Line one") && p.description.ends_with("Line <two>"), "{:?}", p.description);
    assert_eq!(p.flags, vec![("A".to_string(), "On".to_string()), ("B".to_string(), "x".to_string())]);
}

#[test]
fn version_dependencies_of_any_tool_count_as_met() {
    let (_d, installer) = load_xml(
        r#"<config><moduleDependencies operator="And">
            <gameDependency version="1.5.0.0"/><fommDependency version="0.13.21"/><skseDependency version="2.0"/>
            </moduleDependencies></config>"#,
    );
    assert!(Wizard::new(installer, states(&[]), None).module_dependencies_met());
}
