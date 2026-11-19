use super::*;
use crate::board::{
    drc::tests::outline,
    track::tests::{copper_layers, route, route_with_vias, routed_fixture, view},
};
use std::io::Read as _;

/// Two identical lands exercise aperture reuse as their placement rotates and flips.
fn aperture_fixture(shape: PadShape, width: i64, height: i64, pad_rotation: u16, rotation: u16, bottom: bool) -> Session {
    aperture_fixture_with_pads(&[(shape, width, height, pad_rotation); 2], rotation, bottom)
}

/// Author lands through the library editor, load its production encoding, then assign
/// and place the package through the same commands used by live and headless clients.
fn aperture_fixture_with_pads(pads: &[(PadShape, i64, i64, u16)], rotation: u16, bottom: bool) -> Session {
    aperture_fixture_at(pads, rotation, bottom, Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, false)
}

/// Author the same command-built fixture with explicit local/placement centers and copper binding.
/// Rounded rectangles use a quarter of their short side as the corner radius.
fn aperture_fixture_at(
    pads: &[(PadShape, i64, i64, u16)],
    rotation: u16,
    bottom: bool,
    offset: Point,
    position: Point,
    through_hole: bool,
) -> Session {
    let pads: Vec<_> = pads
        .iter()
        .map(|&(shape, width, height, pad_rotation)| {
            let radius = if shape == PadShape::RoundedRectangle { (width.min(height) / 4).max(1) } else { 0 };
            (shape, width, height, pad_rotation, radius)
        })
        .collect();
    aperture_fixture_with_radii(&pads, rotation, bottom, offset, position, through_hole)
}

/// Author command-built lands with an explicit corner radius per pad.
fn aperture_fixture_with_radii(
    pads: &[(PadShape, i64, i64, u16, i64)],
    rotation: u16,
    bottom: bool,
    offset: Point,
    position: Point,
    through_hole: bool,
) -> Session {
    use crate::authoring::{Action, Item, Kind, Primitive};
    let mut editor = crate::authoring::Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    editor.execute(editor.revision(), Action::Delete { objects: vec![2, 3] }).unwrap();
    let Primitive::Pad { pad: template } = editor.snapshot().objects[&1].clone() else { panic!("pad") };
    for (index, &(shape, width, height, pad_rotation, corner_radius_nm)) in pads.iter().enumerate() {
        let mut pad = template.clone();
        pad.number = (index + 1).to_string();
        pad.center = Point { x_nm: index as i64 * 2_000_000 + offset.x_nm, y_nm: offset.y_nm };
        if through_hole {
            pad.copper = crate::board::definition::CopperBinding::AllCopper;
            pad.drill_nm = Some(width.min(height) / 2);
        }
        pad.shape = shape;
        pad.corner_radius_nm = corner_radius_nm;
        pad.width_nm = width;
        pad.height_nm = height;
        pad.rotation = pad_rotation;
        let primitive = Primitive::Pad { pad };
        let action = if index == 0 { Action::Replace { object: 1, primitive } } else { Action::Insert { primitive } };
        editor.execute(editor.revision(), action).unwrap();
    }
    let definition = editor.snapshot().footprint().unwrap();
    let document = editor.snapshot().export_native().unwrap().to_string();
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let manifest =
        json!({"format":crate::library::LIBRARY_FORMAT,"id":definition.library,"name":"Aperture test library","license":definition.license})
            .to_string();
    s.execute("library.load_source", &json!({"source":"apertures","documents":{"library.json":manifest,"footprints/lands.json":document}})).unwrap();
    // Include interior copper in the plated-land fixture.
    let copper_layers = if through_hole { 4 } else { 2 };
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":copper_layers,"expected_revision":s.revision()})).unwrap();
    s.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let component = s.project().unwrap().components().next().unwrap().id();
    s.apply(
        crate::ASSIGN_FOOTPRINT,
        &json!({"component":component,"library":definition.library,"definition":definition.id,
        "definition_revision":definition.revision,"mapping":{"1":"1","2":"2"},
        "unconnected_pads":(3..=pads.len()).map(|number|number.to_string()).collect::<Vec<_>>(),"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(
        crate::PLACE_ASSIGNED_FOOTPRINT,
        &json!({"component":component,"x_nm":position.x_nm,"y_nm":position.y_nm,"expected_revision":s.revision()}),
    )
    .unwrap();
    let footprint = view(&s).footprints().next().unwrap().id;
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":footprint,"x_nm":position.x_nm,"y_nm":position.y_nm,
        "rotation":rotation,"side":if bottom {"back"} else {"front"},"expected_revision":s.revision()}),
    )
    .unwrap();
    outline(&mut s);
    s
}

/// Test-only read-back of the emitted Gerber subset, independent of the writer's aperture key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadAperture {
    kind: char,
    width: i64,
    height: i64,
}

#[derive(Debug, Default)]
struct ReadGerber {
    apertures: BTreeMap<usize, ReadAperture>,
    flashes: Vec<(Point, ReadAperture)>,
    strokes: Vec<(Point, Point, ReadAperture)>,
    regions: Vec<Vec<Point>>,
    rounded: BTreeMap<usize, ReadRounded>,
    rounded_flashes: Vec<(Point, usize)>,
}

#[derive(Debug, Default)]
struct ReadRounded {
    circles: Vec<(i64, Point)>,
    strips: Vec<(i64, Point, Point)>,
    outline: Vec<Point>,
}

/// Parse a strictly positive unsigned millimeter dimension with exactly six decimals.
fn read_mm(value: &str) -> Result<i64, &'static str> {
    let invalid = "invalid aperture dimension";
    let (whole, fraction) = value.split_once('.').ok_or(invalid)?;
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) || fraction.len() != 6 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid);
    }
    let size = whole
        .parse::<i64>()
        .map_err(|_| invalid)?
        .checked_mul(1_000_000)
        .and_then(|w| fraction.parse::<i64>().ok().and_then(|f| w.checked_add(f)))
        .ok_or(invalid)?;
    if size <= 0 {
        return Err(invalid);
    }
    Ok(size)
}

/// Parse a signed, canonical macro coordinate in millimeters, on the nanometer grid.
fn read_macro_coordinate(value: &str) -> Result<i64, &'static str> {
    if value == "0.000000" {
        return Ok(0);
    }
    if let Some(magnitude) = value.strip_prefix('-') { read_mm(magnitude).map(|n| -n) } else { read_mm(value) }
}

/// Decode only the emitted fixed-parameter macro subset (Ucamco Gerber 2026.05 §§4.5.1.3,
/// 4.5.1.4, 4.5.1.6). Exposure must be on; zero rotation means coordinates are already
/// in the aperture's axes. Refuse unsupported primitive parameters rather than ignoring them.
fn read_rounded_macro(lines: &mut std::str::Lines<'_>) -> Result<ReadRounded, &'static str> {
    let mut read = ReadRounded::default();
    let point = |x, y| Ok::<_, &'static str>(Point { x_nm: read_macro_coordinate(x)?, y_nm: read_macro_coordinate(y)? });
    for line in lines {
        if line == "%" {
            return if read.circles.is_empty() { Err("missing macro circles") } else { Ok(read) };
        }
        let fields: Vec<_> = line.strip_suffix('*').ok_or("invalid macro primitive")?.split(',').collect();
        match fields.as_slice() {
            ["1", "1", diameter, x, y, "0"] => read.circles.push((read_mm(diameter)?, point(x, y)?)),
            ["20", "1", width, x, y, end_x, end_y, "0"] => {
                let (start, end) = (point(x, y)?, point(end_x, end_y)?);
                if start == end {
                    return Err("collapsed macro strip");
                }
                read.strips.push((read_mm(width)?, start, end));
            }
            ["4", "1", "4", xy @ .., "0"] if read.outline.is_empty() && xy.len() == 10 => {
                for &[x, y] in xy.as_chunks::<2>().0 {
                    read.outline.push(point(x, y)?);
                }
                if read.outline.first() != read.outline.last() {
                    return Err("open macro outline");
                }
            }
            _ => return Err("unsupported macro primitive"),
        }
    }
    Err("unclosed macro")
}

/// Read canonical signed integer coordinates in the writer's absolute 4.6 millimeter format.
fn read_coordinate(value: &str) -> Result<i64, &'static str> {
    let number = value.parse::<i64>().map_err(|_| "invalid coordinate")?;
    if number.to_string() != value || number.unsigned_abs() > 9_999_999_999 {
        return Err("invalid coordinate");
    }
    Ok(number)
}

