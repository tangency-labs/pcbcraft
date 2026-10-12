use pcbcraft_library::{Graphic, read_footprint, read_symbols};

#[test]
fn real_device_symbols_preserve_pin_types_and_graphics() -> Result<(), Box<dyn std::error::Error>> {
    let symbols = read_symbols(include_str!("fixtures/device.kicad_sym"))?;
    let resistor = symbols.iter().find(|s| s.name == "R").ok_or("R missing")?;
    assert_eq!(resistor.reference, "R");
    assert_eq!(resistor.pins.len(), 2);
    assert!(resistor.unsupported.is_empty());
    assert!(resistor.pins.iter().all(|p| p.electrical_type == "passive"));
    assert!(resistor.pins.iter().any(|p| p.position.y < 0.0));
    assert!(
        resistor
            .graphics
            .iter()
            .any(|g| matches!(g.primitive, Graphic::Rectangle { .. }))
    );
    let led = symbols
        .iter()
        .find(|s| s.name == "LED")
        .ok_or("LED missing")?;
    assert_eq!(led.pins.len(), 2);
    assert!(led.pins.iter().any(|p| p.name == "A"));
    assert!(!led.graphics.is_empty());
    assert!(!led.raw_source.starts_with("(kicad_symbol_lib"));
    Ok(())
}
#[test]
fn real_ic_resolves_inheritance_and_units() -> Result<(), Box<dyn std::error::Error>> {
    let symbols = read_symbols(include_str!("fixtures/lm358.kicad_sym"))?;
    let ic = symbols
        .iter()
        .find(|s| s.name == "LM358")
        .ok_or("LM358 missing")?;
    assert_eq!(ic.extends.as_deref(), Some("LM2904"));
    assert_eq!(ic.value, "LM358");
    assert_eq!(ic.pins.len(), 8);
    assert!(
        ic.pins
            .iter()
            .any(|p| p.unit == 3 && p.electrical_type == "power_in")
    );
    assert!(ic.graphics.iter().any(|g| g.unit == 2));
    Ok(())
}
#[test]
fn real_0805_preserves_roundrect_pads_and_layers() -> Result<(), Box<dyn std::error::Error>> {
    let fp = read_footprint(include_str!("fixtures/R_0805_2012Metric.kicad_mod"))?;
    assert_eq!(fp.pads.len(), 2);
    assert!(fp.pads.iter().all(|p| p.shape == "roundrect"
        && p.pad_type == "smd"
        && p.layers.iter().any(|l| l == "F.Cu")));
    assert!(fp.pads[0].position.x < 0.0);
    assert!(fp.pads[0].roundrect_ratio.is_some());
    assert!(
        fp.graphics
            .iter()
            .any(|g| matches!(g,Graphic::Rectangle{layer,..}if layer=="F.CrtYd"))
    );
    assert!(fp.unsupported.is_empty());
    assert!(fp.raw_source.contains("(model"));
    Ok(())
}

#[test]
#[ignore = "requires installed KiCad 10 libraries in Linux/WSL"]
fn installed_library_catalogs() -> Result<(), Box<dyn std::error::Error>> {
    for name in ["Device", "Amplifier_Operational"] {
        let text = std::fs::read_to_string(format!("/usr/share/kicad/symbols/{name}.kicad_sym"))?;
        let symbols = read_symbols(&text)?;
        assert!(symbols.len() > 100);
    }
    Ok(())
}
#[test]
fn hostile_inputs_and_native_dto_validation() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        "(",
        "(kicad_symbol_lib))",
        "(kicad_symbol_lib (symbol A (extends B)) (symbol B (extends A)))",
        "(kicad_symbol_lib (symbol A (extends Missing)))",
    ] {
        assert!(read_symbols(source).is_err());
    }
    let deep = format!("{}{}", "(".repeat(70), ")".repeat(70));
    assert!(read_symbols(&deep).is_err());
    assert!(
        read_footprint("(footprint X (pad 1 smd circle (at NaN 0) (size 1 1) (layers F.Cu)))")
            .is_err()
    );
    let mut fp = read_footprint(include_str!("fixtures/R_0805_2012Metric.kicad_mod"))?;
    fp.pads[0].size.x = f64::INFINITY;
    assert!(fp.validate().is_err());
    let mut symbols = read_symbols(include_str!("fixtures/device.kicad_sym"))?;
    let s = symbols.first_mut().ok_or("no symbols")?;
    s.pins.push(s.pins[0].clone());
    assert!(s.validate().is_err());
    Ok(())
}
#[test]
fn arbitrary_pin_numbers_unknown_geometry_and_drills() -> Result<(), Box<dyn std::error::Error>> {
    let s = read_symbols(
        "(kicad_symbol_lib (symbol X (symbol X_1_1 (pin input line (at 1 2 90) (length 2) (name CLK) (number A12)))))",
    )?;
    assert_eq!(s[0].pins[0].number, "A12");
    assert_eq!(s[0].pins[0].position.y, -2.0);
    let fp = read_footprint(
        "(footprint X (pad A12 thru_hole custom (at 1 2 90) (size 2 3) (drill oval 1 2 (offset 0.1 0.2)) (layers *.Cu *.Mask) (primitives (gr_poly))))",
    )?;
    assert_eq!(fp.pads[0].position.y, 2.0);
    assert_eq!(fp.pads[0].rotation, 90.0);
    assert_eq!(fp.pads[0].drill.as_ref().ok_or("no drill")?.size.y, 2.0);
    assert!(!fp.unsupported.is_empty());
    Ok(())
}

#[test]
fn inheritance_overrides_properties_and_replaces_local_units()
-> Result<(), Box<dyn std::error::Error>> {
    let symbols = read_symbols(
        r#"(kicad_symbol_lib
        (symbol Parent (property "Reference" "U") (property "Description" "base")
          (symbol Parent_1_1 (pin passive line (at 0 0) (length 1) (name OLD) (number 1)))
          (symbol Parent_0_1 (rectangle (start -1 -1) (end 1 1))))
        (symbol Child (extends "Parent") (property "Description" "override")
          (symbol Child_1_1 (pin input line (at 1 2 90) (length 2) (name NEW) (number A))))
    )"#,
    )?;
    let child = symbols
        .iter()
        .find(|s| s.name == "Child")
        .ok_or("missing child")?;
    assert_eq!(child.reference, "U");
    assert_eq!(child.description, "override");
    assert_eq!(child.pins.len(), 1);
    assert_eq!(child.pins[0].number, "A");
    assert_eq!(child.graphics[0].unit, 0);
    assert!(read_symbols("(kicad_symbol_lib (symbol X (symbol X_1_1) (symbol X_1_1)))").is_err());
    Ok(())
}
