use super::{
    super::{Archive, Output},
    Finding, LayerRole, Mark, Package, point,
};

/// Build a real ZIP with staged file bytes and the exporter manifest.
fn package(files: &[(&str, &str)]) -> Archive {
    Archive::new(&files.iter().map(|(name, contents)| Output { name: (*name).into(), contents: (*contents).into() }).collect::<Vec<_>>()).unwrap()
}

fn gerber(function: &str, commands: &str) -> String {
    format!(
        "%TF.SameCoordinates,Original*%\n%TF.FileFunction,{function}*%\n%TF.FilePolarity,Positive*%\n%FSLAX46Y46*%\n%MOMM*%\n%LPD*%\nG01*\n{commands}M02*\n"
    )
}

#[test]
fn fabrication_review_reads_round_hits_and_finds_open_profile() {
    let outline = gerber("Profile,NP", "%ADD10C,0.100000*%\nD10*\nX0Y0D02*\nX2000000Y0D01*\n");
    let zip =
        package(&[("outline.gbr", &outline), ("plated.drl", "M48\n; TYPE=PLATED\nFMAT,2\nMETRIC\nT1C0.300\n%\nG90\nG05\nT1\nX1.250Y-0.500\nM30\n")]);
    let review = Package::read(zip.bytes()).unwrap();
    let drill = &review.layers[1];
    assert_eq!(drill.role, LayerRole::PlatedDrill);
    assert_eq!(drill.tools[0].hits, 1);
    assert!(matches!(drill.marks[0], Mark::Stroke { ends, diameter_nm: 300_000 } if ends == [point(1_250_000, -500_000); 2]));
    assert!(review.findings.contains(&Finding::OpenOutline { endpoints: 2, first: point(0, 0) }));
}

/// New exports correlate plot and drill frames even when the ZIP manifest validates each file.
#[test]
fn fabrication_review_requires_a_shared_export_frame() {
    let frame = "e39a1667-32ab-87fd-b5db-a97c013e3c58";
    let other = "f39a1667-32ab-87fd-b5db-a97c013e3c58";
    let outline = gerber("Profile,NP", "").replace("SameCoordinates,Original", &format!("SameCoordinates,{frame}"));
    let drill = |identity: &str| format!("M48\n; TYPE=PLATED\n; #@! TF.SameCoordinates,{identity}\nFMAT,2\nMETRIC\n%\nG90\nG05\nM30\n");
    let matching = package(&[("outline.gbr", &outline), ("plated.drl", &drill(frame))]);
    assert_eq!(Package::read(matching.bytes()).unwrap().layers.len(), 2);
    let mismatched = package(&[("outline.gbr", &outline), ("plated.drl", &drill(other))]);
    assert!(Package::read(mismatched.bytes()).unwrap_err().contains("fabrication layer coordinate frames differ"));
}

#[test]
fn fabrication_review_rejects_unverified_or_unsupported_geometry() {
    let outline = gerber("Profile,NP", "%LPC*%\n");
    let error = Package::read(package(&[("outline.gbr", &outline)]).bytes()).unwrap_err();
    assert!(error.contains("Unsupported Gerber record"), "{error}");

    let original = package(&[("outline.gbr", &outline)]);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(original.bytes())).unwrap();
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;
    let mut manifest = Vec::new();
    zip.by_name("manifest.json").unwrap().read_to_end(&mut manifest).unwrap();
    let mut tampered = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    tampered.start_file("outline.gbr", SimpleFileOptions::default()).unwrap();
    tampered.write_all(outline.replace("LPC", "LPD").as_bytes()).unwrap();
    tampered.start_file("manifest.json", SimpleFileOptions::default()).unwrap();
    tampered.write_all(&manifest).unwrap();
    let bytes = tampered.finish().unwrap().into_inner();
    assert!(Package::read(&bytes).unwrap_err().contains("SHA-256 differs"));
}

/// Exercise both the ZIP library's collapsed exact duplicates and the review's case-folded check.
#[test]
fn fabrication_review_rejects_duplicate_members() {
    let outline = gerber("Profile,NP", "");
    let bytes = package(&[("outline.gbr", &outline), ("Outline.gbr", &outline)]);
    assert!(Package::read(bytes.bytes()).unwrap_err().contains("Invalid or duplicate fabrication member name"));

    // ZIP writers disallow exact duplicates. Rename the second file in both ZIP headers.
    let mut bytes = bytes.bytes().to_vec();
    let name = b"Outline.gbr";
    let offsets: Vec<_> = bytes.windows(name.len()).enumerate().filter_map(|(offset, window)| (window == name).then_some(offset)).collect();
    assert_eq!(offsets.len(), 2);
    for offset in offsets {
        bytes[offset] = b'o';
    }
    assert!(Package::read(&bytes).unwrap_err().contains("duplicate member names"));
}

