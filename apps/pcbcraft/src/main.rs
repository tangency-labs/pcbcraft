fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let option = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "PCBCraft — one-window Rust EDA\n\n--sample | --project FILE\n--control 127.0.0.1:7991\n--tab schematic|pcb|libraries|checks\n--light\n--capture /absolute/path.png"
        );
        return Ok(());
    }
    let mut session = if args.iter().any(|a| a == "--sample") {
        match pcbcraft_engine::Session::sample() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("sample: {e}");
                std::process::exit(1);
            }
        }
    } else {
        pcbcraft_engine::Session::default()
    };
    if let Some(path) = option("--project") {
        let result = pcbcraft_io::load(std::path::Path::new(&path))
            .and_then(|project| session.replace(project).map_err(|e| e.to_string()));
        if let Err(e) = result {
            eprintln!("project: {e}");
            std::process::exit(1);
        }
    }
    let shared = pcbcraft_mcp::shared(session);
    if let Some(address) = option("--control")
        && let Err(e) = pcbcraft_mcp::start_bridge(shared.clone(), &address)
    {
        eprintln!("control bridge: {e}");
        std::process::exit(1);
    }
    let capture = option("--capture");
    let tab = option("--tab");
    let light = args.iter().any(|a| a == "--light");
    let project_path = option("--project");
    let options = eframe::NativeOptions {
        viewport: egui_viewport(),
        ..Default::default()
    };
    eframe::run_native(
        "PCBCraft",
        options,
        Box::new(move |cc| {
            Ok(Box::new(
                pcbcraft_ui_egui::App::new(cc, shared, capture, tab.as_deref(), light)
                    .with_project_path(project_path),
            ))
        }),
    )
}
fn egui_viewport() -> eframe::egui::ViewportBuilder {
    eframe::egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 940.0])
        .with_min_inner_size([1000.0, 680.0])
}
