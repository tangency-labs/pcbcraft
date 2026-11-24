use super::*;
use crate::{
    Session,
    authoring::{Action, publishing},
    board::definition::CircularNpth,
    library::sources::{LOAD, NativeSource, PREPARE_PUBLICATION},
    part_authoring::{self, ReferenceField},
    schematic::Point,
};
use serde_json::json;

const LIBRARY: &str = "check-workshop";

/// Shared-command products: a published symbol with terminals 1-4, a published footprint with
/// pads 1-3, the editors that drew them and a part draft mapping terminals 1-3 to pads 1-3.
struct Fixture {
    session: Session,
    documents: Documents,
    footprint_path: String,
    footprint: Reference,
    footprint_editor: crate::authoring::Session,
    part_editor: part_authoring::Session,
}

/// Publish a captured draft through the shared preparation command and return its one document.
/// `base` is the published revision the draft was edited from, or `None` for a new item.
fn publish(session: &mut Session, item: &Item, manifest: &str, base: Option<u32>) -> (Reference, String, String) {
    let mut params = json!({"item":item,"documents":{"library.json":manifest}});
    if let Some(base) = base {
        params["base_revision"] = json!(base);
    }
    let prepared = session.execute(PREPARE_PUBLICATION, &params).unwrap();
    let reference: Reference = serde_json::from_value(prepared["address"].clone()).unwrap();
    let (path, text) = prepared["documents"].as_object().unwrap().iter().next().unwrap();
    (reference, path.clone(), text.as_str().unwrap().to_owned())
}

/// Draw a footprint with pads 1, 2 and 3 through ordinary editor transactions.
fn footprint_editor() -> crate::authoring::Session {
    let mut editor = crate::authoring::Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let Primitive::Pad { mut pad } = editor.snapshot().objects[&1].clone() else { panic!("pad expected") };
    pad.number = "3".into();
    pad.center.x_nm = 1_500_000;
    editor.execute(editor.revision(), Action::Insert { primitive: Primitive::Pad { pad } }).unwrap();
    editor
}

/// Build the fixture through shared commands and editor transactions. Terminal 4 stays
/// unmapped, which a bounded part draft allows and publication refuses.
fn fixture() -> Fixture {
    let mut session = Session::default();
    let mut symbol_editor = crate::authoring::Session::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    let drawn = symbol_editor.snapshot();
    for (object, number) in [(1, "3"), (3, "4")] {
        let Primitive::Pin { electrical_type, at, appearance, alternates, .. } = drawn.objects[&object].clone() else { panic!("pin expected") };
        let primitive = Primitive::Pin {
            number: number.into(),
            name: Some(format!("P{number}")),
            electrical_type,
            at: Point { y_nm: 2_540_000, ..at },
            appearance,
            alternates,
        };
        symbol_editor.execute(symbol_editor.revision(), Action::Insert { primitive }).unwrap();
    }
    let manifest = publishing::new_library(LIBRARY, "Check workshop", &drawn.license).unwrap();
    let (symbol, symbol_path, symbol_text) = publish(&mut session, &symbol_editor.snapshot(), &manifest, None);
    let footprint_editor = footprint_editor();
    let (footprint, footprint_path, footprint_text) = publish(&mut session, &footprint_editor.snapshot(), &manifest, None);
    let documents = Documents::from([("library.json".into(), manifest), (symbol_path, symbol_text), (footprint_path.clone(), footprint_text)]);
    session.execute(LOAD, &json!({"source":"check-fixture","documents":documents})).unwrap();

    let mut part_editor = part_authoring::Session::new(Record::new_part(symbol).unwrap(), false).unwrap();
    let package = ReferenceField::NewPackage { package: "main".into() };
    let associated = part_editor.record().select_reference(&package, Some(footprint.clone()), session.libraries()).unwrap();
    part_editor.replace(part_editor.revision(), associated).unwrap();
    let mapped = with_package(part_editor.record(), |package| {
        package.mapping = (1..=3).map(|n| (n.to_string(), vec![n.to_string()])).collect();
    });
    part_editor.replace(part_editor.revision(), mapped).unwrap();
    Fixture { session, documents, footprint_path, footprint, footprint_editor, part_editor }
}

/// Edit the only package of a generic part draft.
fn with_package(record: &Record, edit: impl FnOnce(&mut Package)) -> Record {
    let mut record = record.clone();
    let Record::Part(part) = &mut record else { panic!("part expected") };
    edit(part.packages.values_mut().next().unwrap());
    record
}

/// A part on the bundled two-terminal connector symbol whose one package uses `footprint`.
fn connector_part(footprint: &Reference, mapping: &[(&str, &[&str])], unconnected: &[&str]) -> Record {
    let symbol = Reference { library: "basic-components".into(), id: "connector".into(), revision: 4 };
    let mut record = Record::new_part(symbol).unwrap();
    let Record::Part(part) = &mut record else { panic!("part expected") };
    part.packages.insert(
        "main".into(),
        Package {
            name: "Main".into(),
            footprint: footprint.clone(),
            mapping: mapping.iter().map(|(terminal, pads)| ((*terminal).into(), pads.iter().map(|pad| (*pad).into()).collect())).collect(),
            unconnected_pads: unconnected.iter().map(|pad| (*pad).into()).collect(),
        },
    );
    record
}

/// Copy the authored pad at `index` to the end of the footprint document, `dx_nm` to the right.
/// No shared command can author this, so it stands for a hand-edited or foreign library file.
fn repeat_pad(text: &str, index: usize, dx_nm: i64) -> String {
    let mut value: Value = serde_json::from_str(text).unwrap();
    let pads = value["pads"].as_array_mut().unwrap();
    let mut pad = pads[index].clone();
    pad["center"]["x_nm"] = json!(pad["center"]["x_nm"].as_i64().unwrap() + dx_nm);
    pads.push(pad);
    serde_json::to_string_pretty(&value).unwrap() + "\n"
}

