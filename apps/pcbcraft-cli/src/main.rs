use pcbcraft_engine::{Command, Session};
use serde_json::{Value, json};
use std::path::Path;

fn option<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let action = args.first().map(String::as_str).unwrap_or("help");
    let mut session = if args.iter().any(|a| a == "--sample") {
        Session::sample().map_err(|e| e.to_string())?
    } else {
        Session::default()
    };
    if let Some(path) = option(&args, "--project") {
        session
            .replace(pcbcraft_io::load(Path::new(path))?)
            .map_err(|e| e.to_string())?;
    }
    match action {
        "commands" => println!("{}", json!({"commands":pcbcraft_engine::commands()})),
        "inspect" => println!("{}", session.inspect()),
        "checks" => println!("{}", session.checks()),
        "tool" => {
            let name = args
                .get(1)
                .ok_or("usage: pcbcraft-cli tool NAME --args JSON")?;
            let input: Value = serde_json::from_str(option(&args, "--args").unwrap_or("{}"))
                .map_err(|e| e.to_string())?;
            let shared = pcbcraft_mcp::shared(session);
            let result = pcbcraft_mcp::call_tool(&shared, name, &input);
            if result.get("isError").and_then(Value::as_bool) == Some(true) {
                return Err(result
                    .pointer("/content/0/text")
                    .and_then(Value::as_str)
                    .unwrap_or("tool failed")
                    .into());
            }
            if let Some(path) = option(&args, "--save") {
                let current = shared.lock().map_err(|_| "session unavailable")?;
                pcbcraft_io::save(&current.project, Path::new(path))?;
            }
            println!("{result}");
        }
        "mcp" => pcbcraft_mcp::serve(
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
            &pcbcraft_mcp::shared(session),
            option(&args, "--connect"),
        )?,
        "run" => {
            let path = args
                .get(1)
                .ok_or("usage: pcbcraft-cli run script.json --save design.pcbcraft")?;
            let text = pcbcraft_io::read_text(Path::new(path))?;
            let script: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let commands: Vec<Command> = serde_json::from_value(
                script
                    .get("commands")
                    .cloned()
                    .ok_or("script needs commands array")?,
            )
            .map_err(|e| e.to_string())?;
            let result = session.batch(&commands, None).map_err(|e| e.to_string())?;
            if let Some(path) = option(&args, "--save") {
                pcbcraft_io::save(&session.project, Path::new(path))?;
            }
            if let Some(path) = option(&args, "--export") {
                pcbcraft_io::atomic_write(
                    Path::new(path),
                    pcbcraft_io::kicad_board(&session.project)?.as_bytes(),
                )?;
            }
            println!("{result}");
        }
        "sample" => {
            let path = option(&args, "--out").ok_or("sample requires --out design.pcbcraft")?;
            let sample = Session::sample().map_err(|e| e.to_string())?;
            pcbcraft_io::save(&sample.project, Path::new(path))?;
            println!("{}", json!({"path":path}));
        }
        "export" => {
            let path = option(&args, "--out").ok_or("export requires --out board.kicad_pcb")?;
            pcbcraft_io::atomic_write(
                Path::new(path),
                pcbcraft_io::kicad_board(&session.project)?.as_bytes(),
            )?;
            println!("{}", json!({"path":path}));
        }
        "snapshot" => {
            let path = option(&args, "--out").ok_or("snapshot requires --out preview.png")?;
            let view = option(&args, "--view").unwrap_or("pcb");
            let bytes = pcbcraft_io::preview_png(&session.project, view)?;
            pcbcraft_io::atomic_write(Path::new(path), &bytes)?;
            println!("{}", json!({"path":path,"view":view}));
        }
        "help" | "--help" | "-h" => println!(
            "PCBCraft — Rust EDA, MCP-first\n\ncommands | inspect | checks | sample | run | export | snapshot | mcp\n\nCommon: --sample or --project FILE\nMCP: pcbcraft-cli mcp [--connect 127.0.0.1:7991]\nDesktop: pcbcraft --sample --control 127.0.0.1:7991\nSee README.md and docs/mcp.md for schemas and examples."
        ),
        _ => return Err(format!("unknown action: {action}")),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("pcbcraft-cli: {error}");
        std::process::exit(1);
    }
}
