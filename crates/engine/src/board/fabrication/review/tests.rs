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