/// The fixture documents with the footprint document replaced by an edited copy.
fn edited(fixture: &Fixture, edit: impl FnOnce(&str) -> String) -> Documents {
    let mut documents = fixture.documents.clone();
    let text = documents.get_mut(&fixture.footprint_path).unwrap();
    *text = edit(text.as_str());
    documents
}

/// Rules of a report, in report order.
fn rules(report: &Report) -> Vec<RuleId> {
    report.findings().iter().map(|finding| finding.rule).collect()
}

/// A transported footprint repeats pad 3 and the part draft leaves terminal 4 unmapped. The one
/// synthetic step is the repeated pad (see [`repeat_pad`]); everything else is built by shared
/// commands and editor transactions. The check reports exactly the two defects and leaves every
/// session, history and document as it was.
#[test]
fn repeated_pad_three_and_unmapped_terminal_give_two_findings() {
    let mut fixture = fixture();
    let documents = edited(&fixture, |text| repeat_pad(text, 2, 2_000_000));
    let item = fixture.footprint_editor.snapshot();
    let record = fixture.part_editor.record().clone();
    let params = json!({"documents":documents,"item":item.as_ref(),"record":record});
    let editors = |fixture: &Fixture| {
        let footprint = &fixture.footprint_editor;
        (footprint.revision(), footprint.can_undo(), footprint.can_redo(), footprint.dirty(), footprint.inspect(), fixture.part_editor.inspect())
    };
    let revision = fixture.session.revision();
    let before = (editors(&fixture), fixture.session.inspect(), revision, params.clone());

    let report = fixture.session.execute(CHECK, &params).unwrap();
    let again = fixture.session.execute(CHECK, &params).unwrap();
    let capture = Capture::new(&documents, Some(item.as_ref()), Some(&record), fixture.session.libraries()).unwrap();
    let direct = check(&capture);

    assert_eq!(report, again, "same capture, same digest and findings");
    assert_eq!(report, serde_json::to_value(&direct).unwrap(), "the command is the engine checker");
    assert_eq!(direct.capture_digest(), capture.digest());
    assert!(!direct.is_clean());
    let [repeated, unmapped] = direct.findings() else { panic!("exactly two findings: {report}") };
    assert_eq!(
        *repeated,
        Finding {
            rule: RuleId::PadNumberRepeated,
            severity: Severity::Error,
            subject: Subject::Pad {
                footprint: fixture.footprint.clone(),
                number: "3".into(),
                source: CaptureSource::Document { path: fixture.footprint_path.clone() },
            },
            occurrences: vec![PadOccurrenceRef { index: 2, ordinal: 1 }, PadOccurrenceRef { index: 3, ordinal: 2 }],
            measured: Quantity { value: 2, unit: Unit::Count },
            limit: Limit { value: 1, unit: Unit::Count, comparator: Comparator::Max },
            basis: Basis::ProjectRule { doc: "docs/footprint-schema.md", section: "Semantic validation" },
        }
    );
    assert_eq!(
        *unmapped,
        Finding {
            rule: RuleId::PackageTerminalUnmapped,
            severity: Severity::Error,
            subject: Subject::Terminal { part: record.reference(), package: "main".into(), terminal: "4".into(), source: CaptureSource::Draft },
            occurrences: Vec::new(),
            measured: Quantity { value: 0, unit: Unit::Count },
            limit: Limit { value: 1, unit: Unit::Count, comparator: Comparator::Min },
            basis: Basis::ProjectRule { doc: "docs/part-schema.md", section: "Part fields" },
        }
    );
    assert_eq!(report["findings"][0]["rule"], "pad.number.repeated");
    assert_eq!(report["findings"][1]["rule"], "package.terminal.unmapped");
    assert_eq!(report["findings"][0]["severity"], "error");
    assert_eq!(report["findings"][1]["severity"], "error");
    // The default run is the edit stage; its errors come from rules that also run at publish.
    assert_eq!(direct.stage(), Stage::Edit);
    assert!(direct.blocks_publish());
    assert_eq!(report["stage"], "edit");
    assert_eq!(report["blocks_publish"], true);
    assert_eq!(fixture.session.apply(CHECK, &params).unwrap(), revision);
    assert_eq!((editors(&fixture), fixture.session.inspect(), fixture.session.revision(), params), before);
}

/// Last good value: one pad numbered 1. First bad value: a second one, which is one finding
/// listing both occurrences in authored order.
#[test]
fn single_pad_number_is_clean_second_occurrence_is_one_finding() {
    let fixture = fixture();
    let catalog = Catalog::default();
    assert!(check(&Capture::from_documents(&fixture.documents, &catalog).unwrap()).is_clean());
    let report = check(&Capture::from_documents(&edited(&fixture, |text| repeat_pad(text, 0, 2_000_000)), &catalog).unwrap());
    assert_eq!(rules(&report), [RuleId::PadNumberRepeated]);
    let finding = &report.findings()[0];
    assert_eq!(finding.measured.value, 2);
    assert_eq!(finding.occurrences, [PadOccurrenceRef { index: 0, ordinal: 1 }, PadOccurrenceRef { index: 3, ordinal: 2 }]);
}

/// Three pads with one number are one finding measured three, never one row per occurrence.
#[test]
fn three_occurrences_are_one_finding_measured_three() {
    let fixture = fixture();
    let documents = edited(&fixture, |text| repeat_pad(&repeat_pad(text, 2, 2_000_000), 2, 4_000_000));
    let capture = Capture::from_documents(&documents, &Catalog::default()).unwrap();
    let report = check(&capture);
    assert_eq!(rules(&report), [RuleId::PadNumberRepeated]);
    let finding = &report.findings()[0];
    assert_eq!(finding.measured, Quantity { value: 3, unit: Unit::Count });
    let ordinals: Vec<_> = finding.occurrences.iter().map(|occurrence| (occurrence.index, occurrence.ordinal)).collect();
    assert_eq!(ordinals, [(2, 1), (3, 2), (4, 3)]);
    let pads = &capture.footprints()[0].pads;
    assert_eq!(pads.len(), 5, "every authored occurrence is kept");
    assert_eq!(pads[4].pad.center.x_nm, 5_500_000);
}

