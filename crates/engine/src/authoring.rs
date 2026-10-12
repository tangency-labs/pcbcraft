use super::*;
use pcbcraft_library::{FootprintDefinition, SymbolDefinition};

pub(super) fn commands() -> Vec<CommandSpec> {
    [
        ("library.symbol.upsert","Create or replace a project-library symbol; existing placed definitions remain embedded",schema(json!({"definition":{"type":"object"},"notice":{"type":"string"}}),&["definition"])),
        ("library.footprint.upsert","Create or replace a project-library footprint",schema(json!({"definition":{"type":"object"},"notice":{"type":"string"}}),&["definition"])),
        ("library.symbol.remove","Remove a project-library symbol; placed instances retain their embedded definition",schema(json!({"name":{"type":"string"}}),&["name"])),
        ("library.footprint.remove","Remove a project-library footprint; placed instances retain their embedded definition",schema(json!({"name":{"type":"string"}}),&["name"])),
        ("library.load","Load and validate a .pcbcraft-lib collection into the project as one undoable transaction",schema(json!({"path":{"type":"string"}}),&["path"])),
        ("component.place_custom","Place a symbol from the editable project library",schema(json!({"name":{"type":"string"},"position":point_schema(),"reference":{"type":"string"},"value":{"type":"string"}}),&["name","position"])),
        ("footprint.assign_custom","Assign a project-library footprint whose pad numbers match the component pins",schema(json!({"id":id_schema(),"name":{"type":"string"}}),&["id","name"])),
    ].into_iter().map(|(id,description,input_schema)|CommandSpec {id,description,input_schema}).collect()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SymbolParams {
    definition: SymbolDefinition,
    notice: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FootprintParams {
    definition: FootprintDefinition,
    notice: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Place {
    name: String,
    position: Point,
    reference: Option<String>,
    value: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Assign {
    id: Id,
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LibraryFile {
    format: String,
    version: u32,
    library: ProjectLibrary,
}
pub(super) fn run(
    p: &mut Project,
    next: &mut Id,
    command: &str,
    params: &Value,
) -> Option<Result<Value, Error>> {
    if !commands().iter().any(|c| c.id == command) {
        return None;
    }
    Some((|| match command {
        "library.symbol.upsert" => {
            let a: SymbolParams = parse(params)?;
            a.definition
                .validate()
                .map_err(|e| Error::Invalid(e.to_string()))?;
            let name = a.definition.name.clone();
            if let Some(notice) = a.notice
                && !p.library.notices.contains(&notice)
            {
                p.library.notices.push(notice);
            }
            if let Some(def) = p.library.symbols.iter_mut().find(|s| s.name == name) {
                *def = a.definition;
            } else {
                p.library.symbols.push(a.definition);
            }
            p.library.symbols.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(json!({"name":name}))
        }
        "library.footprint.upsert" => {
            let a: FootprintParams = parse(params)?;
            a.definition
                .validate()
                .map_err(|e| Error::Invalid(e.to_string()))?;
            let name = a.definition.name.clone();
            if let Some(notice) = a.notice
                && !p.library.notices.contains(&notice)
            {
                p.library.notices.push(notice);
            }
            if let Some(def) = p.library.footprints.iter_mut().find(|s| s.name == name) {
                *def = a.definition;
            } else {
                p.library.footprints.push(a.definition);
            }
            p.library.footprints.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(json!({"name":name}))
        }
        "library.symbol.remove" | "library.footprint.remove" => {
            let a: Name = parse(params)?;
            if command == "library.symbol.remove" {
                let before = p.library.symbols.len();
                p.library.symbols.retain(|s| s.name != a.name);
                if before == p.library.symbols.len() {
                    return Err(missing());
                }
            } else {
                let before = p.library.footprints.len();
                p.library.footprints.retain(|s| s.name != a.name);
                if before == p.library.footprints.len() {
                    return Err(missing());
                }
            }
            Ok(json!({"name":a.name}))
        }
        "library.load" => {
            let a: PathParams = parse(params)?;
            let text = pcbcraft_io::read_text(std::path::Path::new(&a.path))?;
            let file: LibraryFile =
                serde_json::from_str(&text).map_err(|e| Error::Invalid(e.to_string()))?;
            if file.format != "pcbcraft-library" || file.version != 1 {
                return Err(Error::Invalid(
                    "unsupported library file format/version".into(),
                ));
            }
            file.library.validate()?;
            p.library = file.library;
            Ok(json!({"symbols":p.library.symbols.len(),"footprints":p.library.footprints.len()}))
        }
        "component.place_custom" => {
            let a: Place = parse(params)?;
            let def = p
                .library
                .symbols
                .iter()
                .find(|s| s.name == a.name)
                .cloned()
                .ok_or_else(missing)?;
            place_definition(
                p,
                next,
                "project".into(),
                def,
                a.position,
                a.reference,
                a.value,
            )
        }
        "footprint.assign_custom" => {
            let a: Assign = parse(params)?;
            let def = p
                .library
                .footprints
                .iter()
                .find(|s| s.name == a.name)
                .cloned()
                .ok_or_else(missing)?;
            assign_definition(p, a.id, "project".into(), def)
        }
        _ => Err(Error::Invalid("unknown library authoring command".into())),
    })())
}
