//! Project-owned library browsing shares source presentation without inventing registrations.
use super::*;
use pcbcraft_engine::{Project, library::sources::inventory::Inventory};
use std::sync::{Arc, Weak};

/// Share one inventory across panes, invalidated by immutable snapshot identity (including Undo).
#[derive(Debug, Default)]
pub(crate) struct Cache {
    snapshot: Weak<Project>,
    inventory: Option<Result<Arc<Inventory>, String>>,
}

impl PcbApp {
    /// Live clients inspect the same pinned rows and usage as the Project table, without host paths.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn inspect_library_workspace(&self, state: &State) -> Value {
        let mut value = state.inspect();
        if state.project.is_some() && state.project == self.session.project().map(|project| project.id()) {
            match self.project_library_inventory() {
                Ok(inventory) => value["items"] = json!(inventory.entries().filter(|entry| state.includes_project(&entry.item.key)).map(|entry| {
                    json!({"item":entry.item.inspect(),"used_by":entry.used_by,"source_status":inventory.status(&entry.item,self.session.libraries())})
                }).collect::<Vec<_>>()),
                Err(error) => value["error"] = json!(error),
            }
        }
        value
    }
    /// Validate navigation completely before replacing a pane's disposable state. Project addresses
    /// are resolved only against the current project and ambiguous unqualified selections fail.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn library_navigation(&self, view: ViewId, params: &Value) -> Result<State, String> {
        let mut state = self.views.view(view).and_then(|view| view.library.clone()).ok_or("Library pane is unavailable")?;
        let previous = (state.source.clone(), state.project, state.project_library.clone(), state.kind);
        let scope = params["scope"].as_str().unwrap_or(if params.get("source").is_some() || state.project.is_none() { "sources" } else { "project" });
        match scope {
            "project" => {
                if params.get("source").is_some() {
                    return Err("Project browsing takes a library filter, not a source registration".into());
                }
                let project = self.session.project().ok_or("Open a project first")?.id();
                if state.project != Some(project) {
                    state.project_library = None;
                    state.selected = None;
                }
                state.project = Some(project);
                state.source = None;
                if let Some(library) = params["library"].as_str() {
                    if !library.is_empty()
                        && library != "project-only"
                        && !self.project_library_inventory()?.entries().any(|entry| entry.item.key.library == library)
                    {
                        return Err("Library is not pinned in this project".into());
                    }
                    state.project_library = (!library.is_empty()).then(|| library.into());
                }
            }
            "sources" => {
                if params.get("library").is_some() {
                    return Err("Use a source registration when browsing sources".into());
                }
                state.project = None;
                state.project_library = None;
                if let Some(source) = params["source"].as_str() {
                    if !self.session.libraries().inspect().as_array().into_iter().flatten().any(|entry| entry["source"] == source)
                        && !self.library_sources.registered.iter().any(|entry| entry.source == source)
                    {
                        return Err("Unknown library source".into());
                    }
                    state.source = Some(source.into());
                }
            }
            _ => return Err("Choose project or sources scope".into()),
        }
        if let Some(kind) = params["type"].as_u64() {
            if kind > 4 {
                return Err("Choose Type 0 through 4".into());
            }
            state.kind = kind as usize;
        }
        if let Some(query) = params["query"].as_str() {
            if query.len() > 512 {
                return Err("Library query exceeds 512 bytes".into());
            }
            state.query = query.into();
        }
        if previous != (state.source.clone(), state.project, state.project_library.clone(), state.kind) {
            state.selected = None;
        }
        if params.get("definition_revision").is_some() && params.get("item").is_none() {
            return Err("An exact revision requires an item ID.".into());
        }
        if let Some(id) = params["item"].as_str() {
            let inventory = state.project.map(|_| self.project_library_inventory()).transpose()?;
            let items: Vec<_> = if let Some(inventory) = &inventory {
                inventory.entries().map(|entry| &entry.item).filter(|item| state.includes_project(&item.key)).collect()
            } else {
                self.session.libraries().source_items(state.source.as_deref().unwrap_or("")).iter().collect()
            };
            let matches: Vec<_> = items
                .into_iter()
                .filter(|item| {
                    item.key.id == id
                        && (state.kind == 0 || kind_index(item.key.kind) == state.kind)
                        && params["definition_revision"].as_u64().is_none_or(|revision| u64::from(item.key.revision) == revision)
                })
                .collect();
            if matches.iter().map(|item| (&item.key.library, item.key.kind)).collect::<BTreeSet<_>>().len() > 1 {
                return Err("Item is ambiguous; select its library and Type first".into());
            }
            state.selected =
                Some(matches.into_iter().max_by_key(|item| item.key.revision).ok_or("Item is unavailable for this scope and Type")?.key.clone());
        }
        Ok(state)
    }

    /// No project means no retained cache. An unchanged snapshot reuses the typed projection.
    pub(crate) fn project_library_inventory(&self) -> Result<Arc<Inventory>, String> {
        let mut cache = self.library_inventory.borrow_mut();
        let Some(snapshot) = self.session.snapshot() else {
            *cache = Cache::default();
            return Err("Open a project to browse its libraries".into());
        };
        if !cache.snapshot.upgrade().is_some_and(|previous| Arc::ptr_eq(&previous, &snapshot)) {
            cache.inventory = Some(Inventory::from_project(&snapshot).map(Arc::new));
            cache.snapshot = Arc::downgrade(&snapshot);
        }
        cache.inventory.clone().ok_or_else(|| "Project inventory is unavailable".to_owned())?
    }

    /// The current project's pins occupy their own tree, grouped by original collection.
    /// Project-only is a deliberate identity namespace, never a synonym for an offline source.
    pub(super) fn project_library_tree(&self, ui: &mut egui::Ui, state: &mut State, loaded: &Value) {
        let project = self.session.project().map(|project| project.id());
        let open = !state.closed.contains("Project");
        let inventory = self.project_library_inventory();
        let count = inventory
            .as_ref()
            .map(|inventory| inventory.entries().filter(|entry| entry.item.key.kind != Kind::Category).count().to_string())
            .unwrap_or_default();
        let (clicked, fold) = chrome::tree(
            ui,
            ui.make_persistent_id("project_libraries"),
            chrome::Node {
                label: "Project",
                depth: 0,
                fold: Some(open),
                selected: state.project.is_some() && state.project_library.is_none() && state.kind == 0,
                count: &count,
                readonly: false,
                problem: None,
                icon: ToolIcon::Shared("library"),
                accessible: "Project libraries",
            },
        );
        if fold {
            toggle(&mut state.closed, "Project".to_owned());
        }
        if clicked && let Some(project) = project {
            state.select_project(project, None, 0);
        }
        if !open {
            return;
        }
        let (Some(project), Ok(inventory)) = (project, inventory) else {
            ui.label(
                egui::RichText::new(if project.is_none() { "No project open" } else { "Project inventory unavailable" })
                    .small()
                    .color(pcbcraft_chrome::Tokens::get().text_dim),
            );
            return;
        };
        let mut libraries: BTreeSet<String> = inventory
            .entries()
            .filter(|entry| entry.item.key.kind != Kind::Category)
            .map(|entry| project_group(&entry.item.key.library).to_owned())
            .collect();
        // Keep the deliberate local destination visible even before the first variant exists.
        libraries.insert("project-only".into());
        for library in libraries {
            let name = self.project_collection_name(&inventory, &library, loaded);
            let entries: Vec<_> = inventory
                .entries()
                .filter(|entry| project_group(&entry.item.key.library) == library && entry.item.key.kind != Kind::Category)
                .collect();
            let id = format!("project/{library}");
            let open = !entries.is_empty() && !state.closed.contains(&id);
            let (clicked, fold) = chrome::tree(
                ui,
                ui.make_persistent_id(&id),
                chrome::Node {
                    label: &name,
                    depth: 1,
                    fold: (!entries.is_empty()).then_some(open),
                    selected: state.project == Some(project) && state.project_library.as_ref() == Some(&library) && state.kind == 0,
                    count: &entries.len().to_string(),
                    readonly: false,
                    problem: None,
                    icon: ToolIcon::Shared("folder"),
                    accessible: &format!("Project library {name}"),
                },
            );
            if fold {
                toggle(&mut state.closed, id.clone());
            }
            if clicked {
                state.select_project(project, Some(library.clone()), 0);
            }
            if open {
                for (kind, label, icon) in [
                    (1, "Symbols", ToolIcon::Symbol),
                    (2, "Footprints", ToolIcon::Footprint),
                    (3, "Generic parts", ToolIcon::Bom),
                    (4, "Orderable parts", ToolIcon::Shared("library")),
                ] {
                    let count = entries.iter().filter(|entry| kind_index(entry.item.key.kind) == kind).count().to_string();
                    if chrome::tree(
                        ui,
                        ui.make_persistent_id((&id, kind)),
                        chrome::Node {
                            label,
                            depth: 2,
                            fold: None,
                            selected: state.project == Some(project) && state.project_library.as_ref() == Some(&library) && state.kind == kind,
                            count: &count,
                            readonly: false,
                            problem: None,
                            icon,
                            accessible: &format!("Project {label} in {name}"),
                        },
                    )
                    .0
                    {
                        state.select_project(project, Some(library.clone()), kind);
                    }
                }
            }
        }
    }

    /// Prefer the registered display name, retaining saved attribution when that source is absent.
    fn project_collection_name(&self, inventory: &Inventory, library: &str, loaded: &Value) -> String {
        if library == "project-only" {
            return "Project-only".into();
        }
        loaded
            .as_array()
            .into_iter()
            .flatten()
            .find(|source| source["library"] == library)
            .and_then(|source| source["name"].as_str())
            .unwrap_or_else(|| inventory.library_name(library))
            .to_owned()
    }

    /// All exact project revisions remain visible; status compares sources without replacing pins.
    pub(super) fn project_library_items(&self, ui: &mut egui::Ui, state: &mut State, loaded: &Value) {
        let inventory = match self.project_library_inventory() {
            Ok(inventory) => inventory,
            Err(error) => {
                ui.label(error);
                return;
            }
        };
        let name = state
            .project_library
            .as_deref()
            .map(|library| self.project_collection_name(&inventory, library, loaded))
            .unwrap_or_else(|| "Project".into());
        chrome::title(ui, &name);
        if let Some(project) = self.session.project() {
            ui.label(egui::RichText::new(&project.name).small().color(pcbcraft_chrome::Tokens::get().text_dim));
        }
        let previous_kind = state.kind;
        chrome::toolbar(ui, &mut state.query, &mut state.kind);
        if state.kind != previous_kind {
            state.selected = None;
        }
        let query = state.query.trim().to_lowercase();
        let mut entries: Vec<_> = inventory
            .entries()
            .filter(|entry| {
                let item = &entry.item;
                item.key.kind != Kind::Category
                    && state.includes_project(&item.key)
                    && (state.kind == 0 || state.kind == kind_index(item.key.kind))
                    && format!("{} {} {} {}", item.name, item.key.library, item.key.id, entry.used_by.iter().cloned().collect::<Vec<_>>().join(" "))
                        .to_lowercase()
                        .contains(&query)
            })
            .collect();
        entries.sort_by_key(|entry| (entry.item.name.to_lowercase(), entry.item.key.clone()));
        let rows: Vec<_> = entries
            .iter()
            .map(|entry| {
                let item = &entry.item;
                chrome::TableEntry {
                    key: format!("Project {} {}/{} r{}", kind_name(item.key.kind), item.key.library, item.key.id, item.key.revision),
                    cells: vec![
                        item.name.clone(),
                        kind_name(item.key.kind).into(),
                        self.project_collection_name(&inventory, project_group(&item.key.library), loaded),
                        format!("r{}", item.key.revision),
                        entry.used_by.iter().cloned().collect::<Vec<_>>().join(", "),
                        inventory.status(item, self.session.libraries()).into(),
                    ],
                    selected: state.selected.as_ref() == Some(&item.key),
                    branch: None,
                    nested: false,
                }
            })
            .collect();
        ui.label(egui::RichText::new(format!("{} items", rows.len())).small().color(pcbcraft_chrome::Tokens::get().text_dim));
        chrome::table(ui, &["Name", "Type", "Library", "Rev", "Used by", "Source status"], &rows, |action| {
            if let chrome::ItemAction::Select(index) = action {
                state.selected = Some(entries[index].item.key.clone());
            }
        });
    }
}

impl State {
    /// Project selection is bound to the currently open project, independently of source IDs.
    pub(crate) fn project_id(&self) -> Option<uuid::Uuid> {
        self.project
    }
    /// Project navigation clears registration authority so source actions cannot target a stale row.
    fn select_project(&mut self, project: uuid::Uuid, library: Option<String>, kind: usize) {
        self.project = Some(project);
        self.project_library = library;
        self.source = None;
        self.kind = kind;
        self.selected = None;
        self.compact = 1;
    }
    /// A Project-only aggregate can contain several deliberate native variant namespaces.
    fn includes_project(&self, key: &Key) -> bool {
        self.project_library.as_deref().is_none_or(|library| project_group(&key.library) == library)
    }
}

/// The reserved native namespace, rather than source availability, identifies deliberate variants.
fn project_group(library: &str) -> &str {
    if library.starts_with("project-") { "project-only" } else { library }
}