/// A complete package map on the bundled connector is clean; leaving terminal 2 without a pad
/// is one finding with a measured count of zero.
#[test]
fn fully_mapped_part_is_clean_and_dropping_one_terminal_is_one_finding() {
    let fixture = fixture();
    let catalog = Catalog::bundled();
    let complete = connector_part(&fixture.footprint, &[("1", &["1"]), ("2", &["2"])], &["3"]);
    assert!(check(&Capture::from_part_record(&complete, &fixture.documents, &catalog).unwrap()).is_clean());
    let dropped = with_package(&complete, |package| {
        package.mapping.remove("2");
        package.unconnected_pads.push("2".into());
    });
    let report = check(&Capture::from_part_record(&dropped, &fixture.documents, &catalog).unwrap());
    assert_eq!(rules(&report), [RuleId::PackageTerminalUnmapped]);
    let finding = &report.findings()[0];
    assert_eq!(finding.measured.value, 0);
    assert_eq!(finding.limit, Limit { value: 1, unit: Unit::Count, comparator: Comparator::Min });
    assert!(matches!(&finding.subject, Subject::Terminal { terminal, .. } if terminal == "2"));
}

/// A map naming pad 9 of a footprint with pads 1-3 reports that number with no occurrences.
#[test]
fn mapped_pad_absent_from_footprint_is_a_finding() {
    let fixture = fixture();
    let record = connector_part(&fixture.footprint, &[("1", &["1"]), ("2", &["2", "9"])], &["3"]);
    let report = check(&Capture::from_part_record(&record, &fixture.documents, &Catalog::bundled()).unwrap());
    assert_eq!(rules(&report), [RuleId::PackagePadNotInFootprint]);
    let finding = &report.findings()[0];
    assert!(matches!(&finding.subject, Subject::PackagePad { number, .. } if number == "9"));
    assert!(finding.occurrences.is_empty());
}

/// Pad 3 neither mapped nor unconnected is one finding naming its authored occurrence; listing
/// it as unconnected is clean.
#[test]
fn unaccounted_pad_is_a_finding() {
    let fixture = fixture();
    let catalog = Catalog::bundled();
    let record = connector_part(&fixture.footprint, &[("1", &["1"]), ("2", &["2"])], &[]);
    let report = check(&Capture::from_part_record(&record, &fixture.documents, &catalog).unwrap());
    assert_eq!(rules(&report), [RuleId::PackagePadUnaccounted]);
    assert_eq!(report.findings()[0].occurrences, [PadOccurrenceRef { index: 2, ordinal: 1 }]);
    let accounted = with_package(&record, |package| package.unconnected_pads.push("3".into()));
    assert!(check(&Capture::from_part_record(&accounted, &fixture.documents, &catalog).unwrap()).is_clean());
}

/// A symbol-less package carries no terminal map and is clean for `package.pad.unaccounted`,
/// while a symbol-backed package with an empty map still reports every footprint pad.
#[test]
fn symbol_less_package_skips_unaccounted_pad_check_but_empty_symbol_map_reports_all_pads() {
    let fixture = fixture();
    let catalog = Catalog::bundled();
    let mut symbol_less = Record::new_part(None).unwrap();
    let Record::Part(part) = &mut symbol_less else { panic!("part expected") };
    part.packages.insert(
        "main".into(),
        Package { name: "Main".into(), footprint: fixture.footprint.clone(), mapping: BTreeMap::new(), unconnected_pads: Vec::new() },
    );
    assert!(check(&Capture::from_part_record(&symbol_less, &fixture.documents, &catalog).unwrap()).is_clean());

    let empty_map = connector_part(&fixture.footprint, &[], &[]);
    let report = check(&Capture::from_part_record(&empty_map, &fixture.documents, &catalog).unwrap());
    assert_eq!(
        rules(&report),
        [
            RuleId::PackageTerminalUnmapped,
            RuleId::PackageTerminalUnmapped,
            RuleId::PackagePadUnaccounted,
            RuleId::PackagePadUnaccounted,
            RuleId::PackagePadUnaccounted,
        ]
    );
    let unaccounted: Vec<_> = report
        .findings()
        .iter()
        .filter(|finding| finding.rule == RuleId::PackagePadUnaccounted)
        .map(|finding| match &finding.subject {
            Subject::PackagePad { number, .. } => number.as_str(),
            other => panic!("unexpected subject {other:?}"),
        })
        .collect();
    assert_eq!(unaccounted, ["1", "2", "3"]);
}

/// Pad 3 assigned to two terminals is one finding measured two with a maximum of one.
#[test]
fn pad_mapped_twice_is_a_finding() {
    let fixture = fixture();
    let record = connector_part(&fixture.footprint, &[("1", &["1", "3"]), ("2", &["2", "3"])], &[]);
    let report = check(&Capture::from_part_record(&record, &fixture.documents, &Catalog::bundled()).unwrap());
    assert_eq!(rules(&report), [RuleId::PackagePadRepeated]);
    let finding = &report.findings()[0];
    assert_eq!(finding.measured.value, 2);
    assert_eq!(finding.limit.comparator, Comparator::Max);
    assert_eq!(finding.occurrences, [PadOccurrenceRef { index: 2, ordinal: 1 }]);
}

/// Invalid JSON, an unknown field and a repeated JSON key are errors, never a clean report;
/// the command reports them as invalid parameters.
#[test]
fn malformed_document_is_an_error_not_a_clean_report() {
    let mut fixture = fixture();
    let invalid = |_: &str| String::from("{");
    let unknown = |text: &str| {
        let mut value: Value = serde_json::from_str(text).unwrap();
        value["unexpected"] = json!(true);
        value.to_string()
    };
    let repeated = |text: &str| text.replacen("\"name\":", "\"name\": \"Shadow\",\n  \"name\":", 1);
    let edits: [&dyn Fn(&str) -> String; 3] = [&invalid, &unknown, &repeated];
    for edit in edits {
        let documents = edited(&fixture, edit);
        let error = Capture::from_documents(&documents, &Catalog::default()).unwrap_err();
        assert!(matches!(&error, CheckError::Malformed { file, .. } if *file == fixture.footprint_path), "{error}");
        let refused = fixture.session.execute(CHECK, &json!({"documents":documents})).unwrap_err();
        assert!(matches!(refused, crate::CommandError::InvalidParams(_)), "{refused}");
    }
}

