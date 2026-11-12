use super::*;
use crate::{
    ADD_TRACKS, CONNECT_PINS, DELETE_SYMBOL, INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_RESISTOR, PLACE_SYMBOL, PLACE_UNIT, SET_SYMBOL_FIELDS,
    Session,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Create an explicitly board-owning project through the same command used by the application.
fn session() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    session
}

/// Apply one revision-checked shared command without bypassing engine validation or history.
fn edit(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Return the physical board's stable UUID from project inspection.
fn board_id(session: &Session) -> String {
    session.project().unwrap().board_target().unwrap().board.to_string()
}

/// Place a resistor and return its placed symbol and stable logical component IDs.
fn resistor(session: &mut Session, x_nm: i64) -> (String, String) {
    edit(session, PLACE_RESISTOR, json!({"x_nm":x_nm,"y_nm":0}));
    let symbol = session.project().unwrap().symbols().last().unwrap();
    (symbol.id(), symbol.component_id())
}

/// Add the legacy two-terminal marker footprint through the validated placement command.
fn footprint(session: &mut Session, symbol: &str, board: &str, x_nm: i64) {
    edit(session, PLACE_FOOTPRINT, json!({"board":board,"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":x_nm,"y_nm":0}));
}

/// Create a named alternative and return the UUID published by ordinary project inspection.
fn create_variant(session: &mut Session, name: &str, board: &str) -> Uuid {
    edit(session, CREATE_ASSEMBLY_VARIANT, json!({"board":board,"name":name}));
    session.project().unwrap().assembly_variants().find(|variant| variant.name == name.trim()).unwrap().id
}

/// Set or clear one stable part's fitted state in a named variant.
fn population(session: &mut Session, id: Uuid, component: &str, fitted: Value, board: &str) {
    edit(session, SET_ASSEMBLY_VARIANT, json!({"board":board,"variant":id,"component":component,"fitted":fitted}));
}

/// Generate outputs via the same command catalog path exposed to GUI and MCP callers.
fn report(session: &mut Session, board: &str, variant: Value) -> Value {
    session.execute(crate::board::assembly::EXPORT_ASSEMBLY, &json!({"board":board,"variant":variant})).unwrap()
}

/// Look up one generated file's contents in the command report.
fn contents<'a>(report: &'a Value, name: &str) -> &'a str {
    report["assembly"]["files"].as_array().unwrap().iter().find(|file| file["name"] == name).unwrap()["contents"].as_str().unwrap()
}

/// Check whether a CSV contains a complete row beginning with the requested reference.
fn has_reference(csv: &str, reference: &str) -> bool {
    csv.lines().skip(1).any(|line| line.starts_with(&format!("\"{reference}\",")))
}

/// Replace only the named component's attributes while preserving standard metadata fields.
fn attributes(dnp: bool, exclude_bom: bool, exclude_board: bool, exclude_position: bool) -> Value {
    json!({"fields":{},"attributes":{
        "do_not_populate":dnp,
        "exclude_from_bom":exclude_bom,
        "exclude_from_board":exclude_board,
        "exclude_from_simulation":false,
        "exclude_from_position_files":exclude_position,
    }})
}

/// Update component-owned attributes through the authored symbol-fields command.
fn set_attributes(session: &mut Session, symbol: &str, reference: &str, metadata: Value) {
    edit(session, SET_SYMBOL_FIELDS, json!({"symbol":symbol,"reference":reference,"value":"1k","metadata":metadata}));
}