/// Read the fixture subset: file identity, modes, standard and rounded apertures, operation
/// order and structural contour closure. This is not a general CAM reader.
fn try_read_gerber(text: &str, expected_function: &str, expected_polarity: &str) -> Result<ReadGerber, &'static str> {
    if text.contains('\r') || !text.ends_with('\n') {
        return Err("invalid line endings");
    }
    let mut lines = text.lines();
    let software = format!("%TF.GenerationSoftware,PCBcraft,PCBcraft,{}*%", env!("CARGO_PKG_VERSION"));
    for expected in ["G04 PCBcraft fabrication output*", software.as_str(), "%TF.SameCoordinates,Original*%"] {
        if lines.next() != Some(expected) {
            return Err("invalid header");
        }
    }
    let function = lines
        .next()
        .and_then(|line| line.strip_prefix("%TF.FileFunction,"))
        .and_then(|line| line.strip_suffix("*%"))
        .ok_or("invalid file function")?;
    let parts: Vec<_> = function.split(',').collect();
    let valid_function = match parts.as_slice() {
        ["Copper", layer, "Top" | "Bot" | "Inr"] => {
            layer.strip_prefix('L').and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| (1..=64).contains(&n))
        }
        ["Soldermask" | "Paste" | "Legend", "Top" | "Bot"] | ["Profile", "NP"] => true,
        _ => false,
    };
    if !valid_function {
        return Err("invalid file function");
    }
    if function != expected_function {
        let (kind, detail) = expected_function.split_once(',').unwrap();
        if parts[0] != kind {
            return Err("wrong file function");
        }
        if kind == "Copper" && Some(parts[1]) != detail.split(',').next() {
            return Err("wrong copper ordinal");
        }
        return Err("wrong file side");
    }
    let polarity = format!("%TF.FilePolarity,{expected_polarity}*%");
    if lines.next() != Some(polarity.as_str()) {
        return Err("invalid file polarity");
    }
    for expected in ["%FSLAX46Y46*%", "%MOMM*%", "%LPD*%", "G01*"] {
        if lines.next() != Some(expected) {
            return Err("unsupported coordinate mode");
        }
    }
    let mut read = ReadGerber::default();
    let mut macros = BTreeMap::new();
    let mut selected = None;
    let mut position = None;
    let mut region: Option<Vec<Point>> = None;
    let mut body = false;
    while let Some(line) = lines.next() {
        if line == "M02*" {
            if region.is_some() {
                return Err("unclosed region");
            }
            if lines.next().is_some() {
                return Err("records after end");
            }
            if !macros.is_empty() {
                return Err("unused macro definition");
            }
            return Ok(read);
        }
        if let Some(name) = line.strip_prefix("%AMRoundRect") {
            if body {
                return Err("late macro definition");
            }
            let name = name.strip_suffix('*').and_then(read_aperture_code).ok_or("invalid macro name")?;
            if macros.contains_key(&name) || read.rounded.contains_key(&name) {
                return Err("duplicate macro definition");
            }
            macros.insert(name, read_rounded_macro(&mut lines)?);
            continue;
        }
        if let Some(definition) = line.strip_prefix("%ADD") {
            if body {
                return Err("late aperture definition");
            }
            let definition = definition.strip_suffix("*%").ok_or("invalid aperture")?;
            if let Some((code, name)) = definition.split_once("RoundRect") {
                let code = read_aperture_code(code).ok_or("invalid aperture code")?;
                let name = read_aperture_code(name).ok_or("invalid macro name")?;
                let aperture = macros.remove(&name).ok_or("undefined macro")?;
                if name != code || read.apertures.contains_key(&code) || read.rounded.insert(code, aperture).is_some() {
                    return Err("invalid macro aperture definition");
                }
                continue;
            }
            let (code_kind, dimensions) = definition.split_once(',').ok_or("invalid aperture")?;
            let kind = code_kind.chars().last().ok_or("invalid aperture")?;
            if !matches!(kind, 'C' | 'R' | 'O') {
                return Err("unsupported aperture");
            }
            let code = read_aperture_code(&code_kind[..code_kind.len() - 1]).ok_or("invalid aperture code")?;
            let dimensions = dimensions.split('X').map(read_mm).collect::<Result<Vec<_>, _>>()?;
            if dimensions.len() != if kind == 'C' { 1 } else { 2 } {
                return Err("invalid aperture arity");
            }
            let aperture = ReadAperture { kind, width: dimensions[0], height: *dimensions.last().unwrap() };
            if read.rounded.contains_key(&code) || read.apertures.insert(code, aperture).is_some() {
                return Err("duplicate aperture");
            }
            continue;
        }
        body = true;
        if let Some(code) = line.strip_prefix('D') {
            if region.is_some() {
                return Err("selection in region");
            }
            let code = code.strip_suffix('*').and_then(read_aperture_code).ok_or("invalid selection")?;
            if !read.apertures.contains_key(&code) && !read.rounded.contains_key(&code) {
                return Err("undefined aperture");
            }
            selected = Some(code);
        } else if line == "G36*" {
            if region.replace(Vec::new()).is_some() {
                return Err("nested region");
            }
        } else if line == "G37*" {
            let points = region.take().ok_or("region end without start")?;
            if points.len() < 4 || points.first() != points.last() {
                return Err("region contour not closed");
            }
            read.regions.push(points);
        } else if let Some(coordinates) = line.strip_prefix('X') {
            let (x, rest) = coordinates.split_once('Y').ok_or("invalid coordinate")?;
            let (y, operation) = rest.split_once('D').ok_or("invalid coordinate")?;
            let point = Point { x_nm: read_coordinate(x)?, y_nm: read_coordinate(y)? };
            if let Some(points) = region.as_mut() {
                if operation != if points.is_empty() { "02*" } else { "01*" } {
                    return Err("invalid region operation");
                }
                points.push(point);
            } else {
                match operation {
                    "03*" => {
                        let code = selected.ok_or("undefined aperture")?;
                        if let Some(&aperture) = read.apertures.get(&code) {
                            read.flashes.push((point, aperture));
                        } else {
                            read.rounded_flashes.push((point, code));
                        }
                    }
                    "01*" => {
                        let aperture = *read.apertures.get(&selected.ok_or("undefined aperture")?).ok_or("unsupported stroke aperture")?;
                        if aperture.kind != 'C' {
                            return Err("unsupported stroke aperture");
                        }
                        read.strokes.push((position.ok_or("draw without position")?, point, aperture));
                    }
                    "02*" => (),
                    _ => return Err("unsupported coordinate operation"),
                }
            }
            position = Some(point);
        } else {
            return Err("unsupported record");
        }
    }
    Err("missing end record")
}

/// Parse an unsigned D-code of at least 10 with no sign or leading zeroes.
fn read_aperture_code(value: &str) -> Option<usize> {
    let code = value.parse::<usize>().ok()?;
    (code >= 10 && code.to_string() == value).then_some(code)
}