/// An unknown format, and a known one that does not match the authored holes, are errors.
#[test]
fn unsupported_or_mismatched_format_is_an_error() {
    let fixture = fixture();
    for format in ["pcbcraft-footprint/99", FootprintDefinition::format_for(true, &[])] {
        let documents = edited(&fixture, |text| text.replacen(FootprintDefinition::format_for(false, &[]), format, 1));
        let error = Capture::from_documents(&documents, &Catalog::default()).unwrap_err();
        assert_eq!(error, CheckError::UnsupportedFormat { file: fixture.footprint_path.clone(), format: format.into() });
    }
}

/// Format-1 parts require a symbol; format-2 metadata-only parts without a symbol stay accepted.
#[test]
fn format_one_part_without_symbol_is_an_error_while_format_two_is_accepted() {
    let fixture = fixture();
    let path = "parts/spacer-r1.json".to_owned();
    let part = |format: &str| {
        json!({
            "format": format,
            "library": LIBRARY,
            "id": "spacer",
            "revision": 1,
            "name": "Spacer",
            "license": "MIT",
            "source": {"kind": "authored", "reference": "local", "locator": path},
            "packages": {}
        })
        .to_string()
    };
    let manifest = fixture.documents["library.json"].clone();
    let legacy = Documents::from([("library.json".into(), manifest.clone()), (path.clone(), part(LEGACY_PART_FORMAT))]);
    let error = Capture::from_documents(&legacy, &Catalog::default()).unwrap_err();
    assert_eq!(error, CheckError::UnsupportedFormat { file: path.clone(), format: LEGACY_PART_FORMAT.into() });

    let metadata_part = part(PART_FORMAT);
    let metadata_only = Documents::from([("library.json".into(), manifest), (path, metadata_part)]);
    let capture = Capture::from_documents(&metadata_only, &Catalog::default()).unwrap();
    assert!(check(&capture).is_clean());
}

/// A generic part draft with an unsupported format is rejected even without a symbol or
/// packages, while a supported draft with an incomplete package map is still checked.
#[test]
fn unsupported_part_draft_format_is_an_error() {
    let mut fixture = fixture();
    let mut draft = Record::new_part(None).unwrap();
    let Record::Part(part) = &mut draft else { panic!("part expected") };
    part.format = "pcbcraft-part/99".into();
    let error = Capture::from_part_record(&draft, &Documents::new(), &Catalog::default()).unwrap_err();
    assert_eq!(error, CheckError::UnsupportedFormat { file: "record".into(), format: "pcbcraft-part/99".into() });
    let refused = fixture.session.execute(CHECK, &json!({"documents":{},"record":draft})).unwrap_err();
    assert!(matches!(refused, crate::CommandError::InvalidParams(_)), "{refused}");

    let mut legacy_without_symbol = Record::new_part(None).unwrap();
    let Record::Part(part) = &mut legacy_without_symbol else { panic!("part expected") };
    part.format = LEGACY_PART_FORMAT.into();
    let error = Capture::from_part_record(&legacy_without_symbol, &Documents::new(), &Catalog::default()).unwrap_err();
    assert_eq!(error, CheckError::UnsupportedFormat { file: "record".into(), format: LEGACY_PART_FORMAT.into() });
    let refused = fixture.session.execute(CHECK, &json!({"documents":{},"record":legacy_without_symbol})).unwrap_err();
    assert!(matches!(refused, crate::CommandError::InvalidParams(_)), "{refused}");

    let mut legacy = fixture.part_editor.record().clone();
    let Record::Part(part) = &mut legacy else { panic!("part expected") };
    part.format = LEGACY_PART_FORMAT.into();
    let report = check(&Capture::from_part_record(&legacy, &fixture.documents, fixture.session.libraries()).unwrap());
    assert_eq!(rules(&report), [RuleId::PackageTerminalUnmapped]);
}

/// An exact footprint or symbol found neither in the documents nor in the catalog is an error.
#[test]
fn unresolved_reference_is_an_error() {
    let fixture = fixture();
    let missing = Reference { library: LIBRARY.into(), id: "missing".into(), revision: 1 };
    let record = connector_part(&missing, &[("1", &["1"]), ("2", &["2"])], &[]);
    let error = Capture::from_part_record(&record, &fixture.documents, &Catalog::bundled()).unwrap_err();
    assert_eq!(error, CheckError::Unresolved { kind: "footprint", reference: missing });
    let error = Capture::from_part_record(&record, &fixture.documents, &Catalog::default()).unwrap_err();
    assert!(matches!(error, CheckError::Unresolved { kind: "symbol", .. }), "{error}");
}

/// More pads than the native format allows, or a document over the file limit, is an error.
#[test]
fn oversized_capture_is_an_error() {
    let fixture = fixture();
    // The fixture footprint has 3 pads; add copies up to the limit, then one more.
    let documents = edited(&fixture, |text| (4..=MAX_PADS).fold(text.to_owned(), |text, _| repeat_pad(&text, 0, 0)));
    assert!(Capture::from_documents(&documents, &Catalog::default()).is_ok(), "the last good value is the limit");
    let documents = edited(&fixture, |text| (4..=MAX_PADS + 1).fold(text.to_owned(), |text, _| repeat_pad(&text, 0, 0)));
    let error = Capture::from_documents(&documents, &Catalog::default()).unwrap_err();
    assert!(matches!(error, CheckError::TooLarge { limit: MAX_PADS, .. }), "{error}");
    let documents = edited(&fixture, |text| format!("{text}{}", " ".repeat(MAX_FILE_BYTES)));
    let error = Capture::from_documents(&documents, &Catalog::default()).unwrap_err();
    assert_eq!(error, CheckError::TooLarge { what: fixture.footprint_path.clone(), limit: MAX_FILE_BYTES });
}