/// DNP changes BOM and placement membership while leaving schematic, footprints, tracks and nets intact.
#[test]
fn complementary_variants_change_outputs_without_changing_physical_or_electrical_state() {
    let mut session = session();
    let board = board_id(&session);
    edit(&mut session, INITIALIZE_BOARD_SETUP, json!({"board":board,"copper_layers":2}));
    let (r1_symbol, r1) = resistor(&mut session, 0);
    let (r2_symbol, r2) = resistor(&mut session, 25_400_000);
    edit(&mut session, CONNECT_PINS, json!({"from":{"symbol":r1_symbol,"terminal":"1"},"to":{"symbol":r2_symbol,"terminal":"1"}}));
    footprint(&mut session, &r1_symbol, &board, 0);
    footprint(&mut session, &r2_symbol, &board, 25_400_000);

    let front_copper = session
        .project()
        .unwrap()
        .board_setup()
        .unwrap()
        .layers()
        .iter()
        .find(|layer| layer.purpose == crate::board::setup::LayerPurpose::Copper && layer.side == Some(crate::board::setup::Side::Front))
        .unwrap()
        .id;
    edit(
        &mut session,
        ADD_TRACKS,
        json!({"board":board,"layer":front_copper,"points":[{"x_nm":50_000_000,"y_nm":50_000_000},{"x_nm":55_000_000,"y_nm":50_000_000}]}),
    );

    let a = create_variant(&mut session, "Assembly A", &board);
    let b = create_variant(&mut session, "Assembly B", &board);
    population(&mut session, a, &r1, json!(false), &board);
    population(&mut session, b, &r2, json!(false), &board);
    let snapshot = session.inspect()["project"].clone();
    let revision = session.revision();

    let base = report(&mut session, &board, Value::Null);
    let a_report = report(&mut session, &board, json!(a));
    let b_report = report(&mut session, &board, json!(b));

    for (output, r1_fitted, r2_fitted) in [(&base, true, true), (&a_report, false, true), (&b_report, true, false)] {
        assert_eq!(output["revision"], json!(revision), "exports are read-only queries");
        assert_eq!(session.inspect()["project"], snapshot, "choosing an output variant does not change Base or connectivity");
        let bom = contents(output, "BOM.csv");
        let placement = contents(output, "Placement.csv");
        assert_eq!(has_reference(bom, "R1"), r1_fitted);
        assert_eq!(has_reference(bom, "R2"), r2_fitted);
        assert_eq!(has_reference(placement, "R1"), r1_fitted);
        assert_eq!(has_reference(placement, "R2"), r2_fitted);
    }
    assert_eq!(snapshot["schematic"]["symbols"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["schematic"]["wires"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["schematic"]["netlist"]["nets"][0]["terminals"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["board"]["footprints"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["boards"][0]["tracks"].as_array().unwrap().len(), 1);
}

/// BOMs name a pinned package even before its footprint has been placed on the board.
#[test]
fn bom_uses_the_pinned_assignment_when_the_component_is_unplaced() {
    let mut session = session();
    let board = board_id(&session);
    let (_symbol, component) = resistor(&mut session, 0);
    edit(
        &mut session,
        crate::board::assignment::ASSIGN_FOOTPRINT,
        json!({
            "component":component,
            "library":"starter",
            "definition":"resistor-0402",
            "definition_revision":4,
            "mapping":{"1":"2","2":"1"}
        }),
    );

    assert!(session.project().unwrap().board.footprints().is_empty());
    let output = report(&mut session, &board, Value::Null);
    let bom = contents(&output, "BOM.csv");
    assert!(bom.lines().nth(1).unwrap().contains("\"resistor-0402\""), "the pinned package is exported: {bom}");
}

/// CSV output marks formula/control prefixes as text and preserves legitimate signed values.
#[test]
fn csv_cells_neutralize_formula_and_control_prefixes() {
    let mut output = String::new();
    csv_row(&mut output, &["=1+1", "@SUM(1,1)", "\ttext", "\rtext", "\ntext", "+5V", "-5V", "-0.250000"]);
    let expected = "\"'=1+1\",\"'@SUM(1,1)\",\"'\ttext\",\"'\rtext\",\"'\ntext\",\"+5V\",\"-5V\",\"-0.250000\"\r\n".to_owned();
    assert_eq!(output, expected);
}

/// Base DNP, BOM exclusion, position exclusion and board exclusion stay independent.
#[test]
fn base_dnp_and_output_specific_exclusions_are_independent() {
    let mut session = session();
    let board = board_id(&session);
    let (r1_symbol, r1) = resistor(&mut session, 0);
    let (r2_symbol, _r2) = resistor(&mut session, 25_400_000);
    let (r3_symbol, _r3) = resistor(&mut session, 50_800_000);
    let (r4_symbol, _r4) = resistor(&mut session, 76_200_000);
    footprint(&mut session, &r1_symbol, &board, 0);
    footprint(&mut session, &r2_symbol, &board, 25_400_000);
    footprint(&mut session, &r3_symbol, &board, 50_800_000);
    set_attributes(&mut session, &r1_symbol, "R1", attributes(true, false, false, false));
    set_attributes(&mut session, &r2_symbol, "R2", attributes(false, true, false, false));
    set_attributes(&mut session, &r3_symbol, "R3", attributes(false, false, false, true));
    set_attributes(&mut session, &r4_symbol, "R4", attributes(false, false, true, false));

    let fitted = create_variant(&mut session, "Fit R1", &board);
    population(&mut session, fitted, &r1, json!(true), &board);
    let base = report(&mut session, &board, Value::Null);
    let variant = report(&mut session, &board, json!(fitted));
    let base_bom = contents(&base, "BOM.csv");
    let base_placement = contents(&base, "Placement.csv");
    let variant_bom = contents(&variant, "BOM.csv");
    let variant_placement = contents(&variant, "Placement.csv");

    assert!(!has_reference(base_bom, "R1"));
    assert!(!has_reference(base_placement, "R1"), "base DNP excludes placement data too");
    assert!(!has_reference(base_bom, "R2"));
    assert!(has_reference(base_placement, "R2"), "BOM exclusion does not exclude placement data");
    assert!(has_reference(base_bom, "R3"), "position exclusion does not exclude BOM data");
    assert!(!has_reference(base_placement, "R3"));
    assert!(!has_reference(base_bom, "R4"), "board exclusion is independent of DNP and BOM exclusion");
    assert!(has_reference(variant_bom, "R1"));
    assert!(has_reference(variant_placement, "R1"));

    population(&mut session, fitted, &r1, Value::Null, &board);
    let inherited = report(&mut session, &board, json!(fitted));
    assert!(!has_reference(contents(&inherited, "BOM.csv"), "R1"), "null restores inheritance from base DNP");
}

/// Install the checked-in native dual-amplifier data without a network library dependency.
fn dual_unit_session() -> (Session, String, String, String) {
    const SYMBOL: &str = include_str!("../../../../../examples/native-parts/symbols/dual-amplifier.json");
    let mut session = session();
    session
        .execute(
            "library.load_source",
            &json!({"source":"unit-tests","documents":{
                "library.json":r#"{"format":"pcbcraft-library/1","id":"unit-tests","name":"Unit tests","license":"MIT OR Apache-2.0"}"#,
                "symbols/dual.json":SYMBOL,
            }}),
        )
        .unwrap();
    let definition = session.libraries.symbol("unit-tests", "dual-amplifier", 1).unwrap();
    let digest = crate::library::sources::symbol_digest(&definition).unwrap();
    edit(
        &mut session,
        PLACE_SYMBOL,
        json!({"library":"unit-tests","definition":"dual-amplifier","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    let original = session.project().unwrap().symbols()[0].id();
    let component = session.project().unwrap().symbols()[0].component_id();
    edit(&mut session, crate::sheets::CREATE_SHEET, json!({"name":"Unit B"}));
    let second_sheet = session.project().unwrap().sheets().nth(1).unwrap().target.id().to_string();
    edit(&mut session, PLACE_UNIT, json!({"sheet":second_sheet,"component":component,"unit":"b","x_nm":0,"y_nm":0}));
    (session, original, component, second_sheet)
}

/// Shared component IDs survive unit placement, reference rename, deletion, undo and offline persistence.
#[test]
fn multiunit_reference_rename_deletion_undo_and_offline_reopen_preserve_variant_identity() {
    let (mut session, original, component, second_sheet) = dual_unit_session();
    let board = board_id(&session);
    let first_sheet = session.project().unwrap().sheet_target().id().to_string();
    let part_number = component.strip_prefix("component-").unwrap().parse::<u64>().unwrap();
    let variant = create_variant(&mut session, "No amplifier", &board);
    population(&mut session, variant, &component, json!(false), &board);
    edit(&mut session, SET_SYMBOL_FIELDS, json!({"sheet":first_sheet,"symbol":original,"reference":"U42","value":"Dual op amp"}));

    let bom = contents(&report(&mut session, &board, json!(variant)), "BOM.csv").to_owned();
    assert!(!has_reference(&bom, "U42"), "the override follows the shared logical component");
    population(&mut session, variant, &component, Value::Null, &board);
    let fitted_bom = contents(&report(&mut session, &board, json!(variant)), "BOM.csv").to_owned();
    assert_eq!(fitted_bom.lines().count(), 2, "two unit drawings contribute exactly one BOM row");
    assert!(has_reference(&fitted_bom, "U42"));
    population(&mut session, variant, &component, json!(false), &board);

    let snapshot = session.snapshot().unwrap();
    let documents = encode_documents(snapshot.clone()).unwrap();
    let variant_path = snapshot.assembly_variant_document_path(variant).unwrap();
    assert!(documents.contains_key(&variant_path));
    let project_document: Value = serde_json::from_slice(&documents["project.json"]).unwrap();
    assert_eq!(project_document["format"], "pcbcraft-project/18");
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents, "split documents round-trip byte for byte offline",);
    let packed = encode_packed(snapshot.clone()).unwrap();
    let packed_reopened = decode_packed(&packed).unwrap().project;
    assert_eq!(encode_documents(packed_reopened).unwrap(), documents);

    let mut reopened = Session::default();
    reopened.open_snapshot(decode_documents(&documents).unwrap());
    assert_eq!(reopened.project().unwrap().all_symbols().count(), 2);
    assert_eq!(reopened.project().unwrap().components().count(), 1);
    assert_eq!(contents(&report(&mut reopened, &board, json!(variant)), "BOM.csv").lines().count(), 1, "the saved DNP override survives reopen",);

    edit(&mut reopened, DELETE_SYMBOL, json!({"sheet":first_sheet,"symbol":original}));
    assert_eq!(reopened.project().unwrap().all_symbols().count(), 1);
    assert_eq!(reopened.project().unwrap().components().count(), 1);
    assert_eq!(reopened.project().unwrap().assembly_variants().next().unwrap().population.get(&part_number), Some(&false));

    let remaining = reopened.project().unwrap().all_symbols().next().unwrap().id();
    edit(&mut reopened, DELETE_SYMBOL, json!({"sheet":second_sheet,"symbol":remaining}));
    assert_eq!(reopened.project().unwrap().components().count(), 0);
    assert!(reopened.project().unwrap().assembly_variants().next().unwrap().population.is_empty());
    reopened.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(reopened.project().unwrap().components().count(), 1);
    assert_eq!(reopened.project().unwrap().assembly_variants().next().unwrap().population.get(&part_number), Some(&false));
    reopened.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(reopened.project().unwrap().components().count(), 0);

    reopened.apply("edit.undo", &Value::Null).unwrap();
    edit(&mut reopened, DELETE_ASSEMBLY_VARIANT, json!({"board":board,"variant":variant}));
    assert_eq!(reopened.project().unwrap().assembly_variants().count(), 0);
    reopened.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(reopened.project().unwrap().assembly_variants().count(), 1);
}

/// Invalid names, stale revisions, unknown identities and corrupt native references fail visibly.
#[test]
fn invalid_and_stale_variant_references_are_rejected_by_commands_exports_and_storage() {
    let mut session = session();
    let board = board_id(&session);
    let (_symbol, _component) = resistor(&mut session, 0);
    let variant = create_variant(&mut session, "Proto", &board);
    let normalized = create_variant(&mut session, "  Trimmed name  ", &board);
    assert_eq!(session.project().unwrap().assembly_variants().find(|item| item.id == normalized).unwrap().name, "Trimmed name");

    let long_name = "x".repeat(crate::MAX_NAME + 1);
    for name in ["Base", "\n", long_name.as_str(), "Trimmed name"] {
        let mut params = json!({"board":board,"variant":variant,"name":name});
        params["expected_revision"] = json!(session.revision());
        assert!(session.apply(SET_ASSEMBLY_VARIANT, &params).is_err(), "accepted invalid or duplicate name {name:?}");
        assert_eq!(session.project().unwrap().assembly_variants().find(|item| item.id == variant).unwrap().name, "Proto");
    }
    let stale_revision = session.revision().saturating_sub(1);
    assert!(session.apply(CREATE_ASSEMBLY_VARIANT, &json!({"board":board,"name":"Stale","expected_revision":stale_revision})).is_err());
    assert!(
        session
            .apply(
                SET_ASSEMBLY_VARIANT,
                &json!({"board":board,"variant":variant,"component":"component-999999","fitted":false,"expected_revision":session.revision()})
            )
            .is_err()
    );
    assert!(
        session
            .apply(SET_ASSEMBLY_VARIANT, &json!({"board":board,"variant":Uuid::max(),"name":"Unknown","expected_revision":session.revision()}))
            .is_err()
    );
    assert!(
        session.execute(crate::board::assembly::EXPORT_ASSEMBLY, &json!({"board":board})).is_err(),
        "variant selection is mandatory, including for Base"
    );
    assert!(
        matches!(session.export_assembly(&json!({"board":board})), Err(crate::CommandError::InvalidParams(message)) if message.contains("variant")),
        "direct callers must provide the variant key and use null for Base"
    );
    assert!(session.execute(crate::board::assembly::EXPORT_ASSEMBLY, &json!({"board":board,"variant":Uuid::max()})).is_err());

    let mut documents = encode_documents(session.snapshot().unwrap()).unwrap();
    let path = session.project().unwrap().assembly_variant_document_path(variant).unwrap();
    let mut document: Value = serde_json::from_slice(&documents[&path]).unwrap();
    document["population"] = json!({"999999":false});
    documents.insert(path.clone(), serde_json::to_vec_pretty(&document).unwrap());
    let error = decode_documents(&documents).unwrap_err();
    assert!(error.contains("component-999999"), "dangling persisted ID is called out: {error}");

    let clean = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut missing = clean.clone();
    missing.remove(&path);
    assert!(decode_documents(&missing).unwrap_err().contains(&path));

    let mut stale = (*session.snapshot().unwrap()).clone();
    Arc::make_mut(&mut stale.board).assembly_variants.get_mut(&variant).unwrap().population.insert(999_999, false);
    let mut corrupted = Session::default();
    corrupted.open_snapshot(Arc::new(stale));
    let error = corrupted.export_assembly(&json!({"board":board,"variant":variant})).unwrap_err().to_string();
    assert!(error.contains("component-999999"), "export reports a stale live reference: {error}");
}

/// Format 18 cannot be emitted until the decoder-required sheet occurrence document exists.
#[test]
fn variants_require_adopted_sheet_occurrences_before_encoding() {
    let mut session = session();
    let board = board_id(&session);
    create_variant(&mut session, "Proto", &board);
    let mut project = (*session.snapshot().unwrap()).clone();
    project.circuit_use = None;

    let error = encode_documents(Arc::new(project)).unwrap_err();
    assert!(error.contains("adopted sheet occurrences"), "encoder explains the missing decoder-required document: {error}");
}
