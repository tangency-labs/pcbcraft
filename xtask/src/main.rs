use std::process::Command;
fn cargo(args: &[&str]) -> Result<(), String> {
    let status = Command::new("cargo")
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("cargo {} failed", args.join(" ")));
    }
    Ok(())
}
fn run() -> Result<(), String> {
    let action = std::env::args().nth(1).unwrap_or_else(|| "ci".into());
    match action.as_str() {
        "ci" => {
            layers()?;
            cargo(&["fmt", "--all", "--", "--check"])?;
            cargo(&[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ])?;
            cargo(&["test", "--workspace"])?;
            cargo(&[
                "check",
                "--target",
                "wasm32-unknown-unknown",
                "-p",
                "pcbcraft-ui-egui",
                "-p",
                "pcbcraft-web",
            ])?;
        }
        "layers" => layers()?,
        "wasm" => cargo(&[
            "check",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            "pcbcraft-ui-egui",
            "-p",
            "pcbcraft-web",
        ])?,
        _ => return Err("usage: cargo xtask [ci|wasm]".into()),
    }
    Ok(())
}
fn layers() -> Result<(), String> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("cargo metadata failed".into());
    }
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let layer = |name: &str| -> Option<u8> {
        match name {
            "pcbcraft-geom" => Some(0),
            "pcbcraft-doc" | "pcbcraft-library" => Some(1),
            "pcbcraft-rules" | "pcbcraft-io" => Some(2),
            "pcbcraft-engine" => Some(3),
            "pcbcraft-mcp" | "pcbcraft-ui-egui" => Some(4),
            "pcbcraft" | "pcbcraft-cli" | "pcbcraft-web" => Some(5),
            _ => None,
        }
    };
    let packages = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing metadata packages")?;
    for package in packages {
        let name = package
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or("missing package name")?;
        let Some(level) = layer(name) else {
            continue;
        };
        for dependency in package
            .get("dependencies")
            .and_then(serde_json::Value::as_array)
            .ok_or("missing dependencies")?
        {
            let dep = dependency
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or("missing dependency name")?;
            if layer(dep).is_some_and(|target| target > level)
                || (level < 4
                    && ["egui", "eframe", "wgpu", "winit", "rfd", "craft-ui"].contains(&dep))
            {
                return Err(format!("layer violation: {name} depends on {dep}"));
            }
        }
    }
    println!("dependency layers passed");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