/// An isolated stroke stays open even when the plot traverses it in both directions.
#[test]
fn fabrication_review_counts_retraced_outline_edges_once() {
    let closed = "%ADD10C,0.100000*%\nD10*\nX0Y0D02*\nX100Y0D01*\nX100Y100D01*\nX0Y100D01*\nX0Y0D01*\n";
    let outline = gerber("Profile,NP", closed);
    let review = Package::read(package(&[("outline.gbr", &outline)]).bytes()).unwrap();
    assert!(!review.findings.iter().any(|finding| matches!(finding, Finding::OpenOutline { .. })));
    let outline = gerber("Profile,NP", &format!("{closed}X200Y0D02*\nX300Y0D01*\nX200Y0D01*\n"));
    let review = Package::read(package(&[("outline.gbr", &outline)]).bytes()).unwrap();
    assert!(review.findings.contains(&Finding::OpenOutline { endpoints: 2, first: point(200, 0) }));
}

/// The first or last header line cannot override the other, and plating must be present.
#[test]
fn fabrication_review_requires_one_drill_plating_declaration() {
    for declarations in ["", "; TYPE=PLATED\n; TYPE=NON_PLATED\n", "; TYPE=NON_PLATED\n; TYPE=PLATED\n", "; TYPE=PLATED\n; TYPE=PLATED\n"] {
        let drill = format!("M48\n{declarations}FMAT,2\nMETRIC\n%\nG90\nG05\nM30\n");
        let error = Package::read(package(&[("drill.drl", &drill)]).bytes()).unwrap_err();
        assert!(error.contains("plating"), "{error}");
    }
}

/// Historic TYPE comments and modern XNC file functions must identify the same plating.
#[test]
fn fabrication_review_checks_xnc_drill_plating() {
    let drill =
        |legacy_type: &str, xnc: &str| format!("M48\n; TYPE={legacy_type}\n{xnc}FMAT,2\nMETRIC\nT1C0.300\n%\nG90\nG05\nT1\nX1.000Y2.000\nM30\n");
    let plated = "; #@! TF.FileFunction,Plated,1,2,PTH\n";
    let non_plated = "; #@! TF.FileFunction,NonPlated,1,2,NPTH\n";
    for (legacy_type, xnc, role) in [("PLATED", "", LayerRole::PlatedDrill), ("NON_PLATED", non_plated, LayerRole::NonPlatedDrill)] {
        let zip = package(&[("drill.drl", &drill(legacy_type, xnc))]);
        let review = Package::read(zip.bytes()).unwrap();
        assert_eq!(review.layers[0].role, role);
        assert_eq!(review.layers[0].tools[0].hits, 1);
    }
    for (legacy_type, xnc) in [("PLATED", non_plated), ("NON_PLATED", plated)] {
        let zip = package(&[("drill.drl", &drill(legacy_type, xnc))]);
        assert!(Package::read(zip.bytes()).unwrap_err().contains("disagrees with the TYPE plating declaration"));
    }
    let duplicate = drill("PLATED", &format!("{plated}{plated}"));
    assert!(Package::read(package(&[("drill.drl", &duplicate)]).bytes()).unwrap_err().contains("Duplicate XNC drill file function"));
}

/// A second file attribute cannot override the interpretation of the entire plot.
#[test]
fn fabrication_review_rejects_redeclared_gerber_file_attributes() {
    for polarity in ["Positive", "Negative"] {
        let plot = gerber("Copper,L1,Top", &format!("%TF.FilePolarity,{polarity}*%\n"));
        assert!(Package::read(package(&[("copper.gbr", &plot)]).bytes()).unwrap_err().contains("Duplicate Gerber file polarity"));
    }
    for role in ["Copper,L1,Top", "Copper,L2,Bot"] {
        let plot = gerber("Copper,L1,Top", &format!("%TF.FileFunction,{role}*%\n"));
        assert!(Package::read(package(&[("copper.gbr", &plot)]).bytes()).unwrap_err().contains("Duplicate Gerber file function"));
    }
}

/// A single circle is a valid dot aperture; an empty macro would silently lose that flash.
#[test]
fn fabrication_review_rejects_empty_rounded_pad_macros() {
    let flash = "%\n%ADD10RoundRect0*%\nD10*\nX0Y0D03*\n";
    let empty = gerber("Copper,L1,Top", &format!("%AMRoundRect0*\n{flash}"));
    assert!(Package::read(package(&[("top.gbr", &empty)]).bytes()).unwrap_err().contains("macro has no primitives"));
    let dot = gerber("Copper,L1,Top", &format!("%AMRoundRect0*\n1,1,0.3,0,0,0*\n{flash}"));
    let review = Package::read(package(&[("top.gbr", &dot)]).bytes()).unwrap();
    assert_eq!(review.layers[0].marks.len(), 1);
}