/// A package's combined distinct pad count across all mapped terminals and `unconnected_pads` is
/// bounded by `MAX_PADS`, while repeated pad numbers still reach `package.pad.repeated`.
#[test]
fn package_distinct_pad_limit_is_enforced_without_blocking_repeated_pads() {
    let fixture = fixture();
    let catalog = Catalog::bundled();
    let base = connector_part(&fixture.footprint, &[], &[]);
    let pads = |range: std::ops::RangeInclusive<usize>| range.map(|n| n.to_string()).collect::<Vec<_>>();
    let half = MAX_PADS / 2;

    let at_limit = with_package(&base, |package| {
        package.mapping.insert("1".into(), pads(1..=half));
        package.mapping.insert("2".into(), pads((half + 1)..=(MAX_PADS - 1)));
        package.unconnected_pads = vec![MAX_PADS.to_string()];
    });
    assert!(Capture::from_part_record(&at_limit, &fixture.documents, &catalog).is_ok());

    let over_across_terminals = with_package(&base, |package| {
        package.mapping.insert("1".into(), pads(1..=half));
        package.mapping.insert("2".into(), pads((half + 1)..=(MAX_PADS + 1)));
    });
    let error = Capture::from_part_record(&over_across_terminals, &fixture.documents, &catalog).unwrap_err();
    assert!(matches!(error, CheckError::TooLarge { limit: MAX_PADS, .. }), "{error}");

    let over_with_unconnected = with_package(&base, |package| {
        package.mapping.insert("1".into(), pads(1..=half));
        package.mapping.insert("2".into(), pads((half + 1)..=MAX_PADS));
        package.unconnected_pads = vec![(MAX_PADS + 1).to_string()];
    });
    let error = Capture::from_part_record(&over_with_unconnected, &fixture.documents, &catalog).unwrap_err();
    assert!(matches!(error, CheckError::TooLarge { limit: MAX_PADS, .. }), "{error}");

    let repeated_count = half + 1;
    let repeated = with_package(&base, |package| {
        package.mapping.insert("1".into(), pads(1..=repeated_count));
        package.mapping.insert("2".into(), pads(1..=repeated_count));
        package.unconnected_pads = vec!["1".into()];
    });
    let report = check(&Capture::from_part_record(&repeated, &fixture.documents, &catalog).unwrap());
    let repeated_findings: Vec<_> = report.findings().iter().filter(|finding| finding.rule == RuleId::PackagePadRepeated).collect();
    assert_eq!(repeated_findings.len(), repeated_count);
    let pad_one = repeated_findings
        .iter()
        .find(|finding| matches!(&finding.subject, Subject::PackagePad { number, .. } if number == "1"))
        .expect("pad 1 should be reported as repeated");
    assert_eq!(pad_one.measured.value, 3);
}

/// Non-plated holes are not pad occurrences, in a published document or in an editor draft.
#[test]
fn non_plated_holes_are_not_pad_occurrences() {
    let mut fixture = fixture();
    let editor = &mut fixture.footprint_editor;
    let hole = CircularNpth { center: Point { x_nm: 0, y_nm: 3_000_000 }, diameter_nm: 1_000_000 };
    editor.execute(editor.revision(), Action::Insert { primitive: Primitive::NonPlatedHole { hole } }).unwrap();
    let item = editor.snapshot();
    let manifest = fixture.documents["library.json"].clone();
    // The fixture already published this footprint, so the edit names the revision it started from.
    let older = json!({"item":item.as_ref(),"documents":{"library.json":manifest}});
    assert!(fixture.session.execute(PREPARE_PUBLICATION, &older).is_err(), "an edit without its base still needs consent");
    let (_, path, text) = publish(&mut fixture.session, &item, &manifest, Some(fixture.footprint.revision));
    assert!(text.contains(FootprintDefinition::format_for(true, &[])));
    let documents = Documents::from([("library.json".into(), manifest), (path, text)]);
    for capture in [Capture::from_documents(&documents, &Catalog::default()).unwrap(), Capture::from_item(&item).unwrap()] {
        let numbers: Vec<_> = capture.footprints()[0].pads.iter().map(|pad| pad.number.as_str()).collect();
        assert_eq!(numbers, ["1", "2", "3"]);
        assert!(check(&capture).is_clean());
    }
}

/// Reports sort by rule and exact identity, not by document path, insertion or authored order.
#[test]
fn findings_order_is_independent_of_document_order() {
    let fixture = fixture();
    // Pad 2 is repeated before pad 1 is.
    let text = repeat_pad(&repeat_pad(&fixture.documents[&fixture.footprint_path], 1, 2_000_000), 0, 2_000_000);
    let mut copy: Value = serde_json::from_str(&text).unwrap();
    copy["id"] = json!("aa-copy");
    let entries = [
        ("library.json".to_owned(), fixture.documents["library.json"].clone()),
        (fixture.footprint_path.clone(), text),
        ("footprints/zz-copy.json".to_owned(), copy.to_string()),
    ];
    let forward: Documents = entries.iter().cloned().collect();
    let backward: Documents = entries.iter().rev().cloned().collect();
    let report = check(&Capture::from_documents(&forward, &Catalog::default()).unwrap());
    assert_eq!(report, check(&Capture::from_documents(&backward, &Catalog::default()).unwrap()));
    let order: Vec<(String, String)> = report
        .findings()
        .iter()
        .map(|finding| match &finding.subject {
            Subject::Pad { footprint, number, .. } => (footprint.id.clone(), number.clone()),
            other => panic!("unexpected subject {other:?}"),
        })
        .collect();
    let id = fixture.footprint.id.clone();
    assert_eq!(order, [("aa-copy".into(), "1".into()), ("aa-copy".into(), "2".into()), (id.clone(), "1".into()), (id, "2".into())]);
}

