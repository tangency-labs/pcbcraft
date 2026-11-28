use super::*;

const ROOT: &str = "30000000-0000-4000-8000-000000000000";
const A: &str = "30000000-0000-4000-8000-000000000001";
const B: &str = "30000000-0000-4000-8000-000000000002";

fn block(id: &str, file: &str) -> String {
    format!(r#"(sheet (uuid "{id}") (at 10 20) (size 25 25) (property "Sheetname" "Child") (property "Sheetfile" "{file}"))"#)
}

fn project(items: &str) -> String {
    format!(r#"(kicad_sch (uuid "{ROOT}") {items})"#)
}

#[test]
fn repeated_source_is_read_once_and_each_occurrence_uses_its_own_record() {
    let root = project(&format!("{} {}", block(A, "child.kicad_sch"), block(B, "./child.kicad_sch")));
    let child = format!(
        r#"(kicad_sch (symbol (lib_id "Example:TwoUnit") (at 40 40 0) (unit 2)
           (instances (project "Root" (path "/{ROOT}/{B}" (reference "U3") (unit 1))
                                      (path "/{ROOT}/{A}" (reference "U2"))))))"#
    );
    let mut reads = Vec::new();
    let sheets = load("project/root.kicad_sch", |file, _| {
        reads.push(file.to_owned());
        match file {
            "project/root.kicad_sch" => Ok(root.clone()),
            "project/child.kicad_sch" => Ok(child.clone()),
            _ => Err("missing sheet".into()),
        }
    })
    .unwrap();
    assert_eq!(reads, ["project/root.kicad_sch", "project/child.kicad_sch"]);
    assert_eq!(sheets[1].parent, Some((0, 0)));
    assert_eq!(sheets[2].parent, Some((0, 1)));
    assert_eq!(sheets[1].instance_path, format!("/{ROOT}/{A}"));
    assert_eq!(sheets[2].instance_path, format!("/{ROOT}/{B}"));
    assert_eq!(sheets[1].placements, [("U2".to_owned(), 2)]);
    assert_eq!(sheets[2].placements, [("U3".to_owned(), 1)]);
    assert!(Arc::ptr_eq(&sheets[1].source, &sheets[2].source));
}

#[test]
fn missing_paths_cycles_and_escapes_report_the_source_file() {
    let root = project(&block(A, "detail/child.kicad_sch"));
    let read_root = |file: &str| (file == "root.kicad_sch").then(|| root.clone());
    let missing = load("root.kicad_sch", |file, _| read_root(file).ok_or_else(|| "missing sheet".into())).unwrap_err();
    assert!(missing.to_string().contains("detail/child.kicad_sch: missing sheet"));

    let cycle = project(&block(B, "../root.kicad_sch"));
    let looped = load("root.kicad_sch", |file, _| Ok(read_root(file).unwrap_or_else(|| cycle.clone()))).unwrap_err();
    assert!(looped.to_string().contains("root.kicad_sch: a sheet cycle"));
    assert!(looped.to_string().contains(&format!("/{ROOT}/{A}/{B}")));

    let escape = project(&block(B, "../../private.kicad_sch"));
    let invalid = load("root.kicad_sch", |file, _| Ok(read_root(file).unwrap_or_else(|| escape.clone()))).unwrap_err();
    assert!(invalid.to_string().contains("detail/child.kicad_sch: invalid child path"));
}

#[test]
fn invalid_or_duplicate_occurrences_are_rejected() {
    let duplicate = project(&format!("{} {}", block(A, "first.kicad_sch"), block(A, "second.kicad_sch")));
    let error = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { duplicate.clone() } else { "(kicad_sch)".into() })).unwrap_err();
    assert!(error.to_string().contains("second.kicad_sch: duplicate sheet occurrence"));

    let one = project(&block(A, "child.kicad_sch"));
    let other_project = format!(
        r#"(kicad_sch (symbol (lib_id "Example:R") (at 0 0 0)
           (instances (project "Elsewhere" (path "/{ROOT}/{B}" (reference "R8") (unit 1))))))"#
    );
    let error = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { one.clone() } else { other_project.clone() })).unwrap_err();
    assert!(error.to_string().contains(&format!("child.kicad_sch (/{ROOT}/{A}): no matching instance record")));
}