/// Zero signed area can mean a collapsed pad or a bow tie whose lobes cancel.
#[test]
fn fabrication_review_distinguishes_collapsed_pad_from_crossed_region() {
    let bow_tie = gerber("Copper,L1,Top", "G36*\nX0Y0D02*\nX100Y100D01*\nX0Y100D01*\nX100Y0D01*\nX0Y0D01*\nG37*\n");
    assert!(Package::read(package(&[("top.gbr", &bow_tie)]).bytes()).unwrap_err().contains("zero signed area"));
    let collapsed = gerber("Copper,L1,Top", "G36*\nX0Y0D02*\nX0Y50D01*\nX0Y100D01*\nX0Y0D01*\nG37*\n");
    let review = Package::read(package(&[("top.gbr", &collapsed)]).bytes()).unwrap();
    assert!(review.layers[0].marks.is_empty());
}

/// A crossing contour with nonzero area can otherwise pass ear clipping.
#[test]
fn fabrication_review_rejects_crossing_regions_with_nonzero_area() {
    let crossed = gerber("Copper,L1,Top", "G36*\nX0Y0D02*\nX8Y-2D01*\nX4Y0D01*\nX6Y-4D01*\nX4Y4D01*\nX0Y0D01*\nG37*\n");
    assert!(Package::read(package(&[("top.gbr", &crossed)]).bytes()).unwrap_err().contains("Gerber region has crossing edges"));
    let simple = gerber("Copper,L1,Top", "G36*\nX0Y0D02*\nX6Y-4D01*\nX8Y-2D01*\nX4Y0D01*\nX4Y4D01*\nX0Y0D01*\nG37*\n");
    let review = Package::read(package(&[("top.gbr", &simple)]).bytes()).unwrap();
    assert_eq!(review.layers[0].marks.len(), 3);
}

/// A copper pour region joins its circular hole by a cut-in, uses multi-quadrant arcs and may
/// hold several contours; the review draws its exact area within the arc flattening. Arcs stay
/// refused outside pours, where the writer never emits them.
#[test]
fn fabrication_review_draws_pour_regions_with_cut_ins_and_arcs() {
    let pour = "G75*\n%TA.AperFunction,Conductor*%\nG36*\nX0Y0D02*\nX10000000Y0D01*\nX10000000Y5000000D01*\nX7000000Y5000000D01*\nG02*\n\
X5000000Y3000000I-2000000J0D01*\nX3000000Y5000000I0J2000000D01*\nX5000000Y7000000I2000000J0D01*\nX7000000Y5000000I0J-2000000D01*\nG01*\n\
X10000000Y5000000D01*\nX10000000Y10000000D01*\nX0Y10000000D01*\nX0Y0D01*\nX20000000Y0D02*\nX21000000Y0D01*\nX21000000Y1000000D01*\nX20000000Y1000000D01*\nX20000000Y0D01*\nG37*\n%TD.AperFunction*%\n";
    let review = Package::read(package(&[("top.gbr", &gerber("Copper,L1,Top", pour))]).bytes()).unwrap();
    let area: f64 = review.layers[0]
        .marks
        .iter()
        .map(|mark| match mark {
            Mark::Polygon(v) => {
                v.iter()
                    .zip(v.iter().cycle().skip(1))
                    .map(|(a, b)| (a.x_nm as f64) * (b.y_nm as f64) - (a.y_nm as f64) * (b.x_nm as f64))
                    .sum::<f64>()
                    / 2.
            }
            other => panic!("{other:?}"),
        })
        .sum();
    let exact = 101e12 - std::f64::consts::PI * 4e12;
    // Chords within 1 µm of the hole's circle add at most its perimeter times 1 µm of copper.
    let flattening = 2. * std::f64::consts::PI * 2e6 * 1e3;
    assert!(area > exact && area - exact < flattening, "{area} against {exact}");

    let open = pour.replace("X0Y0D01*\nX20000000Y0D02*", "X20000000Y0D02*");
    assert!(Package::read(package(&[("top.gbr", &gerber("Copper,L1,Top", &open))]).bytes()).unwrap_err().contains("Gerber region is open"));
    let pad_arc = "G75*\n%TA.AperFunction,SMDPad,CuDef*%\nG36*\nX0Y0D02*\nG02*\n";
    assert!(Package::read(package(&[("top.gbr", &gerber("Copper,L1,Top", pad_arc))]).bytes()).unwrap_err().contains("Unsupported Gerber record"));
}