/// The check explains the defect without relaxing anything: the editor still refuses a second
/// pad 3 and strict loading still reports the transported document as a problem.
#[test]
fn editor_still_refuses_repeated_pad_numbers() {
    let fixture = fixture();
    let mut editor = footprint_editor();
    let before = editor.snapshot();
    let Primitive::Pad { pad } = before.objects[&4].clone() else { panic!("pad expected") };
    assert!(editor.execute(editor.revision(), Action::Insert { primitive: Primitive::Pad { pad } }).is_err());
    assert_eq!(editor.snapshot(), before);
    let documents = edited(&fixture, |text| repeat_pad(text, 2, 2_000_000));
    let source = NativeSource::load(&documents).unwrap();
    assert!(source.problems().iter().any(|problem| problem.starts_with(&fixture.footprint_path)), "{:?}", source.problems());
}

/// Repeated JSON keys in `library.json` or symbol documents (at top level or nested inside pins)
/// are rejected before `serde_json::Value` decoding and surface as `InvalidParams`.
#[test]
fn repeated_keys_in_manifest_and_symbol_are_rejected_as_invalid_params() {
    let mut fixture = fixture();
    let symbol_path = fixture.documents.keys().find(|path| path.starts_with("symbols/")).unwrap().clone();
    let cases = [
        ("library.json".to_owned(), "\"name\":", "\"name\": \"Shadow\", \"name\":"),
        (symbol_path.clone(), "\"name\":", "\"name\": \"Shadow\", \"name\":"),
        (symbol_path.clone(), "\"terminal\":", "\"terminal\": \"99\", \"terminal\":"),
    ];
    for (target_path, needle, replacement) in cases {
        let mut documents = fixture.documents.clone();
        let original = documents[&target_path].clone();
        let mutated = original.replacen(needle, replacement, 1);
        assert_ne!(mutated, original, "expected {needle} in {target_path}");
        documents.insert(target_path.clone(), mutated);

        let error = Capture::from_documents(&documents, &Catalog::default()).unwrap_err();
        assert!(
            matches!(&error, CheckError::Malformed { file, reason } if *file == target_path && reason.contains("duplicate field")),
            "expected duplicate-key Malformed error for {target_path}, got {error}"
        );
        let refused = fixture.session.execute(CHECK, &json!({"documents":documents})).unwrap_err();
        assert!(matches!(refused, crate::CommandError::InvalidParams(_)), "{refused}");
    }
}

/// Two transported part documents with the same library, ID and revision are a malformed
/// capture; distinct revisions of the same part remain accepted.
#[test]
fn repeated_part_revision_is_malformed_but_distinct_revisions_are_accepted() {
    let fixture = fixture();
    let Record::Part(mut part) = fixture.part_editor.record().clone() else { panic!("part expected") };
    part.library = LIBRARY.into();
    let rev1 = serde_json::to_string(&part).unwrap();

    let mut duplicate = fixture.documents.clone();
    duplicate.insert("parts/a.json".into(), rev1.clone());
    duplicate.insert("parts/b.json".into(), rev1.clone());
    let error = Capture::from_documents(&duplicate, &Catalog::default()).unwrap_err();
    assert_eq!(error, CheckError::Malformed { file: "parts/b.json".into(), reason: "Another document defines the same part revision".into() });

    part.revision += 1;
    let rev2 = serde_json::to_string(&part).unwrap();
    let mut distinct = fixture.documents.clone();
    distinct.insert("parts/a.json".into(), rev1);
    distinct.insert("parts/b.json".into(), rev2);
    let capture = Capture::from_documents(&distinct, &Catalog::default()).unwrap();
    assert_eq!(capture.packages().len(), 2);
}

/// The done capture at the publish stage: two errors, no warnings or notes, and a report that
/// blocks publication. The command and the engine checker agree.
#[test]
fn check_report_counts_by_severity() {
    let mut fixture = fixture();
    let documents = edited(&fixture, |text| repeat_pad(text, 2, 2_000_000));
    let item = fixture.footprint_editor.snapshot();
    let record = fixture.part_editor.record().clone();
    let capture = Capture::new(&documents, Some(item.as_ref()), Some(&record), fixture.session.libraries()).unwrap();
    let options = Options { stage: Stage::Publish, ..Options::default() };
    let report = check_with(&capture, &options, RULES).unwrap();
    assert_eq!(rules(&report), [RuleId::PadNumberRepeated, RuleId::PackageTerminalUnmapped]);
    assert!(report.findings().iter().all(|finding| finding.severity == Severity::Error));
    assert_eq!(report.counts(), Counts { error: 2, warning: 0, info: 0 });
    assert!(report.blocks_publish());
    assert_eq!(report.stage(), Stage::Publish);
    let params = json!({"documents":documents,"item":item.as_ref(),"record":record,"stage":"publish"});
    let value = fixture.session.execute(CHECK, &params).unwrap();
    assert_eq!(value, serde_json::to_value(&report).unwrap());
    assert_eq!(value["counts"], json!({"error":2,"warning":0,"info":0}));
    assert_eq!(value["blocks_publish"], true);
    assert_eq!(value["stage"], "publish");
    // A clean capture counts nothing and blocks nothing.
    let clean = check_with(&Capture::from_documents(&fixture.documents, &Catalog::default()).unwrap(), &options, RULES).unwrap();
    assert_eq!((clean.counts(), clean.blocks_publish()), (Counts::default(), false));
}