/// Filename suffix and independent expected X2 file function and polarity.
type GerberFile = (&'static str, &'static str, &'static str);
const TOP_COPPER: GerberFile = ("-Top.gbr", "Copper,L1,Top", "Positive");
const BOTTOM_PASTE: GerberFile = ("-Paste_Bottom.gbr", "Paste,Bot", "Positive");
const TOP: [GerberFile; 3] = [TOP_COPPER, ("-Mask_Top.gbr", "Soldermask,Top", "Negative"), ("-Paste_Top.gbr", "Paste,Top", "Positive")];
const BOTTOM: [GerberFile; 3] = [("-Bottom.gbr", "Copper,L2,Bot", "Positive"), ("-Mask_Bottom.gbr", "Soldermask,Bot", "Negative"), BOTTOM_PASTE];

/// Return the two-layer fixture's copper, mask and paste expectations in that order
/// for the given side. Bottom copper is L2.
fn surface_files(bottom: bool) -> [GerberFile; 3] {
    if bottom { BOTTOM } else { TOP }
}

fn read_gerber(outputs: &[Output], (suffix, function, polarity): GerberFile) -> ReadGerber {
    try_read_gerber(file(outputs, suffix), function, polarity).unwrap_or_else(|error| panic!("{suffix}: {error}"))
}

/// A mixed, command-built package uses C/R/O and a rounded macro in one file, including the swapped rectangle.
/// Ucamco Gerber Layer Format Specification 2026.05 section 4.4 defines these standard templates.
#[test]
fn mixed_pad_shapes_use_standard_flashes_with_resolved_sizes() {
    let s = aperture_fixture_with_pads(
        &[
            (PadShape::Circle, 400_000, 400_000, 0),
            (PadShape::Rectangle, 800_000, 400_000, 0),
            (PadShape::Rectangle, 800_000, 400_000, 90),
            (PadShape::Oval, 800_000, 400_000, 0),
            (PadShape::RoundedRectangle, 800_000, 400_000, 0),
        ],
        0,
        false,
    );
    let outputs = view(&s).fabrication_outputs().unwrap();
    let read = read_gerber(&outputs, TOP_COPPER);
    assert!(read.strokes.is_empty() && read.regions.is_empty());
    assert_eq!(read.apertures.len(), 4);
    assert_eq!(read.rounded.len(), 1);
    assert_eq!(read.rounded_flashes, [(Point { x_nm: 38_000_001, y_nm: 25_000_003 }, 14)]);
    let expected = [('C', 400_000, 400_000), ('R', 800_000, 400_000), ('R', 400_000, 800_000), ('O', 800_000, 400_000)];
    assert_eq!(read.flashes.len(), expected.len());
    for (((center, aperture), pad), (kind, width, height)) in read.flashes.iter().zip(view(&s).footprints().next().unwrap().pads).zip(expected) {
        assert_eq!(*center, Point { x_nm: pad.center.x_nm, y_nm: -pad.center.y_nm });
        assert_eq!(*aperture, ReadAperture { kind, width, height });
    }
    assert_eq!(outputs, view(&s).fabrication_outputs().unwrap());
    assert_eq!(zip(&outputs).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
}

/// Untransformed standard apertures retain global X/Y dimensions (Ucamco Gerber 2026.05 section 4.4).
/// Rectangle flash extents and half-grid fallback regions match the resolved corners exactly;
/// equal-parity obround flashes match the land capsule, while mixed-parity obrounds retain
/// the existing rounded-endpoint stroke rather than claiming exact contour equivalence.
#[test]
fn standard_pad_apertures_read_back_on_copper_mask_and_paste() {
    for (shape, width, height, kind) in [
        (PadShape::Circle, 400_001, 400_001, 'C'),
        (PadShape::Rectangle, 800_000, 400_002, 'R'),
        (PadShape::Rectangle, 800_001, 400_003, 'R'),
        (PadShape::Rectangle, 800_000, 400_003, 'R'),
        (PadShape::Oval, 400_003, 800_001, 'O'),
        (PadShape::Oval, 400_001, 400_001, 'O'),
        (PadShape::Rectangle, 1, 2, 'R'),
        (PadShape::Oval, 2, 1, 'O'),
    ] {
        for (pad_rotation, rotation) in [(0, 0), (90, 0), (0, 90), (90, 90), (0, 270)] {
            for bottom in [false, true] {
                let s = aperture_fixture(shape, width, height, pad_rotation, rotation, bottom);
                let outputs = view(&s).fabrication_outputs().unwrap();
                let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
                for output in surface_files(bottom) {
                    let read = read_gerber(&outputs, output);
                    if shape == PadShape::Oval && width % 2 != height % 2 {
                        assert!(read.flashes.is_empty() && read.regions.is_empty());
                        assert_eq!(read.strokes.len(), pads.len());
                        for ((a, b, aperture), pad) in read.strokes.iter().zip(&pads) {
                            let corners = pad.copper.unwrap();
                            let x = (corners[1].x_nm - corners[0].x_nm).abs() + (corners[2].x_nm - corners[1].x_nm).abs();
                            let y = (corners[1].y_nm - corners[0].y_nm).abs() + (corners[2].y_nm - corners[1].y_nm).abs();
                            assert_eq!(*aperture, ReadAperture { kind: 'C', width: x.min(y), height: x.min(y) });
                            assert_eq!((a.x_nm + b.x_nm, a.y_nm + b.y_nm), (2 * pad.center.x_nm, -2 * pad.center.y_nm));
                            let span = (x - y).abs() + 1;
                            assert_eq!(((a.x_nm - b.x_nm).abs(), (a.y_nm - b.y_nm).abs()), if x > y { (span, 0) } else { (0, span) });
                        }
                        continue;
                    }
                    assert!(read.strokes.is_empty());
                    if shape == PadShape::Rectangle && (width % 2 != 0 || height % 2 != 0) {
                        assert!(read.flashes.is_empty() && read.apertures.is_empty());
                        assert_eq!(read.regions.len(), pads.len());
                        for (region, pad) in read.regions.iter().zip(&pads) {
                            let corners = pad.copper.unwrap();
                            let expected: Vec<_> =
                                corners.iter().chain(std::iter::once(&corners[0])).map(|p| Point { x_nm: p.x_nm, y_nm: -p.y_nm }).collect();
                            assert_eq!(*region, expected, "half-grid rectangle keeps its exact contour");
                        }
                        continue;
                    }
                    assert!(read.regions.is_empty());
                    assert_eq!(read.apertures.len(), 1, "equal pads share an aperture");
                    assert_eq!(read.flashes.len(), pads.len());
                    for ((center, aperture), pad) in read.flashes.iter().zip(&pads) {
                        assert_eq!(*center, Point { x_nm: pad.center.x_nm, y_nm: -pad.center.y_nm });
                        assert_eq!(aperture.kind, kind);
                        let expected_size = if (pad_rotation / 90 + rotation / 90).is_multiple_of(2) { (width, height) } else { (height, width) };
                        assert_eq!((aperture.width, aperture.height), expected_size, "final axis order, including two quarter turns and mirror");
                        let corners = pad.copper.unwrap();
                        let (min_x, max_x) = (corners.iter().map(|p| p.x_nm).min().unwrap(), corners.iter().map(|p| p.x_nm).max().unwrap());
                        let (min_y, max_y) = (corners.iter().map(|p| p.y_nm).min().unwrap(), corners.iter().map(|p| p.y_nm).max().unwrap());
                        assert_eq!((aperture.width, aperture.height), (max_x - min_x, max_y - min_y));
                        if shape == PadShape::Rectangle {
                            assert_eq!(2 * center.x_nm - aperture.width, 2 * min_x);
                            assert_eq!(2 * center.x_nm + aperture.width, 2 * max_x);
                            assert_eq!(2 * center.y_nm - aperture.height, -2 * max_y);
                            assert_eq!(2 * center.y_nm + aperture.height, -2 * min_y);
                        }
                    }
                }
                for output in surface_files(!bottom) {
                    let opposite = read_gerber(&outputs, output);
                    assert!(opposite.flashes.is_empty() && opposite.regions.is_empty() && opposite.strokes.is_empty(), "{}", output.0);
                }
            }
        }
    }
}

/// Shape and X/Y order are part of aperture identity, even when numerical sizes coincide.
/// A round stroke after a rectangular/obround flash must reselect a circular aperture.
#[test]
fn aperture_identity_and_selection_retain_shape_and_orientation() {
    let mut gerber = Gerber::new(Frame::BOARD_ZERO);
    let point = Point { x_nm: 0, y_nm: 0 };
    for aperture in [
        Aperture::Circle(400_000),
        Aperture::Rectangle(400_000, 400_000),
        Aperture::Obround(400_000, 400_000),
        Aperture::Rectangle(800_000, 400_000),
        Aperture::Rectangle(400_000, 800_000),
        Aperture::Obround(800_000, 400_000),
        Aperture::Obround(400_000, 800_000),
    ] {
        gerber.aperture(aperture);
        gerber.body.push_str("X0Y0D03*\n");
    }
    gerber.stroke(&[point, Point { x_nm: 1_000_000, y_nm: 0 }], 400_000);
    let text = gerber.finish("Copper,L1,Top", "Positive");
    let read = try_read_gerber(&text, "Copper,L1,Top", "Positive").unwrap();
    assert_eq!(read.apertures.len(), 7);
    assert_eq!(read.flashes.len(), 7);
    assert_eq!(
        read.flashes,
        [
            ('C', 400_000, 400_000),
            ('R', 400_000, 400_000),
            ('O', 400_000, 400_000),
            ('R', 800_000, 400_000),
            ('R', 400_000, 800_000),
            ('O', 800_000, 400_000),
            ('O', 400_000, 800_000),
        ]
        .map(|(kind, width, height)| (point, ReadAperture { kind, width, height }))
    );
    assert_eq!(read.strokes, [(point, Point { x_nm: 1_000_000, y_nm: 0 }, ReadAperture { kind: 'C', width: 400_000, height: 400_000 })]);
    assert_eq!(read.strokes[0].2.kind, 'C');
    assert_eq!(read.strokes[0].2.width, 400_000);
    assert_eq!(text.matches("D10*\n").count(), 2, "circle is selected again after the last O flash");
}

/// Non-cardinal rectangles retain their exact polygon and obrounds their existing round stroke.
#[test]
fn rotated_rectangular_and_obround_pad_fallbacks_retain_geometry() {
    for shape in [PadShape::Rectangle, PadShape::Oval] {
        for (pad_rotation, rotation) in [(45, 0), (0, 37), (13, 90)] {
            for bottom in [false, true] {
                let s = aperture_fixture(shape, 800_000, 400_000, pad_rotation, rotation, bottom);
                let outputs = view(&s).fabrication_outputs().unwrap();
                assert_eq!(outputs, view(&s).fabrication_outputs().unwrap());
                assert_eq!(zip(&outputs).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
                let read = read_gerber(&outputs, surface_files(bottom)[2]);
                let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
                assert!(read.flashes.is_empty());
                if shape == PadShape::Oval {
                    assert!(read.regions.is_empty());
                    assert_eq!(read.strokes.len(), pads.len());
                    for ((a, b, aperture), pad) in read.strokes.iter().zip(pads) {
                        assert_eq!(aperture.kind, 'C');
                        assert!((aperture.width - 400_000).abs() <= 1);
                        assert_eq!(a.x_nm + b.x_nm, 2 * pad.center.x_nm);
                        assert_eq!(a.y_nm + b.y_nm, -2 * pad.center.y_nm);
                        let corners = pad.copper.unwrap();
                        let (dx, dy) = ((corners[1].x_nm - corners[0].x_nm) as f64, -(corners[1].y_nm - corners[0].y_nm) as f64);
                        let length = dx.hypot(dy);
                        // Existing endpoint and projected-corner rounding each contributes <= 1 nm.
                        assert!(((b.x_nm - a.x_nm) as f64 - dx * 400_000. / length).abs() <= 2.);
                        assert!(((b.y_nm - a.y_nm) as f64 - dy * 400_000. / length).abs() <= 2.);
                    }
                } else {
                    assert!(read.strokes.is_empty());
                    assert_eq!(read.regions.len(), pads.len());
                    for (region, pad) in read.regions.iter().zip(pads) {
                        assert_eq!(region.first(), region.last());
                        let corners = pad.copper.unwrap();
                        let expected: Vec<_> =
                            corners.iter().chain(std::iter::once(&corners[0])).map(|p| Point { x_nm: p.x_nm, y_nm: -p.y_nm }).collect();
                        assert_eq!(*region, expected);
                    }
                }
            }
        }
    }
}

/// Read exact corner diameters and verify the whole union: four circles centered on the
/// inner contour, one diameter-wide strip along each edge, and a filled closed inner outline.
/// Ucamco Gerber 2026.05 §§4.5.1.3, 4.5.1.4 and 4.5.1.6 specify the parsed primitives.
#[test]
fn rounded_pad_macros_read_back_radius_and_orientation() {
    for (local, placed) in [(0, 0), (0, 90), (0, 37), (37, 0), (13, 37)] {
        for bottom in [false, true] {
            let mut s = aperture_fixture(PadShape::RoundedRectangle, 800_000, 400_000, local, placed, bottom);
            s.apply(crate::SET_PLACEMENT_ORIGIN, &json!({"board":s.project().unwrap().board_target().unwrap().board,"origin":{"kind":"point","x_nm":1_000_003,"y_nm":-2_000_007},"expected_revision":s.revision()})).unwrap();
            let fabrication = view(&s).fabrication().unwrap();
            if (local, placed) == (0, 37) {
                let revision = s.revision();
                let returned = s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
                assert_eq!(returned["fabrication"], serde_json::to_value(&fabrication).unwrap());
                assert_eq!(s.revision(), revision);
                assert_eq!(zip(&fabrication.files).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
            }
            let pads = view(&s).footprints().next().unwrap().pads;
            let expected_angle = if bottom { f64::from(placed) - f64::from(local) } else { f64::from(placed) + f64::from(local) };
            let (sin, cos) = expected_angle.to_radians().sin_cos();
            for output in surface_files(bottom) {
                let read = read_gerber(&fabrication.files, output);
                assert!(read.regions.is_empty() && read.strokes.is_empty() && read.flashes.is_empty(), "{}", output.0);
                assert_eq!(read.rounded_flashes.len(), pads.len());
                if placed.is_multiple_of(90) {
                    assert_eq!(read.rounded.len(), 1, "identical projected geometry reuses its aperture");
                }
                for (&(at, code), pad) in read.rounded_flashes.iter().zip(&pads) {
                    assert_eq!(at, Point { x_nm: pad.center.x_nm - 1_000_003, y_nm: -(pad.center.y_nm + 2_000_007) });
                    assert_eq!(pad.corner_radius_nm, 100_000);
                    let aperture = &read.rounded[&code];
                    assert_eq!((aperture.circles.len(), aperture.strips.len(), aperture.outline.len()), (4, 4, 5));
                    assert!(aperture.circles.iter().all(|(d, _)| *d == 2 * pad.corner_radius_nm), "exact authored radius in nanometers");
                    let vertices = &aperture.outline[..4];
                    for (&vertex, (_, circle)) in vertices.iter().zip(&aperture.circles) {
                        assert_eq!(vertex, *circle);
                    }
                    for ((&start, &end), strip) in aperture.outline.iter().zip(&aperture.outline[1..]).zip(&aperture.strips) {
                        assert_eq!(*strip, (2 * pad.corner_radius_nm, start, end));
                    }
                    // Check the model-to-file frame independently of the emitter. The model
                    // rounds each rotation and the final inner vertices, not the Gerber radius.
                    for (x, y) in [(-300_000., -100_000.), (-300_000., 100_000.), (300_000., -100_000.), (300_000., 100_000.)] {
                        let (x, y) = (x * cos - y * sin, x * sin + y * cos);
                        assert!(vertices.iter().any(|p| (p.x_nm as f64 - x).abs() <= 2. && (p.y_nm as f64 - y).abs() <= 2.));
                    }
                }
            }
            for output in surface_files(!bottom) {
                assert!(read_gerber(&fabrication.files, output).rounded_flashes.is_empty());
            }
        }
    }
}

/// The maximum legal radius fills a point core as one disk, or a line core as a capsule.
#[test]
fn rounded_pad_macros_handle_point_and_line_cores() {
    for width in [2, 4] {
        let s = aperture_fixture(PadShape::RoundedRectangle, width, 2, 0, 0, false);
        let outputs = view(&s).fabrication_outputs().unwrap();
        let read = read_gerber(&outputs, TOP_COPPER);
        assert!(read.regions.is_empty());
        assert_eq!(read.rounded.len(), 1);
        assert_eq!(read.rounded_flashes.len(), 2);
        let aperture = &read.rounded[&read.rounded_flashes[0].1];
        assert!(aperture.outline.is_empty());
        if width == 2 {
            assert_eq!(aperture.circles, [(2, Point { x_nm: 0, y_nm: 0 })]);
            assert!(aperture.strips.is_empty());
        } else {
            let (start, end) = (Point { x_nm: -1, y_nm: 0 }, Point { x_nm: 1, y_nm: 0 });
            assert_eq!(aperture.circles, [(2, start), (2, end)]);
            assert_eq!(aperture.strips, [(2, start, end)]);
        }
    }
}

/// Assert exact refusal reasons for wrong X2 metadata, unsupported modes, noncanonical
/// aperture spellings and misordered or structurally unclosed contours.
#[test]
fn gerber_subset_reader_rejects_header_syntax_and_order_mutations() {
    let s = aperture_fixture_with_pads(
        &[(PadShape::Circle, 400_000, 400_000, 0), (PadShape::Rectangle, 800_000, 400_000, 0), (PadShape::Oval, 800_000, 400_000, 0)],
        0,
        false,
    );
    let outputs = view(&s).fabrication_outputs().unwrap();
    let (suffix, function, polarity) = TOP_COPPER;
    let text = file(&outputs, suffix);
    assert_eq!(read_gerber(&outputs, TOP_COPPER).flashes.len(), 3);
    let first_definition = text.lines().find(|line| line.starts_with("%ADD")).unwrap();
    for (mutant, reason) in [
        (text.replace("%TF.SameCoordinates,Original*%\n", ""), "invalid header"),
        (text.replace("%TF.FileFunction,Copper,L1,Top*%", "%TF.FileFunction,Unknown*%"), "invalid file function"),
        (text.replace("%TF.FileFunction,Copper,L1,Top*%", "%TF.FileFunction,Legend,Top*%"), "wrong file function"),
        (text.replace("Copper,L1,Top", "Copper,L1,Bot"), "wrong file side"),
        (text.replace("Copper,L1,Top", "Copper,L2,Top"), "wrong copper ordinal"),
        (text.replace("%TF.FilePolarity,Positive*%", "%TF.FilePolarity,Negative*%"), "invalid file polarity"),
        (text.replace("%MOMM*%", "%MOIN*%"), "unsupported coordinate mode"),
        (text.replace("%FSLAX46Y46*%", "%FSLAX45Y45*%"), "unsupported coordinate mode"),
        (text.replace("G01*", "G02*"), "unsupported coordinate mode"),
        (text.replace("%LPD*%", "%LPC*%"), "unsupported coordinate mode"),
        (text.replace("G01*\n", "G01*\n%LPC*%\n"), "unsupported record"),
        (text.replace("G01*\n", "G01*\n%LR90*%\n"), "unsupported record"),
        (text.replace("G01*\n", "G01*\n%LS2*%\n"), "unsupported record"),
        (text.replace("G01*\n", "G01*\n%LMX*%\n"), "unsupported record"),
        (text.replace("R,0.800000", "R,-0.800000"), "invalid aperture dimension"),
        (text.replace("R,0.800000", "R,-0.000001"), "invalid aperture dimension"),
        (text.replace("R,0.800000", "R,0.000000"), "invalid aperture dimension"),
        (text.replace("R,0.800000", "R,0.80000"), "invalid aperture dimension"),
        (text.replace("R,0.800000", "R,0.800000X0.100000"), "invalid aperture arity"),
        (text.replace("\nD10*\n", "\nD99*\n"), "undefined aperture"),
        (text.replace("\nD10*\n", "\nD+10*\n"), "invalid selection"),
        (text.replace("\nD10*\n", "\nD010*\n"), "invalid selection"),
        (text.replace("%ADD10", "%ADD+10").replace("\nD10*\n", "\nD+10*\n"), "invalid aperture code"),
        (text.replace("%ADD10", "%ADD010"), "invalid aperture code"),
        (text.replace(first_definition, &format!("{first_definition}\n{first_definition}")), "duplicate aperture"),
        (text.replace("M02*", &format!("{first_definition}\nM02*")), "late aperture definition"),
        (text.replace("M02*\n", ""), "missing end record"),
        (format!("{text}G01*\n"), "records after end"),
    ] {
        assert_ne!(mutant, text, "mutation must affect fixture");
        assert_eq!(try_read_gerber(&mutant, function, polarity).unwrap_err(), reason);
    }
    let bottom = aperture_fixture(PadShape::Circle, 400_000, 400_000, 0, 0, true);
    let outputs = view(&bottom).fabrication_outputs().unwrap();
    let (paste_suffix, paste_function, paste_polarity) = BOTTOM_PASTE;
    let paste = file(&outputs, paste_suffix);
    assert_eq!(read_gerber(&outputs, BOTTOM_PASTE).flashes.len(), 2);
    for (replacement, reason) in [("Paste,Top", "wrong file side"), ("Legend,Top", "wrong file function")] {
        let mutant = paste.replace(paste_function, replacement);
        assert_ne!(mutant, paste, "mutation must affect fixture");
        assert_eq!(try_read_gerber(&mutant, paste_function, paste_polarity).unwrap_err(), reason);
    }
    assert_eq!(read_mm("0.000001"), Ok(1));
    assert_eq!(read_mm("100.000000"), Ok(100_000_000));
    assert_eq!(read_mm("9223372036854.775807"), Ok(i64::MAX));
    assert_eq!(read_mm("9223372036854.775808"), Err("invalid aperture dimension"));
    for coordinate in ["0", "1", "-1", "9999999999", "-9999999999"] {
        assert_eq!(read_coordinate(coordinate), Ok(coordinate.parse().unwrap()));
    }
    for invalid in ["10000000000", "-10000000000", "9223372036854775808", "+1", "01", "-0", "1.0", ""] {
        assert_eq!(read_coordinate(invalid), Err("invalid coordinate"));
    }
    for invalid in ["-0.000001", "-0.000000", "+0.000001", "0.000000", "0.0000010", "0.00000x", "9223372036854775808.000000"] {
        assert_eq!(read_mm(invalid), Err("invalid aperture dimension"));
    }
    let s = aperture_fixture(PadShape::Rectangle, 800_001, 400_003, 0, 0, false);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let text = file(&outputs, suffix);
    assert_eq!(read_gerber(&outputs, TOP_COPPER).regions.len(), 2);
    let start = text.lines().find(|line| line.ends_with("D02*")).unwrap().strip_suffix("D02*").unwrap();
    for (mutant, reason) in [
        (text.replacen(&format!("{start}D01*"), "X0Y0D01*", 1), "region contour not closed"),
        (format!("{}M02*\n", text.split_once("G37*\n").unwrap().0), "unclosed region"),
        (text.replace("D01*", "D02*"), "invalid region operation"),
        (text.replacen("D02*", "D01*", 1), "invalid region operation"),
        (text.replacen("D01*", "D03*", 1), "invalid region operation"),
        (text.replace("G37*\n", ""), "nested region"),
        (text.replace("G36*", "G36*\nG36*"), "nested region"),
    ] {
        assert_ne!(mutant, text, "mutation must affect fixture");
        assert_eq!(try_read_gerber(&mutant, function, polarity).unwrap_err(), reason);
    }
}

/// Reject wrong macro exposure, nonzero rotation, or a flash using an undefined template.
#[test]
fn gerber_subset_reader_rejects_invalid_rounded_macro_parameters() {
    let s = aperture_fixture(PadShape::RoundedRectangle, 800_000, 400_000, 0, 0, false);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let (suffix, function, polarity) = TOP_COPPER;
    let text = file(&outputs, suffix);
    assert_eq!(read_gerber(&outputs, TOP_COPPER).rounded_flashes.len(), 2);
    for (mutant, reason) in [
        (text.replacen("1,1,0.200000,", "1,0,0.200000,", 1), "unsupported macro primitive"),
        (text.replacen(",0*\n", ",90*\n", 1), "unsupported macro primitive"),
        (text.replace("%ADD10RoundRect10*%", "%ADD10RoundRect99*%"), "undefined macro"),
    ] {
        assert_ne!(mutant, text);
        assert_eq!(try_read_gerber(&mutant, function, polarity).unwrap_err(), reason);
    }
}

/// Snapped boxes of each shape are equal within one placed definition, but each pad's
/// local rotation independently decides whether it can flash R or O.
#[test]
fn aperture_eligibility_is_per_pad_within_one_footprint() {
    let s = aperture_fixture_with_pads(
        &[(PadShape::Rectangle, 2, 2, 0), (PadShape::Rectangle, 2, 2, 1), (PadShape::Oval, 2, 4, 0), (PadShape::Oval, 2, 4, 1)],
        0,
        false,
    );
    let pads = view(&s).footprints().next().unwrap().pads;
    assert_eq!(
        pads.iter().map(|pad| axis_aligned_size(&pad.copper.unwrap())).collect::<Vec<_>>(),
        [Some((2, 2)), Some((2, 2)), Some((2, 4)), Some((2, 4))]
    );
    let [rect, rotated_rect, oval, rotated_oval] = [30_000_001, 32_000_001, 34_000_001, 36_000_001].map(|x_nm| Point { x_nm, y_nm: 25_000_003 });
    let outputs = view(&s).fabrication_outputs().unwrap();
    let read = read_gerber(&outputs, TOP_COPPER);
    assert_eq!(read.flashes, [(rect, ReadAperture { kind: 'R', width: 2, height: 2 }), (oval, ReadAperture { kind: 'O', width: 2, height: 4 })]);
    assert_eq!(read.regions.len(), 1);
    assert_eq!(
        read.regions[0],
        [(-1, 1), (1, 1), (1, -1), (-1, -1), (-1, 1)].map(|(dx, dy)| Point { x_nm: rotated_rect.x_nm + dx, y_nm: rotated_rect.y_nm + dy })
    );
    assert_eq!(
        read.strokes,
        [(
            Point { y_nm: rotated_oval.y_nm + 1, ..rotated_oval },
            Point { y_nm: rotated_oval.y_nm - 1, ..rotated_oval },
            ReadAperture { kind: 'C', width: 2, height: 2 },
        )]
    );
}

/// A legal 1 nm radius clamps to about 0.707 nm after the corners of a rotated 2×2 nm pad
/// snap to a diamond. Its point core flashes as the nearest whole-nanometre disk.
#[test]
fn tiny_rounded_pad_flashes_its_clamped_point_core() {
    let s = aperture_fixture(PadShape::RoundedRectangle, 2, 2, 45, 0, false);
    let pads = view(&s).footprints().next().unwrap().pads;
    assert_eq!(pads.len(), 2);
    assert!(pads.iter().all(|pad| pad.corner_radius_nm == 1));
    let contour = Contour::pad(pads[0].center, pads[0].copper.unwrap(), pads[0].shape, pads[0].corner_radius_nm);
    assert!((contour.radius - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let read = read_gerber(&outputs, TOP_COPPER);
    assert!(read.apertures.is_empty() && read.flashes.is_empty() && read.strokes.is_empty() && read.regions.is_empty());
    assert_eq!(read.rounded.len(), 1);
    let at: Vec<_> = read.rounded_flashes.iter().map(|&(at, _)| at).collect();
    assert_eq!(at, [30_000_001, 32_000_001].map(|x_nm| Point { x_nm, y_nm: 25_000_003 }));
    let aperture = &read.rounded[&read.rounded_flashes[0].1];
    assert_eq!(aperture.circles, [(1, Point { x_nm: 0, y_nm: 0 })]);
    assert!(aperture.strips.is_empty() && aperture.outline.is_empty());
}

/// A maximum radius on a rotated 0.6 × 0.4 mm pad clamps below 200,000 nm when its corners
/// snap. It still flashes circular corners, within 1 nm of the authored and model radii.
/// Includes one- and two-stage projections; the latter can cross the authored tolerance
/// before diameter rounding but meet it after rounding.
#[test]
fn rotated_maximum_radius_pads_keep_circular_corners() {
    for (local, placed, bottom, offset, first_diameter) in [
        (45, 0, false, (0, 0), 399_999),
        (0, 45, false, (0, 0), 399_999),
        (45, 0, true, (0, 0), 399_999),
        (13, 0, false, (0, 0), 400_000),
        (30, 0, false, (0, 0), 400_000),
        (18, 32, false, (3, 11), 399_998),
    ] {
        let pads = [(PadShape::RoundedRectangle, 600_000, 400_000, local, 200_000); 2];
        let offset = Point { x_nm: offset.0, y_nm: offset.1 };
        let s = aperture_fixture_with_radii(&pads, placed, bottom, offset, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, false);
        let fabrication = view(&s).fabrication().unwrap();
        let pads = view(&s).footprints().next().unwrap().pads;
        let contours: Vec<_> = pads.iter().map(|pad| Contour::pad(pad.center, pad.copper.unwrap(), pad.shape, pad.corner_radius_nm)).collect();
        // The second pad's different local center can cause different placement rounding.
        assert!(contours[0].radius < 200_000., "the first pad must clamp the radius");
        if (local, placed) == (18, 32) {
            assert!(contours[0].radius < 199_999., "the model radius must miss the authored tolerance before diameter rounding");
        }
        let expected_angle = if bottom { f64::from(placed) - f64::from(local) } else { f64::from(placed) + f64::from(local) };
        let (sin, cos) = expected_angle.to_radians().sin_cos();
        for output in surface_files(bottom) {
            let read = read_gerber(&fabrication.files, output);
            assert!(read.regions.is_empty() && read.strokes.is_empty() && read.flashes.is_empty(), "{} at {local}/{placed}", output.0);
            assert_eq!(read.rounded_flashes.len(), pads.len());
            assert_eq!(read.rounded[&read.rounded_flashes[0].1].circles[0].0, first_diameter);
            for ((&(at, code), pad), contour) in read.rounded_flashes.iter().zip(&pads).zip(&contours) {
                assert_eq!(at, Point { x_nm: pad.center.x_nm, y_nm: -pad.center.y_nm });
                let aperture = &read.rounded[&code];
                assert!(aperture.outline.is_empty(), "the short sides meet, leaving a line core");
                assert_eq!(aperture.circles.len(), 2);
                let diameter = aperture.circles[0].0;
                assert!(diameter.abs_diff(2 * pad.corner_radius_nm) <= 2, "authored radius within 1 nm");
                assert!((diameter as f64 / 2. - contour.radius).abs() <= 0.25, "diameter rounded to the nearest 1 nm");
                let [(_, start), (_, end)] = [aperture.circles[0], aperture.circles[1]];
                assert!(aperture.circles.iter().all(|&(d, _)| d == diameter));
                assert_eq!(aperture.strips, [(diameter, start, end)]);
                // The core runs along the long axis, 100,000 nm each side of the center.
                let (x, y) = (100_000. * cos, 100_000. * sin);
                for (point, sign) in [(start, -1.), (end, 1.)] {
                    let near = |s: f64| (point.x_nm as f64 - s * x).abs() <= 2. && (point.y_nm as f64 - s * y).abs() <= 2.;
                    assert!(near(sign) || near(-sign), "{point:?} at {local}/{placed}");
                }
            }
        }
    }
}

/// Two rotations can snap a tiny inner box onto three edges without clamping its radius.
/// Keep that pad's original closed region rather than writing a malformed quad primitive.
#[test]
fn tiny_rounded_pad_with_non_strict_core_retains_region() {
    let s = aperture_fixture_at(
        &[(PadShape::RoundedRectangle, 3, 3, 165); 2],
        45,
        false,
        Point { x_nm: -1, y_nm: 1 },
        Point { x_nm: 30_000_001, y_nm: -25_000_003 },
        false,
    );
    let pads = view(&s).footprints().next().unwrap().pads;
    let first = &pads[0];
    let contour = Contour::pad(first.center, first.copper.unwrap(), first.shape, first.corner_radius_nm);
    assert_eq!(contour.radius, 1.);
    let at = Point { x_nm: first.center.x_nm, y_nm: -first.center.y_nm };
    let read = read_gerber(&view(&s).fabrication_outputs().unwrap(), TOP_COPPER);
    assert!(!read.rounded_flashes.iter().any(|(center, _)| *center == at));
    assert_eq!(read.regions[0].len(), 29, "the positive fallback radius still produces four chorded corners");
    assert_eq!(read.regions[0].first(), read.regions[0].last());
}

/// Tiny non-cardinal and twice-rounded collapsed lands stay on the existing fallback, even
/// when their snapped corners look cardinal. Net-cardinal mixed stages are conservative fallbacks.
#[test]
fn tiny_and_composed_rotations_do_not_flash_rectangular_or_obround_apertures() {
    for (shape, width, height, local, rotation, bottom, offset) in [
        (PadShape::Oval, 1, 2, 31, 40, true, Point { x_nm: 0, y_nm: 2 }),
        (PadShape::Rectangle, 1, 2, 31, 42, true, Point { x_nm: 0, y_nm: 2 }),
        (PadShape::Rectangle, 2, 2, 0, 1, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Rectangle, 2, 2, 1, 0, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 2, 4, 0, 1, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 2, 4, 1, 0, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 2, 1, 1, 0, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 1, 2, 0, 1, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 400_000, 400_000, 45, 0, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Rectangle, 800_000, 400_000, 45, 45, false, Point { x_nm: 0, y_nm: 0 }),
        (PadShape::Oval, 800_000, 400_000, 45, 45, true, Point { x_nm: 0, y_nm: 0 }),
    ] {
        let s =
            aperture_fixture_at(&[(shape, width, height, local); 2], rotation, bottom, offset, Point { x_nm: 30_000_001, y_nm: -25_000_003 }, false);
        let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
        if width == 2 && (height == 2 || height == 4) {
            assert_eq!(axis_aligned_size(&pads[0].copper.unwrap()), Some((width, height)), "snapped geometry alone would permit a flash");
        }
        if local == 31 {
            let corners = pads[0].copper.unwrap();
            assert!(corners.iter().all(|p| p.x_nm == corners[0].x_nm), "counterexample collapses onto one axis");
            assert_eq!(axis_aligned_size(&corners), None);
        }
        let outputs = view(&s).fabrication_outputs().unwrap();
        for output in surface_files(bottom) {
            let read = read_gerber(&outputs, output);
            assert!(read.apertures.values().all(|a| a.kind == 'C' && a.width > 0 && a.height > 0));
            if shape == PadShape::Rectangle {
                assert!(read.flashes.is_empty() && read.strokes.is_empty());
                assert_eq!(read.regions.len(), pads.len());
                for (region, pad) in read.regions.iter().zip(&pads) {
                    let c = pad.copper.unwrap();
                    assert_eq!(*region, c.iter().chain(std::iter::once(&c[0])).map(|p| Point { x_nm: p.x_nm, y_nm: -p.y_nm }).collect::<Vec<_>>());
                }
                if local == 31 {
                    let path = &read.regions[0];
                    assert!(path.iter().all(|p| p.x_nm == path[0].x_nm));
                    // The closing edge retraces the preceding collinear steps; structural
                    // closure does not imply a filled land.
                    let low = path.iter().map(|p| p.y_nm).min().unwrap();
                    let high = path.iter().map(|p| p.y_nm).max().unwrap();
                    assert_eq!(high - low, 3);
                    assert_eq!(path.first(), path.last());
                }
            } else {
                assert!(read.regions.is_empty());
                assert_eq!(read.flashes.len() + read.strokes.len(), pads.len());
                if local == 31 {
                    let first = Point { x_nm: pads[0].center.x_nm, y_nm: -pads[0].center.y_nm };
                    assert!(read.flashes.iter().any(|(at, a)| *at == first && a.kind == 'C' && a.width == 1));
                }
                if width == height {
                    assert!(read.strokes.is_empty());
                    for ((at, a), pad) in read.flashes.iter().zip(&pads) {
                        assert_eq!(*at, Point { x_nm: pad.center.x_nm, y_nm: -pad.center.y_nm });
                        assert_eq!(a.kind, 'C');
                        assert!((a.width - width).abs() <= 1, "rounded square fallback diameter");
                    }
                }
            }
        }
    }
}

/// Standard O flashes match the engine land capsule only when both dimension parities agree.
/// Mixed parity retains rounded-endpoint strokes; those approximations need not equal the land.
#[test]
fn obround_capsules_match_land_geometry_across_parity_sides_and_output_origins() {
    let mut observed_fallback_difference = false;
    for (width, height) in [(2, 1), (1, 2), (3, 2), (2, 3), (4, 2), (3, 1), (2, 2), (1, 1)] {
        for (center, origin) in [(0, 0), (3, -5), (-3, 5)] {
            for rotation in [0, 90] {
                for bottom in [false, true] {
                    let mut s = aperture_fixture_at(
                        &[(PadShape::Oval, width, height, 0); 2],
                        rotation,
                        bottom,
                        Point { x_nm: 0, y_nm: 0 },
                        Point { x_nm: center, y_nm: -center },
                        false,
                    );
                    s.apply(crate::SET_PLACEMENT_ORIGIN, &json!({"board":s.project().unwrap().board_target().unwrap().board,"origin":{"kind":"point","x_nm":origin,"y_nm":-origin},"expected_revision":s.revision()})).unwrap();
                    let fabrication = view(&s).fabrication().unwrap();
                    let frame = fabrication.origin.frame();
                    let lands = crate::board::track::lands(view(&s));
                    let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
                    for output in surface_files(bottom) {
                        let read = read_gerber(&fabrication.files, output);
                        assert!(read.regions.is_empty());
                        if width % 2 == height % 2 {
                            assert!(read.strokes.is_empty());
                            assert_eq!(read.flashes.len(), lands.len());
                            for ((at, a), land) in read.flashes.iter().zip(&lands) {
                                assert_eq!(a.kind, 'O');
                                let contour = land.contour();
                                assert_eq!(2. * contour.radius, a.width.min(a.height) as f64);
                                let expected: BTreeSet<_> = contour
                                    .core
                                    .map(|p| {
                                        let p = frame.to_output(p);
                                        (2 * p.x_nm, 2 * p.y_nm)
                                    })
                                    .into_iter()
                                    .collect();
                                let span = (a.width - a.height).abs();
                                let actual: BTreeSet<_> = [-1, 1]
                                    .map(|sign| {
                                        (
                                            2 * at.x_nm + if a.width >= a.height { sign * span } else { 0 },
                                            2 * at.y_nm + if a.height > a.width { sign * span } else { 0 },
                                        )
                                    })
                                    .into_iter()
                                    .collect();
                                assert_eq!(actual, expected, "analytic O core and land core");
                            }
                        } else {
                            assert!(read.flashes.is_empty());
                            assert_eq!(read.strokes.len(), lands.len());
                            for ((a, b, aperture), land) in read.strokes.iter().zip(&lands) {
                                assert_eq!(*aperture, ReadAperture { kind: 'C', width: width.min(height), height: width.min(height) });
                                let contour = land.contour();
                                let expected: BTreeSet<_> = contour
                                    .core
                                    .map(|p| {
                                        let p = frame.to_output(p);
                                        (p.x_nm, p.y_nm)
                                    })
                                    .into_iter()
                                    .collect();
                                let actual = BTreeSet::from([(a.x_nm, a.y_nm), (b.x_nm, b.y_nm)]);
                                let center = frame.to_output(pads.iter().find(|pad| pad.number == land.key.1).unwrap().center);
                                let half = ((width - height).abs() + 1) / 2;
                                let along_x = (rotation == 0) == (width > height);
                                let legacy: BTreeSet<_> = [-1, 1]
                                    .map(|sign| {
                                        (center.x_nm + if along_x { sign * half } else { 0 }, center.y_nm + if along_x { 0 } else { sign * half })
                                    })
                                    .into_iter()
                                    .collect();
                                assert_eq!(actual, legacy, "existing rounded-endpoint fallback position and axis");
                                observed_fallback_difference |= actual != expected;
                                assert_eq!((a.x_nm - b.x_nm).abs() + (a.y_nm - b.y_nm).abs(), (width - height).abs() + 1);
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(observed_fallback_difference, "legacy endpoint rounding is not general exact land equivalence");
}

/// All-copper plated pads reach every copper and both outer mask files, but not paste.
#[test]
fn through_hole_pad_flashes_are_excluded_from_paste() {
    for (shape, kind) in [(PadShape::Circle, 'C'), (PadShape::Rectangle, 'R'), (PadShape::Oval, 'O')] {
        let width = if shape == PadShape::Circle { 400_000 } else { 800_000 };
        let s = aperture_fixture_at(
            &[(shape, width, 400_000, 0); 2],
            90,
            true,
            Point { x_nm: 0, y_nm: 0 },
            Point { x_nm: 3_000_000, y_nm: -3_000_000 },
            true,
        );
        let outputs = view(&s).fabrication_outputs().unwrap();
        // The back-side quarter turn swaps the aperture axes and mirrors the local X offsets.
        let aperture = ReadAperture { kind, width: 400_000, height: width };
        let expected = [3_000_000, 1_000_000].map(|y_nm| (Point { x_nm: 3_000_000, y_nm }, aperture));
        for output in [
            TOP_COPPER,
            ("-Inner_1.gbr", "Copper,L2,Inr", "Positive"),
            ("-Inner_2.gbr", "Copper,L3,Inr", "Positive"),
            ("-Bottom.gbr", "Copper,L4,Bot", "Positive"),
            TOP[1],
            BOTTOM[1],
        ] {
            let read = read_gerber(&outputs, output);
            assert_eq!(read.flashes, expected, "{}", output.0);
            assert!(read.regions.is_empty() && read.strokes.is_empty());
        }
        for output in [TOP[2], BOTTOM[2]] {
            let read = read_gerber(&outputs, output);
            assert!(read.flashes.is_empty() && read.regions.is_empty() && read.strokes.is_empty());
        }
    }
}

/// Shared export dispatch is read-only, deterministic, origin-aware and survives native reopen.
/// Exports between undo and redo cannot consume either history entry or alter saved bytes.
#[test]
fn aperture_exports_preserve_revision_storage_and_history() {
    use crate::persistence::{decode_documents, encode_documents};
    let mut s = aperture_fixture(PadShape::Oval, 800_001, 400_003, 90, 0, true);
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    s.apply(crate::SET_PLACEMENT_ORIGIN, &json!({"board":s.project().unwrap().board_target().unwrap().board,"origin":{"kind":"point","x_nm":1_000_003,"y_nm":-2_000_007},"expected_revision":s.revision()}))
        .unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let revision = s.revision();
    let result = s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    let direct = view(&s).fabrication().unwrap();
    let records: Vec<_> = direct
        .files
        .iter()
        .map(|output| {
            let bytes = output.contents.as_bytes();
            json!({"name": output.name, "size_bytes": bytes.len(), "sha256": format!("{:x}", Sha256::digest(bytes))})
        })
        .collect();
    assert_eq!(
        result["fabrication"],
        json!({"files": direct.files, "origin": direct.origin, "manifest": {"generator_version": env!("CARGO_PKG_VERSION"), "files": records}})
    );
    assert_eq!(s.revision(), revision);
    assert_eq!(s.apply(EXPORT_FABRICATION, &json!({})).unwrap(), revision);
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    let read = read_gerber(&direct.files, BOTTOM_PASTE);
    let pad = view(&s).footprints().next().unwrap().pads.remove(0);
    assert_eq!(read.flashes[0].0, Point { x_nm: pad.center.x_nm - 1_000_003, y_nm: -(pad.center.y_nm + 2_000_007) });
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
    assert_eq!(reopened.board_view(reopened.board_target().unwrap()).unwrap().fabrication().unwrap(), direct);
    assert_eq!(zip(&direct.files).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

fn file<'a>(outputs: &'a [Output], suffix: &str) -> &'a str {
    &outputs
        .iter()
        .find(|output| output.name.ends_with(suffix))
        .unwrap_or_else(|| panic!("no {suffix} in {:?}", outputs.iter().map(|o| &o.name).collect::<Vec<_>>()))
        .contents
}

/// Outputs need a board setup and a closed outline, and say which is missing.
#[test]
fn outputs_require_setup_and_a_closed_outline() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let project = s.project().unwrap();
    let error = project.board_view(project.board_target().unwrap()).unwrap().fabrication_outputs().unwrap_err();
    assert!(error.contains("board setup"), "{error}");
    let s = routed_fixture();
    let error = view(&s).fabrication_outputs().unwrap_err();
    assert!(error.contains("closed Board Outline"), "{error}");
}

/// A local reference move affects only that footprint's silk and round-trips through native storage.
#[test]
fn moved_reference_changes_only_its_gerber_silkscreen() {
    use crate::persistence::{decode_documents, encode_documents};
    let mut s = routed_fixture();
    outline(&mut s);
    let before = view(&s).fabrication_outputs().unwrap();
    let original = encode_documents(s.snapshot().unwrap()).unwrap();
    let footprint = view(&s).footprints().next().unwrap();
    let id = footprint.id.clone();
    let pads = footprint.pads;
    let offset = Point { x_nm: 2_000_000, y_nm: -1_000_000 };
    let revision = s.revision();
    let params = json!({"footprint":id,"offset":offset,"expected_revision":revision});
    s.apply(crate::SET_FOOTPRINT_REFERENCE_OFFSET, &params).unwrap();
    assert!(s.apply(crate::SET_FOOTPRINT_REFERENCE_OFFSET, &params).is_err());
    assert_eq!(view(&s).footprints().next().unwrap().pads, pads);
    let after = view(&s).fabrication_outputs().unwrap();
    assert_eq!(
        before.iter().zip(&after).filter(|(a, b)| a != b).map(|(_, b)| b.name.as_str()).collect::<Vec<_>>(),
        [after.iter().find(|output| output.name.ends_with("-Silkscreen_Top.gbr")).unwrap().name.as_str()]
    );
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    assert!(String::from_utf8_lossy(&documents[&crate::persistence::test_board_path(&documents)]).contains("pcbcraft-board/15"));
    assert_eq!(encode_documents(decode_documents(&documents).unwrap()).unwrap(), documents);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
    s.apply("edit.redo", &json!({})).unwrap();
    let revision = s.revision();
    s.apply(
        crate::SET_FOOTPRINT_REFERENCE_OFFSET,
        &json!({"footprint":id,"offset":crate::board::DEFAULT_REFERENCE_OFFSET,"expected_revision":revision}),
    )
    .unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), original);
}

/// Format 15 retains routes, vias and bottom-side geometry, with reference offsets transformed locally.
#[test]
fn moved_reference_round_trips_with_bottom_tracks_and_vias() {
    use crate::persistence::{decode_documents, encode_documents};
    let mut session = routed_fixture();
    let [front, back] = copper_layers(&session)[..] else { panic!("two copper layers") };
    route_with_vias(
        &mut session,
        front,
        &[Point { x_nm: 0, y_nm: 20_000_000 }, Point { x_nm: 3_000_000, y_nm: 20_000_000 }, Point { x_nm: 6_000_000, y_nm: 20_000_000 }],
        &[(1, back)],
    )
    .unwrap();
    let footprint = view(&session).footprints().next().unwrap();
    let id = footprint.id.clone();
    let origin = footprint.position;
    session
        .apply(
            crate::SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":id,"x_nm":origin.x_nm,"y_nm":origin.y_nm,"rotation":90,"side":"back","expected_revision":session.revision()}),
        )
        .unwrap();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    let before = view(&session).footprints().find(|f| f.id == id).unwrap();
    let points: Vec<_> = before.graphics.iter().filter(|g| g.reference).flat_map(|g| g.points.iter().copied()).collect();
    let pads = before.pads;
    session
        .apply(
            crate::SET_FOOTPRINT_REFERENCE_OFFSET,
            &json!({"footprint":id,"offset":{"x_nm":2_000_000,"y_nm":-1_000_000},"expected_revision":session.revision()}),
        )
        .unwrap();
    let moved = view(&session).footprints().find(|f| f.id == id).unwrap();
    assert_eq!(moved.pads, pads);
    let actual: Vec<_> = moved.graphics.iter().filter(|g| g.reference).flat_map(|g| g.points.iter().copied()).collect();
    let expected: Vec<_> = points.iter().map(|p| Point { x_nm: p.x_nm - 1_000_000, y_nm: p.y_nm + 2_000_000 }).collect();
    assert!(!expected.is_empty());
    assert_eq!(actual, expected, "mirror local X before the quarter turn");
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    let board: serde_json::Value = serde_json::from_slice(&saved[&crate::persistence::test_board_path(&saved)]).unwrap();
    assert_eq!(board["format"], "pcbcraft-board/15");
    assert!(!board["tracks"].as_array().unwrap().is_empty());
    assert!(!board["vias"].as_array().unwrap().is_empty());
    assert!(board["footprints"].as_array().unwrap().iter().any(|f| f["bottom"] == true));
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    session
        .apply(
            crate::SET_FOOTPRINT_REFERENCE_OFFSET,
            &json!({"footprint":id,"offset":crate::board::DEFAULT_REFERENCE_OFFSET,"expected_revision":session.revision()}),
        )
        .unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), original);
}

/// A routed two-layer board produces each layer file with the right content: pad flashes and
/// tracks on their copper, vias on both layers, mask and paste openings for front SMD pads,
/// the silkscreen, the outline, and one drill tool per hole size. Y is negated throughout.
#[test]
fn a_routed_board_produces_gerber_and_drill_files() {
    let mut s = routed_fixture();
    outline(&mut s);
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let y = 20_000_000;
    route(&mut s, front, &[Point { x_nm: 1_000_000, y_nm: y }, Point { x_nm: 3_000_000, y_nm: y }]).unwrap();
    route_with_vias(
        &mut s,
        front,
        &[Point { x_nm: 5_000_000, y_nm: y }, Point { x_nm: 7_000_000, y_nm: y }, Point { x_nm: 9_000_000, y_nm: y }],
        &[(1, back)],
    )
    .unwrap();
    let outputs = view(&s).fabrication_outputs().unwrap();
    let names: Vec<_> = outputs.iter().map(|output| output.name.as_str()).collect();
    for suffix in [
        "-Top.gbr",
        "-Bottom.gbr",
        "-Mask_Top.gbr",
        "-Mask_Bottom.gbr",
        "-Paste_Top.gbr",
        "-Paste_Bottom.gbr",
        "-Silkscreen_Top.gbr",
        "-Outline.gbr",
        "-PTH.drl",
    ] {
        assert!(names.iter().any(|name| name.ends_with(suffix)), "{suffix} in {names:?}");
    }
    for output in outputs.iter().filter(|output| output.name.ends_with(".gbr")) {
        let text = &output.contents;
        assert!(text.contains("%FSLAX46Y46*%") && text.contains("%MOMM*%") && text.ends_with("M02*\n"), "{}", output.name);
        assert!(text.lines().all(|line| line.ends_with('*') || line.ends_with("*%") || line == "%"), "{}: Gerber command delimiters", output.name);
    }

    let pads: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).collect();
    let top = file(&outputs, "-Top.gbr");
    assert!(top.contains("%TF.FileFunction,Copper,L1,Top*%"));
    assert_eq!(read_gerber(&outputs, TOP_COPPER).rounded_flashes.len(), pads.len(), "one flash per front pad");
    assert!(top.contains("C,0.250000*%"), "the 0.25 mm track aperture");
    assert!(top.contains(&format!("X1000000Y{}D02*\nX3000000Y{}D01*", -y, -y)), "the front track, Y negated");
    assert!(top.contains(&format!("X7000000Y{}D03*", -y)), "the via is flashed on the top layer");
    let bottom = file(&outputs, "-Bottom.gbr");
    assert!(bottom.contains("%TF.FileFunction,Copper,L2,Bot*%"));
    assert!(read_gerber(&outputs, BOTTOM[0]).rounded_flashes.is_empty(), "SMD pads stay on the front");
    assert!(bottom.contains(&format!("X7000000Y{}D03*", -y)) && bottom.contains(&format!("X9000000Y{}D01*", -y)), "via and back track");
    assert_eq!(read_gerber(&outputs, TOP[1]).rounded_flashes.len(), pads.len());
    assert!(file(&outputs, "-Mask_Top.gbr").contains("%TF.FilePolarity,Negative*%"));
    assert_eq!(read_gerber(&outputs, TOP[2]).rounded_flashes.len(), pads.len());
    assert!(read_gerber(&outputs, BOTTOM_PASTE).rounded_flashes.is_empty());
    let profile = file(&outputs, "-Outline.gbr");
    assert!(profile.contains("%TF.FileFunction,Profile,NP*%"));
    assert_eq!(profile.matches("D01*").count(), 4, "a closed rectangle is four strokes");

    let drill = file(&outputs, "-PTH.drl");
    assert!(drill.starts_with("M48\n") && drill.ends_with("M30\n"));
    assert!(drill.contains("T1C0.300\n"), "{drill}");
    assert!(drill.contains("X7.000Y-20.000\n"), "{drill}");
}

/// Read the JSON actually stored in a captured ZIP.
fn embedded_manifest(capture: &Archive) -> Value {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(capture.bytes())).unwrap();
    serde_json::from_reader(archive.by_name("manifest.json").unwrap()).unwrap()
}

/// The same board gives byte-identical outputs and archives, including rounded pad macros.
/// The sole extra member describes the exact generated bytes in order, not itself or the
/// outer ZIP. All entries have fixed metadata.
#[test]
fn embedded_manifest_matches_every_generated_member() {
    let mut session = routed_fixture();
    outline(&mut session);
    let outputs = session.export_fabrication(&json!({})).unwrap();
    assert!(!read_gerber(&outputs, TOP_COPPER).rounded_flashes.is_empty(), "the archive fixture exercises macros");
    assert_eq!(outputs, session.export_fabrication(&json!({})).unwrap());
    let capture = session.export_fabrication_archive(&json!({})).unwrap();
    assert_eq!(capture.bytes(), zip(&outputs).unwrap());
    assert_eq!(capture, session.export_fabrication_archive(&json!({})).unwrap());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(capture.bytes())).unwrap();
    let names: Vec<_> = archive.file_names().map(|name| name.unwrap().into_owned()).collect();
    assert_eq!(names.iter().filter(|name| name.as_str() == "manifest.json").count(), 1);
    assert!(!outputs.is_empty());
    assert_eq!(archive.len(), outputs.len() + 1);
    let mut records = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        assert_eq!(entry.system(), zip::System::Unix);
        assert_eq!(entry.unix_mode(), Some(0o100644));
        assert_eq!(entry.last_modified(), Some(zip::DateTime::default()));
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if let Some(output) = outputs.get(index) {
            assert_eq!(entry.name().unwrap(), output.name);
            assert_eq!(bytes, output.contents.as_bytes());
            records.push(json!({"name": output.name, "size_bytes": bytes.len(), "sha256": format!("{:x}", Sha256::digest(&bytes))}));
        } else {
            assert_eq!(entry.name().unwrap(), "manifest.json");
            assert_eq!(bytes.last(), Some(&b'\n'));
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!({"generator_version": env!("CARGO_PKG_VERSION"), "files": records}));
        }
    }
}

/// Byte sizes and digests include multibyte UTF-8 and preserve LF versus CRLF without normalization.
#[test]
fn manifest_counts_bytes_not_characters_or_lines() {
    let outputs = [Output { name: "text.txt".into(), contents: "é\r\nλ\n".into() }];
    let capture = Archive::new(&outputs).unwrap();
    let manifest = embedded_manifest(&capture);
    let record = &manifest["files"][0];
    assert_eq!(record["size_bytes"], 7);
    assert_eq!(record["sha256"], format!("{:x}", Sha256::digest(b"\xc3\xa9\r\n\xce\xbb\n")));
    assert_ne!(record["sha256"], format!("{:x}", Sha256::digest("é\nλ\n".as_bytes())));
}

/// Generic callers cannot shadow the embedded manifest with another file of that name.
#[test]
fn archive_reserves_the_manifest_member_name() {
    for name in ["manifest.json", "MANIFEST.JSON"] {
        let output = Output { name: name.into(), contents: "not the manifest".into() };
        assert!(Archive::new(&[output]).unwrap_err().contains("manifest.json is reserved"));
    }
}

/// The command result and embedded manifest agree. Neither export edits documents, revision or
/// undo/redo; a later edit affects the new capture but not the earlier ZIP or its manifest.
#[test]
fn manifest_is_read_only_and_stays_bound_to_its_capture() {
    use crate::persistence::encode_documents;
    let mut session = routed_fixture();
    outline(&mut session);
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let revision = session.revision();
    let history = (session.undo.len(), session.redo.len());
    let capture = session.export_fabrication_archive(&json!({})).unwrap();
    let manifest = embedded_manifest(&capture);
    let result = session.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    assert_eq!(result["fabrication"]["manifest"], manifest);
    assert_eq!(result["revision"], revision);
    assert_eq!(session.revision(), revision);
    assert_eq!((session.undo.len(), session.redo.len()), history);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    let [front, _] = copper_layers(&session)[..] else { panic!("two copper layers") };
    route(&mut session, front, &[Point { x_nm: 1_000_000, y_nm: 20_000_000 }, Point { x_nm: 3_000_000, y_nm: 20_000_000 }]).unwrap();
    let changed = session.export_fabrication_archive(&json!({})).unwrap();
    assert_ne!(capture.bytes(), changed.bytes());
    assert_ne!(manifest["files"], embedded_manifest(&changed)["files"]);
    assert_eq!(embedded_manifest(&capture), manifest);
}

/// The command edits nothing and returns every file through execute.
#[test]
fn export_command_returns_files_without_editing() {
    let mut s = routed_fixture();
    outline(&mut s);
    let revision = s.revision();
    let result = s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    assert_eq!(result["revision"], revision);
    assert_eq!(s.revision(), revision);
    let files = result["fabrication"]["files"].as_array().unwrap();
    assert!(files.iter().any(|file| file["name"].as_str().unwrap().ends_with("-PTH.drl")));
}

/// Sanitization and case folding cannot merge copper files with each other or the fixed outputs.
/// Names and ZIP bytes remain stable, and fixed file functions keep their conventional names.
#[test]
fn colliding_layer_names_produce_unique_deterministic_archives() {
    for second in ["F_Cu", "f_Cu"] {
        let mut s = Session::default();
        s.apply("project.new", &json!({"create_board":true})).unwrap();
        s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":6,"expected_revision":s.revision()})).unwrap();
        let mut setup = s.inspect()["project"]["board"]["setup"].clone();
        let names = ["F.Cu", second, "Mask_Top", "Paste_Top", "Silkscreen_Top", "Outline"];
        let order: Vec<_> = view(&s)
            .setup()
            .unwrap()
            .stackup()
            .strata
            .iter()
            .filter_map(|stratum| if let Stratum::Copper { layer, .. } = stratum { Some(*layer) } else { None })
            .collect();
        assert_eq!(order.len(), names.len());
        for (id, name) in order.into_iter().zip(names) {
            let layer = setup["layers"].as_array_mut().unwrap().iter_mut().find(|layer| layer["id"] == json!(id)).unwrap();
            layer["name"] = json!(name);
        }
        s.apply(crate::SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":s.revision()})).unwrap();
        outline(&mut s);
        let outputs = view(&s).fabrication_outputs().unwrap();
        let unique: BTreeSet<_> = outputs.iter().map(|output| output.name.to_ascii_lowercase()).collect();
        assert_eq!(unique.len(), outputs.len());
        assert!(outputs[0].name.ends_with("-F_Cu.gbr"));
        assert!(outputs[1].name.ends_with("-2.gbr"));
        assert!(outputs[2].name.ends_with("-Mask_Top-2.gbr"));
        assert!(file(&outputs, "-Mask_Top.gbr").contains("%TF.FileFunction,Soldermask,Top*%"));
        assert!(file(&outputs, "-Silkscreen_Top.gbr").contains("%TF.FileFunction,Legend,Top*%"));
        assert!(file(&outputs, "-Outline.gbr").contains("%TF.FileFunction,Profile,NP*%"));
        assert_eq!(outputs, view(&s).fabrication_outputs().unwrap());
        assert_eq!(zip(&outputs).unwrap(), zip(&view(&s).fabrication_outputs().unwrap()).unwrap());
    }
}

/// Footprint graphics are already transformed into board space; export only flips their Y axis.
#[test]
fn translated_rotated_silkscreen_preserves_board_coordinates() {
    let mut s = routed_fixture();
    outline(&mut s);
    let id = view(&s).footprints().next().unwrap().id;
    s.apply(
        crate::SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":id,"x_nm":30_000_000,"y_nm":-25_000_000,"rotation":90,"expected_revision":s.revision()}),
    )
    .unwrap();
    let footprint = view(&s).footprints().find(|footprint| footprint.id == id).unwrap();
    assert_eq!(footprint.position, Point { x_nm: 30_000_000, y_nm: -25_000_000 });
    assert_eq!(footprint.rotation, 90);
    let graphic = footprint.graphics.iter().find(|graphic| graphic.purpose == GraphicPurpose::Silkscreen && graphic.points.len() >= 2).unwrap();
    let (from, to) = (graphic.points[0], graphic.points[1]);
    let expected = format!("X{}Y{}D02*\nX{}Y{}D01*", from.x_nm, -from.y_nm, to.x_nm, -to.y_nm);
    let outputs = view(&s).fabrication_outputs().unwrap();
    assert!(file(&outputs, "-Silkscreen_Top.gbr").contains(&expected), "{expected}");
}

/// Both command entry points reject the same invalid targets, parameters and unavailable boards,
/// and refusals retain revision, snapshot and undo/redo availability.
#[test]
fn fabrication_apply_and_execute_share_validation() {
    let mut s = Session::default();
    for phase in 0..3 {
        if phase == 1 {
            s.apply("project.new", &json!({"create_board":true})).unwrap();
        } else if phase == 2 {
            s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":s.revision()})).unwrap();
        }
        let before = s.inspect();
        for params in [json!({}), json!({"board":true}), json!({"board":uuid::Uuid::from_u128(1)})] {
            let applied = s.apply(EXPORT_FABRICATION, &params).unwrap_err().to_string();
            assert_eq!(s.execute(EXPORT_FABRICATION, &params).unwrap_err().to_string(), applied);
            assert_eq!(s.inspect(), before);
        }
    }
    outline(&mut s);
    let before = s.inspect();
    assert_eq!(s.apply(EXPORT_FABRICATION, &json!({})).unwrap(), s.revision());
    let exported = s.execute(EXPORT_FABRICATION, &json!({})).unwrap();
    assert!(!exported["fabrication"]["files"].as_array().unwrap().is_empty());
    assert_eq!(s.inspect(), before);
}
