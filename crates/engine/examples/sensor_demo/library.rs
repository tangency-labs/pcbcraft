//! Original example-specific drawings; no third-party symbol or footprint assets are embedded.
use pcbcraft_engine::{
    Session,
    library::sources::{Documents, NativeSource},
};
use std::{error::Error, sync::Arc};

/// Load the exact example records through the normal bounded native source validator.
pub(super) fn install(session: &mut Session) -> Result<(), Box<dyn Error>> {
    let documents: Documents = [
        ("library.json", include_str!("../../../../examples/sensor-node-library/library.json")),
        ("symbols/attiny402.json", include_str!("../../../../examples/sensor-node-library/symbols/attiny402.json")),
        ("symbols/tmp102.json", include_str!("../../../../examples/sensor-node-library/symbols/tmp102.json")),
        ("footprints/soic8.json", include_str!("../../../../examples/sensor-node-library/footprints/soic8.json")),
        ("footprints/sot563.json", include_str!("../../../../examples/sensor-node-library/footprints/sot563.json")),
        ("footprints/header-1x03-254.json", include_str!("../../../../examples/sensor-node-library/footprints/header-1x03-254.json")),
        ("footprints/header-1x04-254.json", include_str!("../../../../examples/sensor-node-library/footprints/header-1x04-254.json")),
    ]
    .into_iter()
    .map(|(p, b)| (p.to_owned(), b.to_owned()))
    .collect();
    session.install_library_source("sensor-demo", Arc::new(NativeSource::load(&documents)?))?;
    Ok(())
}