/// A rule registered only for publication reports nothing at the edit stage and its finding at
/// the publish stage. The default stage is edit.
#[test]
fn check_stage_filter() {
    let fixture = fixture();
    let capture = Capture::from_documents(&edited(&fixture, |text| repeat_pad(text, 2, 2_000_000)), &Catalog::default()).unwrap();
    let publish_only = [Rule { stages: &[Stage::Publish], ..RULES[0] }];
    assert_eq!(publish_only[0].id, RuleId::PadNumberRepeated);
    let edit = check_with(&capture, &Options::default(), &publish_only).unwrap();
    assert_eq!(edit.stage(), Stage::Edit);
    assert!(edit.is_clean(), "a publish-only rule does not run while editing");
    assert_eq!((edit.counts(), edit.blocks_publish()), (Counts::default(), false));
    let publish = check_with(&capture, &Options { stage: Stage::Publish, ..Options::default() }, &publish_only).unwrap();
    assert_eq!(rules(&publish), [RuleId::PadNumberRepeated]);
    assert!(publish.blocks_publish());
    // The built-in rule runs at every stage, and its error at the edit stage blocks publication.
    let built_in = check_with(&capture, &Options::default(), RULES).unwrap();
    assert_eq!(rules(&built_in), [RuleId::PadNumberRepeated]);
    assert!(built_in.blocks_publish());
}

/// Lowering `package.pad.repeated` to a warning keeps its finding but no longer blocks
/// publication when no other error is reported.
#[test]
fn check_override_lowers_to_warning() {
    let mut fixture = fixture();
    let record = connector_part(&fixture.footprint, &[("1", &["1", "3"]), ("2", &["2", "3"])], &[]);
    let capture = Capture::from_part_record(&record, &fixture.documents, &Catalog::bundled()).unwrap();
    let options = |severity: Severity| Options { stage: Stage::Publish, severity: BTreeMap::from([("package.pad.repeated".to_owned(), severity)]) };
    let default = check_with(&capture, &Options { stage: Stage::Publish, ..Options::default() }, RULES).unwrap();
    assert_eq!(rules(&default), [RuleId::PackagePadRepeated]);
    assert!(default.blocks_publish());
    let lowered = check_with(&capture, &options(Severity::Warning), RULES).unwrap();
    assert_eq!(rules(&lowered), [RuleId::PackagePadRepeated]);
    assert_eq!(lowered.findings()[0].severity, Severity::Warning);
    assert_eq!(lowered.counts(), Counts { error: 0, warning: 1, info: 0 });
    assert!(!lowered.blocks_publish());
    assert_eq!(lowered.capture_digest(), default.capture_digest(), "an override changes the run, not the capture");
    let params = json!({"documents":fixture.documents,"record":record,"stage":"publish","severity":{"package.pad.repeated":"warning"}});
    let value = fixture.session.execute(CHECK, &params).unwrap();
    assert_eq!(value, serde_json::to_value(&lowered).unwrap());
    assert_eq!(value["findings"][0]["severity"], "warning");
    assert_eq!(value["blocks_publish"], false);
}

/// An override naming no registered rule is refused, never ignored.
#[test]
fn check_override_unknown_rule_refused() {
    let mut fixture = fixture();
    let capture = Capture::from_documents(&fixture.documents, &Catalog::default()).unwrap();
    let options = Options { severity: BTreeMap::from([("pad.number.missing".to_owned(), Severity::Warning)]), ..Options::default() };
    assert_eq!(check_with(&capture, &options, RULES).unwrap_err(), CheckError::UnknownRule { rule: "pad.number.missing".into() });
    let params = json!({"documents":fixture.documents,"severity":{"pad.number.missing":"warning"}});
    let refused = fixture.session.execute(CHECK, &params).unwrap_err();
    assert!(matches!(&refused, crate::CommandError::InvalidParams(message) if message.contains("pad.number.missing")), "{refused}");
    let refused = fixture.session.execute(CHECK, &json!({"documents":fixture.documents,"severity":{"pad.number.repeated":"fatal"}})).unwrap_err();
    assert!(matches!(refused, crate::CommandError::InvalidParams(_)), "{refused}");
    let refused = fixture.session.execute(CHECK, &json!({"documents":fixture.documents,"stage":"review"})).unwrap_err();
    assert!(matches!(refused, crate::CommandError::InvalidParams(_)), "{refused}");
}

/// A required rule can be lowered to a warning (last good value) but not to info (first bad
/// value); an optional rule can.
#[test]
fn check_override_required_floor() {
    let mut fixture = fixture();
    let capture = Capture::from_documents(&fixture.documents, &Catalog::default()).unwrap();
    let options = |severity: Severity| Options { severity: BTreeMap::from([("pad.number.repeated".to_owned(), severity)]), ..Options::default() };
    assert!(RULES.iter().all(|rule| rule.required));
    assert!(check_with(&capture, &options(Severity::Warning), RULES).is_ok());
    assert_eq!(
        check_with(&capture, &options(Severity::Info), RULES).unwrap_err(),
        CheckError::BelowFloor { rule: "pad.number.repeated".into(), floor: Severity::Warning }
    );
    let optional = [Rule { required: false, ..RULES[0] }];
    assert!(check_with(&capture, &options(Severity::Info), &optional).is_ok());
    let params = json!({"documents":fixture.documents,"severity":{"pad.number.repeated":"info"}});
    let refused = fixture.session.execute(CHECK, &params).unwrap_err();
    assert!(matches!(&refused, crate::CommandError::InvalidParams(message) if message.contains("pad.number.repeated")), "{refused}");
}

/// `blocks_publish` follows the rule's stages, not the stage that ran: the built-in errors block
/// publication at edit, place and publish, while an error from a rule that never runs at publish
/// is counted but blocks nothing.
#[test]
fn check_blocks_publish_follows_the_rule_stages() {
    let mut fixture = fixture();
    let documents = edited(&fixture, |text| repeat_pad(text, 2, 2_000_000));
    let capture = Capture::from_documents(&documents, &Catalog::default()).unwrap();
    for (stage, name) in [(Stage::Edit, "edit"), (Stage::Place, "place"), (Stage::Publish, "publish")] {
        let report = check_with(&capture, &Options { stage, ..Options::default() }, RULES).unwrap();
        assert_eq!(rules(&report), [RuleId::PadNumberRepeated], "{name}");
        assert_eq!(report.counts(), Counts { error: 1, warning: 0, info: 0 }, "{name}");
        assert!(report.blocks_publish(), "{name}");
        let value = fixture.session.execute(CHECK, &json!({"documents":documents,"stage":name})).unwrap();
        assert_eq!(value, serde_json::to_value(&report).unwrap(), "{name}");
        assert_eq!((&value["stage"], &value["blocks_publish"]), (&json!(name), &json!(true)));
    }
    let edit_only = [Rule { stages: &[Stage::Edit], ..RULES[0] }];
    let report = check_with(&capture, &Options::default(), &edit_only).unwrap();
    assert_eq!(rules(&report), [RuleId::PadNumberRepeated]);
    assert_eq!(report.findings()[0].severity, Severity::Error);
    assert_eq!(report.counts(), Counts { error: 1, warning: 0, info: 0 });
    assert!(!report.blocks_publish(), "an edit-only error does not block publication");
}

/// A rule registered only for placement reports at the place stage and nowhere else; an optional
/// rule at `info`, by default or by override, is counted as info and blocks nothing.
#[test]
fn check_place_stage_and_info_findings() {
    let fixture = fixture();
    let capture = Capture::from_documents(&edited(&fixture, |text| repeat_pad(text, 2, 2_000_000)), &Catalog::default()).unwrap();
    let place_only = [Rule { stages: &[Stage::Place], ..RULES[0] }];
    for stage in [Stage::Edit, Stage::Publish] {
        assert!(check_with(&capture, &Options { stage, ..Options::default() }, &place_only).unwrap().is_clean(), "{stage:?}");
    }
    let place = check_with(&capture, &Options { stage: Stage::Place, ..Options::default() }, &place_only).unwrap();
    assert_eq!((place.stage(), rules(&place)), (Stage::Place, vec![RuleId::PadNumberRepeated]));
    assert!(!place.blocks_publish(), "a place-only rule does not block publication");
    assert_eq!(serde_json::to_value(&place).unwrap()["stage"], "place");

    let note = [Rule { severity: Severity::Info, required: false, ..RULES[0] }];
    let lowered = Options { severity: BTreeMap::from([("pad.number.repeated".to_owned(), Severity::Info)]), ..Options::default() };
    let optional = [Rule { required: false, ..RULES[0] }];
    for report in [check_with(&capture, &Options::default(), &note).unwrap(), check_with(&capture, &lowered, &optional).unwrap()] {
        assert_eq!(rules(&report), [RuleId::PadNumberRepeated]);
        assert_eq!(report.findings()[0].severity, Severity::Info);
        assert_eq!(report.counts(), Counts { error: 0, warning: 0, info: 1 });
        assert!(!report.is_clean());
        assert!(!report.blocks_publish());
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["findings"][0]["severity"], "info");
        assert_eq!(value["counts"], json!({"error":0,"warning":0,"info":1}));
    }
}

const SLOT_FORMAT: &str = "pcbcraft-footprint/5";

/// Turn the pad at `index` into a through-hole land with a plated slot and set `format`, the
/// way a published plated-slot footprint is written.
fn slotted(text: &str, index: usize, format: &str) -> String {
    let mut value: Value = serde_json::from_str(text).unwrap();
    value["format"] = json!(format);
    let pad = value["pads"][index].as_object_mut().unwrap();
    pad.remove("corner_radius_nm");
    pad.remove("drill_nm");
    pad.insert("shape".into(), json!("circle"));
    pad.insert("width_nm".into(), json!(1_800_000));
    pad.insert("height_nm".into(), json!(1_800_000));
    pad.insert("copper".into(), json!("all_copper"));
    pad.insert("plated_slot".into(), json!({"width_nm":600_000,"length_nm":1_200_000,"rotation_mdeg":0}));
    serde_json::to_string_pretty(&value).unwrap() + "\n"
}

/// A plated-slot footprint is checked like any other: its slotted pad is a pad, a repeated
/// slotted pad number is one finding, and the slot format must match the authored holes.
#[test]
fn plated_slot_footprints_are_checked() {
    let mut fixture = fixture();
    let slot = edited(&fixture, |text| slotted(text, 2, SLOT_FORMAT));
    assert!(NativeSource::load(&slot).unwrap().problems().is_empty(), "the slotted fixture is a valid footprint");
    let capture = Capture::from_documents(&slot, &Catalog::default()).unwrap();
    let pads = &capture.footprints()[0].pads;
    let numbers: Vec<_> = pads.iter().map(|pad| pad.number.as_str()).collect();
    assert_eq!(numbers, ["1", "2", "3"]);
    assert!(pads[2].pad.plated_slot.is_some());
    assert!(check(&capture).is_clean());
    let value = fixture.session.execute(CHECK, &json!({"documents":slot})).unwrap();
    assert_eq!(value["findings"], json!([]));

    let repeated = edited(&fixture, |text| repeat_pad(&slotted(text, 2, SLOT_FORMAT), 2, 2_000_000));
    let report = check(&Capture::from_documents(&repeated, &Catalog::default()).unwrap());
    assert_eq!(rules(&report), [RuleId::PadNumberRepeated]);
    assert_eq!(report.findings()[0].measured.value, 2);
    assert_eq!(report.findings()[0].occurrences, [PadOccurrenceRef { index: 2, ordinal: 1 }, PadOccurrenceRef { index: 3, ordinal: 2 }]);

    // The slot format without a slot, and a slot under an older format, are mismatches.
    let unslotted = edited(&fixture, |text| text.replacen(FootprintDefinition::format_for(false, &[]), SLOT_FORMAT, 1));
    let older = edited(&fixture, |text| slotted(text, 2, FootprintDefinition::format_for(true, &[])));
    for (documents, format) in [(unslotted, SLOT_FORMAT), (older, FootprintDefinition::format_for(true, &[]))] {
        let error = Capture::from_documents(&documents, &Catalog::default()).unwrap_err();
        assert_eq!(error, CheckError::UnsupportedFormat { file: fixture.footprint_path.clone(), format: format.into() });
    }
}
