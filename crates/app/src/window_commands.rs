//! Window commands: the editor's view, window, settings and file controls as named commands, so
//! a live MCP client reaches them through the same handlers as the ribbon, sheet tabs, status
//! bar and Preferences. They change how the project is shown, never the project document, its
//! revision or its undo history. Each view or window is addressed by the ID inspection reports.

use egui::Context;

use crate::PcbApp;
#[cfg(not(target_arch = "wasm32"))]
pub(super) use live::{inspect_editor, window_catalog};

/// A contextual panel that a window shows or hides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Panel {
    Browser,
    Parts,
    Nets,
    Properties,
    Filters,
}

impl PcbApp {
    /// Show or hide one of the bound window's panels, as the View ribbon's toggles do. Opening
    /// the Parts Catalog focuses search, like Capture › Symbol; an open chooser is left alone.
    /// Selection filters require a project view; definition-view requests fail without changing state.
    pub(super) fn set_panel(&mut self, ctx: &Context, panel: Panel, open: bool) -> Result<(), String> {
        if panel == Panel::Filters && self.views.active.auxiliary() {
            return Err("Selection filters require a project view".into());
        }
        match panel {
            Panel::Browser => self.browser.open = open,
            Panel::Parts if open => {
                if !self.parts.open {
                    self.parts.focus_search();
                }
            }
            Panel::Parts => self.parts.close(ctx),
            Panel::Nets => self.net_inspector.open = open,
            Panel::Properties => {
                if !open {
                    self.views.active.panels[crate::workspace_panels::PROPERTIES].cancel_drag();
                }
                self.properties_open = open;
            }
            Panel::Filters => {
                if !open {
                    self.views.active.panels[crate::workspace_panels::FILTERS].cancel_drag();
                }
                self.views.active.filters_open = open;
            }
        }
        Ok(())
    }

    /// Whether the bound window shows `panel`; parked filter choices in definition views are hidden.
    pub(super) fn panel_open(&self, panel: Panel) -> bool {
        match panel {
            Panel::Browser => self.browser.open,
            Panel::Parts => self.parts.open,
            Panel::Nets => self.net_inspector.open,
            Panel::Properties => self.properties_open,
            Panel::Filters => !self.views.active.auxiliary() && self.views.active.filters_open,
        }
    }

    /// Navigate the addressed board's layer picker without changing visibility or document history.
    /// Next/previous wrap in the same catalog order as the picker, including technical and hidden
    /// layers; front/back resolve explicit copper side metadata. Unknown views/directions fail atomically.
    pub(super) fn navigate_board_layer(&mut self, id: crate::ViewId, direction: &str) -> Result<(), String> {
        use pcbcraft_engine::board::setup::{LayerPurpose, Side};
        let state = self.views.view(id).filter(|view| view.workspace == crate::Workspace::Board).ok_or("The addressed view must show a board")?;
        let board = self
            .session
            .project()
            .and_then(|project| state.board.target.and_then(|target| project.board_view(target)))
            .ok_or("The view's board is unavailable")?;
        let layers = crate::board_panels::ordered_layers(board);
        if layers.is_empty() {
            return Err("This board has no available layers".into());
        }
        let current = layers.iter().position(|layer| state.board.active_layer.as_deref() == Some(layer.id.to_string().as_str()));
        let chosen = match direction {
            "next" => layers.get(current.map_or(0, |i| (i + 1) % layers.len())),
            "previous" => layers.get(current.map_or(layers.len() - 1, |i| (i + layers.len() - 1) % layers.len())),
            "front" | "back" => layers.iter().find(|layer| {
                layer.purpose == LayerPurpose::Copper && layer.side == Some(if direction == "front" { Side::Front } else { Side::Back })
            }),
            _ => return Err("Layer direction must be next, previous, front or back".into()),
        }
        .ok_or("This board has no copper layer on that side")?
        .id
        .to_string();
        self.views.view_mut(id).ok_or("The addressed view closed")?.board.set_layer(&chosen, true, None);
        Ok(())
    }

    /// Fit the addressed view to the project on its next paint, clear of every visible card.
    /// Unknown views are ignored; the project and other views are unchanged.
    pub(super) fn fit_view(&mut self, id: crate::ViewId) {
        if let Some(view) = self.views.view_mut(id) {
            view.camera.request_fit();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod live {
    use egui::Context;
    use pcbcraft_chrome::{grid, units::Unit, workspace::Placement};
    use pcbcraft_engine::{
        schematic::{MAX_CENTER_NM, Point},
        selection::Selection,
    };
    use serde_json::{Map, Value, json};

    use super::Panel;
    use crate::{
        Dialog, PcbApp, ViewId, Workspace,
        arrangements::{Action, Start},
        canvas_camera::MAX_BOARD_ZOOM,
        editor_layout::HostId,
        editor_windows::ViewAction,
        preferences::{Preferences, ScrollAction, SelectionFollow, ThemeChoice, ZoomAround},
        schematic_canvas::Tool,
    };

    /// The kinds of value a window command parameter takes.
    #[derive(Clone, Copy, Debug)]
    enum Kind {
        /// A view ID from inspection.
        View,
        /// A typed engine authoring transaction.
        LibraryAction,
        /// A runtime document ID from inspect_editor.library_editors.
        LibraryDocument,
        /// A library document revision from inspect_editor.library_editors.
        LibraryRevision,
        /// A stable primitive ID from library inspection.
        LibraryPrimitive,
        /// A bounded group of distinct positive library primitive IDs.
        PrimitiveIds,
        /// A window ID from inspection.
        Window,
        /// An arrangement ID from inspection.
        Arrangement,
        /// An arrangement name: 1 to 80 characters on one line.
        Name,
        /// Library search text may be empty to clear a filter.
        Query,
        Bool,
        /// Opaque sRGB #RRGGBB, or null to restore a physical-role default.
        Color,
        Choice(&'static [&'static str]),
        Integer {
            min: i64,
            max: i64,
            step: i64,
        },
        /// A schematic point in nanometres within the coordinate bounds.
        Point,
        /// Symbol, wire and optional named-net anchor ID lists, as the project's group commands take.
        Selection,
        /// A list of distinct footprint IDs.
        Footprints,
        /// Persistent document/object UUID, resolved against the addressed view before acting.
        Object,
    }

    impl Kind {
        /// The JSON schema published to MCP clients.
        fn schema(self) -> Value {
            let coordinate = json!({"type": "integer", "minimum": -MAX_CENTER_NM, "maximum": MAX_CENTER_NM});
            let ids = json!({"type": "array", "uniqueItems": true, "items": {"type": "string"}});
            match self {
                Self::PrimitiveIds => json!({"type":"array","items":{"type":"integer","minimum":1},"maxItems":512,"uniqueItems":true}),
                Self::LibraryAction => {
                    json!({"type":"object","properties":{"operation":{"type":"string","enum":["insert","replace","replace_many","delete","transform","duplicate","paste","array","metadata","undo","redo"]}},"required":["operation"],"description":"A typed authoring Action: insert primitive; replace object/primitive; replace_many primitives map keyed by ID; delete objects; transform objects with transform {pivot,offset,quarter_turns,mirror_x,mirror_y}; duplicate objects/offset; paste primitives array/offset; array objects/columns/rows/column_step/row_step; metadata name/license/source/reference_prefix/default_value; undo/redo. Points are {x_nm,y_nm}; inspect_editor includes typed primitives."})
                }
                Self::View | Self::Window | Self::Arrangement | Self::LibraryDocument | Self::LibraryRevision | Self::LibraryPrimitive => {
                    json!({"type": "integer", "minimum": 0})
                }
                Self::Object => json!({"type":"string","format":"uuid"}),
                Self::Query => json!({"type":"string","maxLength":512}),
                Self::Name => json!({"type": "string", "minLength": 1, "maxLength": 80}),
                Self::Bool => json!({"type": "boolean"}),
                Self::Color => json!({"oneOf":[{"type":"string","pattern":"^#[0-9A-Fa-f]{6}$"},{"type":"null"}]}),
                Self::Choice(values) => json!({"type": "string", "enum": values}),
                Self::Integer { min, max, step } => json!({"type": "integer", "minimum": min, "maximum": max, "multipleOf": step}),
                Self::Point => {
                    json!({"type": "object", "properties": {"x_nm": coordinate, "y_nm": coordinate}, "required": ["x_nm", "y_nm"], "additionalProperties": false})
                }
                Self::Selection => {
                    let mut labels = ids.clone();
                    labels["maxItems"] = json!(pcbcraft_engine::net_labels::MAX_LABELS);
                    json!({"type": "object", "properties": {"symbols": ids, "wires": ids, "net_labels": labels, "no_connects": pcbcraft_engine::selection::Selection::marker_schema()}, "required": ["symbols", "wires"], "additionalProperties": false})
                }
                Self::Footprints => ids,
            }
        }

        /// Check a raw value without coercion, giving the reason it doesn't fit.
        fn validate(self, value: &Value) -> Result<(), String> {
            let fits = match self {
                Self::PrimitiveIds => value.as_array().is_some_and(|items| {
                    items.len() <= 512
                        && items.iter().all(|item| item.as_u64().is_some_and(|id| id > 0))
                        && items.iter().filter_map(Value::as_u64).collect::<std::collections::BTreeSet<_>>().len() == items.len()
                }),
                Self::LibraryAction => {
                    serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= pcbcraft_engine::authoring::MAX_BYTES)
                        && serde_json::from_value::<pcbcraft_engine::authoring::Action>(value.clone()).is_ok()
                }
                Self::View | Self::Window | Self::Arrangement | Self::LibraryDocument | Self::LibraryRevision | Self::LibraryPrimitive => {
                    value.as_u64().is_some()
                }
                Self::Query => value.as_str().is_some_and(|s| s.len() <= 512),
                Self::Name => value.as_str().is_some(),
                Self::Bool => value.is_boolean(),
                Self::Color => value.is_null() || value.as_str().is_some_and(|text| crate::board_appearance::parse_color(text).is_ok()),
                Self::Choice(values) => value.as_str().is_some_and(|value| values.contains(&value)),
                Self::Integer { min, max, step } => value.as_i64().is_some_and(|n| (min..=max).contains(&n) && n % step == 0),
                Self::Point => point(value).is_some(),
                Self::Object => value.as_str().is_some_and(|id| !id.is_empty() && id.len() <= 64),
                Self::Selection => Selection::parse(value).is_some(),
                Self::Footprints => footprints(value).is_some(),
            };
            if fits {
                return Ok(());
            }
            Err(match self {
                Self::PrimitiveIds => "must be a bounded list of distinct positive primitive IDs".into(),
                Self::LibraryAction => "must be a bounded native authoring Action".into(),
                Self::LibraryDocument => "must be a library document ID from inspect_editor.library_editors".into(),
                Self::LibraryRevision => "must be the library document revision from inspect_editor.library_editors".into(),
                Self::LibraryPrimitive => "must be a primitive ID from library inspection".into(),
                Self::View => "must be a view ID from inspect_editor".to_owned(),
                Self::Window => "must be a window ID from inspect_editor".to_owned(),
                Self::Arrangement => "must be an arrangement ID from inspect_editor".to_owned(),
                Self::Object => "must be an object UUID from project inspection".to_owned(),
                Self::Query => "must be library search text of at most 512 bytes".into(),
                Self::Name => "must be a name".to_owned(),
                Self::Bool => "must be true or false".to_owned(),
                Self::Color => "must be #RRGGBB or null to restore the default".to_owned(),
                Self::Choice(values) => format!("must be one of {}", values.join(", ")),
                Self::Integer { min, max, step: 1 } => format!("must be a whole number from {min} to {max}"),
                Self::Integer { min, max, step } => format!("must be a multiple of {step} from {min} to {max}"),
                Self::Point => "must be a point with whole-nanometre x_nm and y_nm within the schematic bounds".to_owned(),
                Self::Selection => "must contain unique symbol, wire and optional named-net anchor ID lists".to_owned(),
                Self::Footprints => "must be a list of distinct footprint IDs".to_owned(),
            })
        }
    }

    /// Read a bounded schematic point with exactly `x_nm` and `y_nm`.
    fn point(value: &Value) -> Option<Point> {
        let object = value.as_object().filter(|object| object.len() == 2)?;
        let coordinate = |key: &str| object.get(key)?.as_i64().filter(|n| (-MAX_CENTER_NM..=MAX_CENTER_NM).contains(n));
        Some(Point { x_nm: coordinate("x_nm")?, y_nm: coordinate("y_nm")? })
    }

    /// Read a list of distinct footprint IDs.
    fn footprints(value: &Value) -> Option<std::collections::BTreeSet<String>> {
        let items = value.as_array()?;
        let ids: std::collections::BTreeSet<String> = items.iter().map(|item| item.as_str().map(str::to_owned)).collect::<Option<_>>()?;
        (ids.len() == items.len()).then_some(ids)
    }

    /// One parameter of a window command.
    #[derive(Clone, Copy, Debug)]
    struct Param {
        name: &'static str,
        summary: &'static str,
        required: bool,
        kind: Kind,
    }

    /// A window command as discovery describes it.
    #[derive(Clone, Copy, Debug)]
    struct WindowCommand {
        id: &'static str,
        title: &'static str,
        summary: &'static str,
        params: &'static [Param],
    }

    impl WindowCommand {
        /// Describe the command like an engine command, marked as acting on the window.
        fn to_json(self) -> Value {
            let properties: Map<String, Value> = self
                .params
                .iter()
                .map(|param| {
                    let mut schema = param.kind.schema();
                    schema["description"] = json!(param.summary);
                    (param.name.to_owned(), schema)
                })
                .collect();
            let required: Vec<&str> = self.params.iter().filter(|param| param.required).map(|param| param.name).collect();
            json!({
                "id": self.id,
                "title": self.title,
                "summary": self.summary,
                "scope": "window",
                "params": {"type": "object", "properties": properties, "required": required, "additionalProperties": false},
            })
        }

        /// Reject unknown, missing and ill-typed parameters before anything changes.
        fn check(self, params: &Value) -> Result<(), String> {
            let empty = Map::new();
            let given = match params {
                Value::Null => &empty,
                Value::Object(object) => object,
                _ => return Err(format!("`{}` takes an object of parameters", self.id)),
            };
            if let Some(extra) = given.keys().find(|key| !self.params.iter().any(|param| param.name == key.as_str())) {
                return Err(format!("`{}` has no parameter `{extra}`", self.id));
            }
            for param in self.params {
                match given.get(param.name) {
                    Some(value) => param.kind.validate(value).map_err(|reason| format!("`{}` {reason}", param.name))?,
                    None if param.required => return Err(format!("`{}` needs `{}`", self.id, param.name)),
                    None => {}
                }
            }
            Ok(())
        }
    }

    const VIEW: Param = Param { name: "view", summary: "The view's ID, from inspect_editor", required: true, kind: Kind::View };
    const ARRANGEMENT: Param =
        Param { name: "arrangement", summary: "An arrangement ID from inspect_editor", required: true, kind: Kind::Arrangement };
    const WINDOW: Param = Param { name: "window", summary: "The window's ID, from inspect_editor", required: true, kind: Kind::Window };
    const THEMES: &[&str] = &["system", "dark", "light"];
    const UNITS: &[&str] = &["mil", "mm"];
    const FOLLOW: &[&str] = &["off", "both", "secondary_follows_primary", "primary_follows_secondary"];
    const SCROLL: &[&str] = &["zoom", "pan"];
    const ZOOM_AROUND: &[&str] = &["pointer", "center"];
    const PANELS: &[&str] = &["browser", "parts", "nets", "properties", "selection_filters"];
    const CATALOG_MODES: &[&str] = &["expanded", "compact"];
    const TABS: &[&str] = &["capture", "layout", "sheets", "analyze", "libraries", "manage", "route", "draw", "manufacture", "view"];
    const WORKSPACES: &[&str] = &["schematic", "board"];
    const PLACEMENTS: &[&str] = &["here", "beside", "window"];
    const BOARD_GRIDS: &[&str] = &["off", "dots", "lines"];

    /// Optional, so one call can change any subset; a call with none changes nothing.
    const fn setting(name: &'static str, summary: &'static str, kind: Kind) -> Param {
        Param { name, summary, required: false, kind }
    }

    const CATALOG: &[WindowCommand] = &[
        WindowCommand {
            id: "app.update_pcb",
            title: "Update PCB from Schematic",
            summary: "Open the reviewed schematic-to-PCB update dialog from an addressed project pane. Board creation and legacy adoption remain explicit actions; opening never applies changes.",
            params: &[VIEW],
        },
        WindowCommand {
            id: "app.set_workspace",
            title: "Choose Workspace",
            summary: "Choose a workspace in an addressed pane (default: focused main pane), here, beside it or in a native window. Definition sessions and project history remain independent.",
            params: &[
                Param {
                    name: "workspace",
                    summary: "Workspace to enter",
                    required: true,
                    kind: Kind::Choice(&["schematic", "board", "symbol_editor", "footprint_editor", "library"]),
                },
                Param { name: "view", summary: "Source pane; defaults to focused main pane", required: false, kind: Kind::View },
                Param { name: "placement", summary: "Where to open it", required: false, kind: Kind::Choice(PLACEMENTS) },
            ],
        },
        WindowCommand {
            id: "app.library_source",
            title: "Manage Library Source",
            summary: "Run Library workspace source management. Add/relocate use native pickers; Delete requires the same folder/count confirmation as the GUI. No project edits.",
            params: &[
                VIEW,
                Param {
                    name: "action",
                    summary: "Source operation",
                    required: true,
                    kind: Kind::Choice(&["add", "refresh", "relocate", "remove", "delete"]),
                },
                Param { name: "source", summary: "Registration ID, defaults to selected source", required: false, kind: Kind::Name },
            ],
        },
        WindowCommand {
            id: "app.library_browse",
            title: "Browse Library",
            summary: "Change a Library pane's source, Type, search or selected item without changing project revision/history. Inspect its selection through inspect_editor.",
            params: &[
                VIEW,
                Param { name: "source", summary: "Registration ID", required: false, kind: Kind::Name },
                Param { name: "query", summary: "Search text; empty clears it", required: false, kind: Kind::Query },
                Param {
                    name: "type",
                    summary: "0 all, 1 symbols, 2 footprints, 3 generic parts, 4 orderable parts",
                    required: false,
                    kind: Kind::Integer { min: 0, max: 4, step: 1 },
                },
                Param { name: "item", summary: "Item ID within selected source and Type", required: false, kind: Kind::Name },
            ],
        },
        WindowCommand {
            id: "app.library_new",
            title: "New Library Item",
            summary: "Create an independent symbol or footprint definition workspace without editing project instances.",
            params: &[Param { name: "kind", summary: "Definition kind", required: true, kind: Kind::Choice(&["symbol", "footprint"]) }],
        },
        WindowCommand {
            id: "app.library_fork_selected",
            title: "Edit Definition Copy",
            summary: "Fork the actual focused symbol or footprint selection under a new identity, leaving its source and project instances unchanged.",
            params: &[VIEW, Param { name: "kind", summary: "Definition kind", required: true, kind: Kind::Choice(&["symbol", "footprint"]) }],
        },
        WindowCommand {
            id: "app.library_edit",
            title: "Edit Library Item",
            summary: "Apply one revision-checked authoring transaction to a library document, using its separate undo history.",
            params: &[
                Param { name: "document", summary: "Runtime library document ID", required: true, kind: Kind::LibraryDocument },
                Param {
                    name: "expected_revision",
                    summary: "Library document revision from inspection",
                    required: true,
                    kind: Kind::LibraryRevision,
                },
                Param { name: "action", summary: "Typed authoring Action", required: true, kind: Kind::LibraryAction },
            ],
        },
        WindowCommand {
            id: "app.library_focus",
            title: "Focus Library Workspace",
            summary: "Focus an open library document; omit document to return to the project with library edits retained.",
            params: &[Param {
                name: "document",
                summary: "Runtime library document ID; omit for project",
                required: false,
                kind: Kind::LibraryDocument,
            }],
        },
        WindowCommand {
            id: "app.library_view",
            title: "Library View",
            summary: "Select a library primitive or definition metadata and navigate its canvas without changing authored data or history.",
            params: &[
                Param { name: "document", summary: "Runtime library document ID", required: true, kind: Kind::LibraryDocument },
                Param { name: "view", summary: "Definition pane ID; defaults to its current presentation", required: false, kind: Kind::View },
                Param { name: "selected", summary: "Stable primitive ID from library inspection", required: false, kind: Kind::LibraryPrimitive },
                Param {
                    name: "selected_objects",
                    summary: "Stable IDs for group selection; cannot combine with selected/definition",
                    required: false,
                    kind: Kind::PrimitiveIds,
                },
                Param { name: "definition", summary: "Select definition metadata instead of a primitive", required: false, kind: Kind::Bool },
                Param { name: "fit", summary: "Fit the definition", required: false, kind: Kind::Bool },
                Param { name: "ribbon_tab", summary: "Authoring ribbon tab", required: false, kind: Kind::Choice(&["draw", "library", "view"]) },
                Param { name: "browser", summary: "Whether the definition Browser is visible", required: false, kind: Kind::Bool },
                Param { name: "properties", summary: "Whether compact Properties is visible", required: false, kind: Kind::Bool },
                Param { name: "zoom_percent", summary: "Camera zoom percentage", required: false, kind: Kind::Integer { min: 1, max: 800, step: 1 } },
                Param { name: "center", summary: "Center in definition nanometres", required: false, kind: Kind::Point },
                Param {
                    name: "grid_spacing_nm",
                    summary: "Grid spacing offered by this editor, in nanometres",
                    required: false,
                    kind: Kind::Integer { min: 25_000, max: 1_270_000, step: 1 },
                },
            ],
        },
        WindowCommand {
            id: "app.library_close",
            title: "Close Library Item",
            summary: "Close an addressed library document. Unsaved fields or edits require an explicit discard; running file jobs prevent closing.",
            params: &[
                Param { name: "document", summary: "Runtime library document ID", required: true, kind: Kind::LibraryDocument },
                Param { name: "discard", summary: "Explicitly discard unsaved library edits", required: false, kind: Kind::Bool },
            ],
        },
        WindowCommand {
            id: "app.library_open",
            title: "Open Library Item",
            summary: "Open the library item picker in the editor; bounded input opens a new document and never replaces unsaved work.",
            params: &[],
        },
        WindowCommand {
            id: "app.library_save",
            title: "Save Library Copy",
            summary: "Choose a new destination for the captured library document. Export writes its native definition; browser downloads do not confirm durable saving.",
            params: &[
                Param { name: "document", summary: "Runtime library document ID", required: true, kind: Kind::LibraryDocument },
                Param { name: "export", summary: "Export native definition rather than editable item", required: false, kind: Kind::Bool },
            ],
        },
        WindowCommand {
            id: "view.open_sheet",
            title: "Open Sheet",
            summary: "Open an explicitly addressed sheet in this pane, as its Browser row does. Other panes retain their targets.",
            params: &[VIEW, Param { name: "sheet", summary: "Sheet UUID from project inspection", required: true, kind: Kind::Object }],
        },
        WindowCommand {
            id: "view.focus",
            title: "Focus View",
            summary: "Make the view its window's focused pane, as clicking its sheet tab does.",
            params: &[VIEW],
        },
        WindowCommand {
            id: "view.split",
            title: "Split",
            summary: "Split or unsplit the view's window, as View › Split does. The view becomes its window's focused pane; unsplitting keeps it.",
            params: &[VIEW, Param { name: "split", summary: "Whether the window shows several panes", required: true, kind: Kind::Bool }],
        },
        WindowCommand {
            id: "view.reorder",
            title: "Move Pane",
            summary: "Move a pane into another pane's position in the same window, shifting intervening panes and retaining dividers and cameras.",
            params: &[
                VIEW,
                Param { name: "target", summary: "View ID of the destination pane in the same window", required: true, kind: Kind::View },
            ],
        },
        WindowCommand {
            id: "view.open_beside",
            title: "Open Beside",
            summary: "Open another view of the same sheet beside this one.",
            params: &[VIEW],
        },
        WindowCommand { id: "view.close", title: "Close View", summary: "Close the view. The main window always keeps one view.", params: &[VIEW] },
        WindowCommand {
            id: "view.open_window",
            title: "New Window",
            summary: "Open another view of the same sheet in a new editor window.",
            params: &[VIEW],
        },
        WindowCommand {
            id: "view.detach",
            title: "Detach View",
            summary: "Move the view into its own editor window. Detaching the main window's only view leaves another view behind.",
            params: &[VIEW],
        },
        WindowCommand {
            id: "view.dock",
            title: "Dock View",
            summary: "Move a pane and its cached documents beside a pane in another window of this instance. Defaults to the main window's focused pane.",
            params: &[
                VIEW,
                Param { name: "beside", summary: "Destination view ID in another window of this instance", required: false, kind: Kind::View },
            ],
        },
        WindowCommand {
            id: "view.fit",
            title: "Fit",
            summary: "Fit the view to the whole schematic, clear of the visible panels, on its next paint.",
            params: &[VIEW],
        },
        WindowCommand {
            id: "view.center",
            title: "Center View",
            summary: "Pan so a schematic point is at the middle of the view's uncovered canvas, keeping the zoom.",
            params: &[VIEW, Param { name: "at", summary: "The point to center on", required: true, kind: Kind::Point }],
        },
        WindowCommand {
            id: "view.zoom",
            title: "Zoom",
            summary: "Zoom about the view center. The limit is 6400% in PCB layout and 800% in the schematic.",
            params: &[
                VIEW,
                Param {
                    name: "zoom_percent",
                    summary: "The zoom in percent",
                    required: true,
                    kind: Kind::Integer { min: 1, max: (MAX_BOARD_ZOOM * 100.0) as i64, step: 1 },
                },
            ],
        },
        WindowCommand {
            id: "view.set_grid",
            title: "Grid Spacing",
            summary: "Set the view's snap grid spacing, as the status bar's grid menu does. Later views start from it.",
            params: &[
                VIEW,
                Param {
                    name: "spacing_nm",
                    summary: "The spacing in nanometres (50 mil is 1270000)",
                    required: true,
                    kind: Kind::Integer { min: *grid::SPACING_RANGE_NM.start(), max: *grid::SPACING_RANGE_NM.end(), step: 1 },
                },
            ],
        },
        WindowCommand {
            id: "view.navigate_layer",
            title: "Navigate Layers",
            summary: "Choose the next/previous layer in picker order, or front/back copper, as the PCB keyboard shortcuts do. Wraps across technical/hidden layers without changing visibility or document history.",
            params: &[
                VIEW,
                Param {
                    name: "direction",
                    summary: "next, previous, front or back",
                    required: true,
                    kind: Kind::Choice(&["next", "previous", "front", "back"]),
                },
            ],
        },
        WindowCommand {
            id: "view.set_layer",
            title: "Layer",
            summary: "Activate or show/hide an addressed PCB layer through the Layers controls. Changes only the addressed view and cancels unfinished drawing when necessary.",
            params: &[
                VIEW,
                Param { name: "layer", summary: "Layer UUID on this view's board", required: true, kind: Kind::Object },
                Param { name: "active", summary: "Make this the active drawing layer", required: false, kind: Kind::Bool },
                Param { name: "visible", summary: "Show or hide this layer", required: false, kind: Kind::Bool },
            ],
        },
        WindowCommand {
            id: "view.set_ratsnest_visibility",
            title: "Ratsnest Visibility",
            summary: "Show or hide accepted logical connections in this PCB view, without changing project data or undo history.",
            params: &[VIEW, Param { name: "visible", summary: "Show accepted connections", required: true, kind: Kind::Bool }],
        },
        WindowCommand {
            id: "view.set_board_flipped",
            title: "Flip Board View",
            summary: "View the board from below or above in this PCB view, as Design › Flip view does. Keeps the centered point and zoom; changes no project data or undo history.",
            params: &[VIEW, Param { name: "flipped", summary: "Show the board from below", required: true, kind: Kind::Bool }],
        },
        WindowCommand {
            id: "view.set_layer_color",
            title: "Layer Color",
            summary: "Set or reset a board's personal layer color, through the same handler as its swatch. Shared by every view of that board, saved locally and excluded from project files/undo.",
            params: &[
                VIEW,
                Param { name: "layer", summary: "Layer UUID on the view's board", required: true, kind: Kind::Object },
                Param { name: "color", summary: "Opaque #RRGGBB or null to reset", required: true, kind: Kind::Color },
            ],
        },
        WindowCommand {
            id: "view.set_layer_visibility",
            title: "Layer Visibility",
            summary: "Show all layers or isolate the active layer, as the Layers card does. Changes only the addressed board view and cancels unfinished drawing.",
            params: &[
                VIEW,
                Param {
                    name: "mode",
                    summary: "all shows every layer; active reveals only the active layer",
                    required: true,
                    kind: Kind::Choice(&["all", "active"]),
                },
            ],
        },
        WindowCommand {
            id: "view.set_panel_position",
            title: "Panel Position",
            summary: "Float or dock Layers, Properties or Selection filters in this view, without editing the project.",
            params: &[
                VIEW,
                Param {
                    name: "panel",
                    summary: "Which contextual card",
                    required: true,
                    kind: Kind::Choice(&["layers", "properties", "selection_filters"]),
                },
                Param { name: "position", summary: "Edge anchor or floating", required: true, kind: Kind::Choice(&["left", "right", "floating"]) },
                setting("x", "Floating offset from the canvas left, in logical points", Kind::Integer { min: -100000, max: 100000, step: 1 }),
                setting("y", "Floating offset from the canvas top, in logical points", Kind::Integer { min: -100000, max: 100000, step: 1 }),
            ],
        },
        WindowCommand {
            id: "view.set_selection_filters",
            title: "Selection Filters",
            summary: "Change a project view's selection filters or collapse its filter card. Unsupported workspace categories are refused; document data and history stay unchanged.",
            params: &[
                VIEW,
                setting("symbols", "Select schematic symbols", Kind::Bool),
                setting("wires", "Select schematic wires", Kind::Bool),
                setting("sheets", "Select hierarchical sheet symbols", Kind::Bool),
                setting("labels", "Select hierarchical labels", Kind::Bool),
                setting("no_connects", "Select no-connect markers", Kind::Bool),
                setting("footprints", "Select board footprints", Kind::Bool),
                setting("tracks", "Select board tracks", Kind::Bool),
                setting("vias", "Select board vias", Kind::Bool),
                setting("graphics", "Select board graphics", Kind::Bool),
                setting("collapsed", "Collapse the filter card", Kind::Bool),
                setting("open", "Show the filter card", Kind::Bool),
            ],
        },
        WindowCommand {
            id: "view.select",
            title: "Select",
            summary: "Replace a schematic view's selection with these symbols, wires, named-net anchors and no-connect markers, as clicking or box selection does. Empty lists clear it. Other views highlight it under the Follow selection setting. PCB views take view.select_footprints.",
            params: &[VIEW, Param { name: "items", summary: "IDs from inspect_project", required: true, kind: Kind::Selection }],
        },
        WindowCommand {
            id: "view.select_footprints",
            title: "Select Footprints",
            summary: "Replace a PCB view's selection with these footprints, or select one footprint's reference text using element=reference. An empty footprint list clears selection. Other views highlight the counterpart symbols under the Follow selection setting.",
            params: &[
                VIEW,
                Param {
                    name: "footprints",
                    summary: "Footprint IDs on the view's board, from inspect_project",
                    required: true,
                    kind: Kind::Footprints,
                },
                Param {
                    name: "element",
                    summary: "Select footprint bodies (default) or one placement's reference text and Properties",
                    required: false,
                    kind: Kind::Choice(&["footprint", "reference"]),
                },
            ],
        },
        WindowCommand {
            id: "view.set_workspace",
            title: "Workspace",
            summary: "Show the schematic or the PCB layout, as the workspace picker does: here (a tab in this view's pane, reusing one that already shows it), in a new view beside it, or in a new window. Inspect the editor for the view that shows it.",
            params: &[
                VIEW,
                Param { name: "workspace", summary: "What to show", required: true, kind: Kind::Choice(WORKSPACES) },
                Param { name: "placement", summary: "Where; here when not given", required: false, kind: Kind::Choice(PLACEMENTS) },
            ],
        },
        WindowCommand {
            id: "window.set_panel",
            title: "Show Panel",
            summary: "Show or hide the Browser, Parts Catalog, Net Inspector, Properties or the focused view’s Selection filters card.",
            params: &[
                WINDOW,
                Param { name: "panel", summary: "Which panel", required: true, kind: Kind::Choice(PANELS) },
                Param { name: "open", summary: "Whether it shows", required: true, kind: Kind::Bool },
            ],
        },
        WindowCommand {
            id: "window.set_catalog_mode",
            title: "Catalog Presentation",
            summary: "Show the Parts Catalog as the expanded chooser or its compact dock, retaining its library, query and selection.",
            params: &[
                WINDOW,
                Param { name: "mode", summary: "Catalog presentation", required: true, kind: Kind::Choice(CATALOG_MODES) },
                Param { name: "repeat", summary: "Keep placing the selected part or power symbol until Escape", required: false, kind: Kind::Bool },
            ],
        },
        WindowCommand {
            id: "window.set_ribbon_tab",
            title: "Ribbon Tab",
            summary: "Show a schematic tab (Capture, Sheets, Analyze, Libraries, View) or PCB tab (Layout, Draw, Analyze, Manufacture, Libraries, View). Manage aliases Sheets; Route aliases Layout.",
            params: &[WINDOW, Param { name: "tab", summary: "Which tab", required: true, kind: Kind::Choice(TABS) }],
        },
        WindowCommand {
            id: "window.close",
            title: "Close Window",
            summary: "Close one editor window and its views while another stays open. The last window can only be closed in the editor.",
            params: &[WINDOW],
        },
        WindowCommand {
            id: "arrangement.select",
            title: "Show Arrangement",
            summary: "Switch to a named arrangement, as clicking its tab does: its saved views and windows replace the current ones, which the current arrangement keeps.",
            params: &[ARRANGEMENT],
        },
        WindowCommand {
            id: "arrangement.create",
            title: "New Arrangement",
            summary: "Add a named arrangement and switch to it, as the tabs' plus and Duplicate do: from the current views, from just the focused view, or as a copy of another arrangement. Up to 32.",
            params: &[
                Param { name: "name", summary: "Its name, unique ignoring case", required: true, kind: Kind::Name },
                Param { name: "single_view", summary: "Start with only the focused view", required: false, kind: Kind::Bool },
                Param { name: "copy_of", summary: "The arrangement to copy instead", required: false, kind: Kind::Arrangement },
            ],
        },
        WindowCommand {
            id: "arrangement.rename",
            title: "Rename Arrangement",
            summary: "Rename an arrangement, as its tab's Rename does. Views and windows are unchanged.",
            params: &[ARRANGEMENT, Param { name: "name", summary: "Its new name, unique ignoring case", required: true, kind: Kind::Name }],
        },
        WindowCommand {
            id: "arrangement.delete",
            title: "Delete Arrangement",
            summary: "Delete an arrangement, as its tab's Delete does; the last one stays. Deleting the shown one switches to the first that remains. This can't be undone.",
            params: &[ARRANGEMENT],
        },
        WindowCommand {
            id: "app.set_preferences",
            title: "Preferences",
            summary: "Change any of the person's settings, as the Preferences window does. Settings not given stay as they are; an invalid one changes none.",
            params: &[
                setting("theme", "The palette; system follows the desktop", Kind::Choice(THEMES)),
                setting("scale_percent", "The interface scale", Kind::Integer { min: 75, max: 200, step: 5 }),
                setting("units", "The unit lengths and coordinates are shown in", Kind::Choice(UNITS)),
                setting("grid_opacity_percent", "How strongly the grid shows; 0 hides it", Kind::Integer { min: 0, max: 100, step: 5 }),
                setting("schematic_grid_visible", "Show the schematic grid; true restores zero opacity", Kind::Bool),
                setting("board_grid_visible", "Show the PCB grid; true overrides Off and zero opacity", Kind::Bool),
                setting("board_grid_style", "How PCB views draw their grid", Kind::Choice(BOARD_GRIDS)),
                setting("selection_follow", "Which views pan to reveal another view's selection", Kind::Choice(FOLLOW)),
                setting("scroll", "What the mouse wheel does on the canvas", Kind::Choice(SCROLL)),
                setting("reverse_zoom", "Reverse the wheel's zoom direction", Kind::Bool),
                setting("zoom_speed_percent", "Wheel zoom speed", Kind::Integer { min: 50, max: 200, step: 10 }),
                setting("zoom_around", "The point that stays still while zooming", Kind::Choice(ZOOM_AROUND)),
            ],
        },
        WindowCommand {
            id: "app.set_home",
            title: "Home",
            summary: "Show the Home page in place of the workspace, or return to the workspace. The project stays open.",
            params: &[Param { name: "shown", summary: "Whether Home shows", required: true, kind: Kind::Bool }],
        },
        WindowCommand {
            id: "file.save",
            title: "Save",
            summary: "Save the project to the location it was opened from or last saved to, as File › Save does. Inspect the editor for when the save finishes. An Untitled project must first be saved in the editor.",
            params: &[],
        },
    ];

    /// Every window command, described for live discovery.
    pub(crate) fn window_catalog() -> Vec<Value> {
        CATALOG.iter().map(|command| command.to_json()).collect()
    }

    /// Show a theme the way window commands name it.
    fn theme_name(theme: ThemeChoice) -> &'static str {
        match theme {
            ThemeChoice::System => "system",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
        }
    }

    /// Show the selection-follow policy the way window commands name it.
    fn follow_name(follow: SelectionFollow) -> &'static str {
        match follow {
            SelectionFollow::Off => "off",
            SelectionFollow::Both => "both",
            SelectionFollow::SecondaryFollowsPrimary => "secondary_follows_primary",
            SelectionFollow::PrimaryFollowsSecondary => "primary_follows_secondary",
        }
    }

    /// Report settings with the names and values `app.set_preferences` takes.
    fn inspect_preferences(preferences: &Preferences) -> Value {
        json!({
            "theme": theme_name(preferences.theme),
            "scale_percent": preferences.scale_percent,
            "units": preferences.units.symbol(),
            "grid_opacity_percent": preferences.grid_opacity_percent,
            "schematic_grid_visible": preferences.schematic_grid_visible,
            "board_grid_visible": preferences.board_grid_visible,
            "board_grid_style": match preferences.board_grid_style {
                grid::Style::Off => "off",
                grid::Style::Dots => "dots",
                grid::Style::Lines => "lines",
            },
            "selection_follow": follow_name(preferences.selection_follow),
            "scroll": match preferences.navigation.scroll { ScrollAction::Zoom => "zoom", ScrollAction::Pan => "pan" },
            "reverse_zoom": preferences.navigation.reverse_zoom,
            "zoom_speed_percent": preferences.navigation.zoom_speed_percent,
            "zoom_around": match preferences.navigation.zoom_around { ZoomAround::Pointer => "pointer", ZoomAround::Center => "center" },
        })
    }

    /// Apply validated settings, then explicit visibility requests. Visibility wins over an
    /// accompanying Off style or zero opacity, restoring dots/default opacity when needed.
    /// Without a visibility request, explicit style and opacity remain unchanged.
    fn updated_preferences(mut preferences: Preferences, params: &Value) -> Preferences {
        let text = |key: &str| params.get(key).and_then(Value::as_str);
        let number = |key: &str| params.get(key).and_then(Value::as_u64);
        if let Some(theme) = text("theme") {
            preferences.theme = match theme {
                "system" => ThemeChoice::System,
                "light" => ThemeChoice::Light,
                _ => ThemeChoice::Dark,
            };
        }
        if let Some(scale) = number("scale_percent").and_then(|n| u16::try_from(n).ok()) {
            preferences.scale_percent = scale;
        }
        if let Some(units) = text("units") {
            preferences.units = if units == "mm" { Unit::Mm } else { Unit::Mil };
        }
        if let Some(opacity) = number("grid_opacity_percent").and_then(|n| u8::try_from(n).ok()) {
            preferences.grid_opacity_percent = opacity;
        }
        if let Some(style) = text("board_grid_style") {
            preferences.board_grid_style = match style {
                "dots" => grid::Style::Dots,
                "lines" => grid::Style::Lines,
                _ => grid::Style::Off,
            };
        }
        for (key, board) in [("board_grid_visible", true), ("schematic_grid_visible", false)] {
            if let Some(visible) = params.get(key).and_then(Value::as_bool) {
                preferences.set_grid_visible(board, visible);
            }
        }
        if let Some(follow) = text("selection_follow") {
            preferences.selection_follow = match follow {
                "both" => SelectionFollow::Both,
                "secondary_follows_primary" => SelectionFollow::SecondaryFollowsPrimary,
                "primary_follows_secondary" => SelectionFollow::PrimaryFollowsSecondary,
                _ => SelectionFollow::Off,
            };
        }
        if let Some(scroll) = text("scroll") {
            preferences.navigation.scroll = if scroll == "pan" { ScrollAction::Pan } else { ScrollAction::Zoom };
        }
        if let Some(reverse) = params.get("reverse_zoom").and_then(Value::as_bool) {
            preferences.navigation.reverse_zoom = reverse;
        }
        if let Some(speed) = number("zoom_speed_percent").and_then(|n| u16::try_from(n).ok()) {
            preferences.navigation.zoom_speed_percent = speed;
        }
        if let Some(around) = text("zoom_around") {
            preferences.navigation.zoom_around = if around == "center" { ZoomAround::Center } else { ZoomAround::Pointer };
        }
        preferences
    }

    /// Name the view's unfinished gesture, if any.
    fn tool_name(tool: &Tool) -> &'static str {
        match tool {
            Tool::NoConnect { .. } => "no_connect",
            Tool::Field { .. } => "move_symbol_field",
            Tool::NetLabel { .. } => "net_label",
            Tool::SheetGesture { .. } => "sheet_geometry",
            Tool::Hierarchy { .. } => "hierarchy",
            Tool::PlaceSymbol { .. } => "place_symbol",
            Tool::Wire { .. } => "wire",
            Tool::BoxSelect { .. } => "box_select",
            Tool::MoveItems { .. } => "move_items",
            Tool::Move { .. } => "move_symbol",
            Tool::Reshape { .. } => "reshape_wire",
        }
    }

    /// The editor's windows, visible views, panels and settings, with the IDs window commands
    /// take. Parked views of a collapsed split are not listed; they return when it reopens.
    /// Report inline text editing, dialogs and file operations that can hold live commands.
    pub(crate) fn inspect_editor(app: &PcbApp) -> Value {
        let windows: Vec<Value> = app
            .views
            .hosts
            .iter()
            .map(|(id, host)| {
                let panels = app.host_panels(*id);
                json!({
                    "id": id.0,
                    "main": *id == app.views.main,
                    "views": host.layout.views().iter().map(|view| view.0).collect::<Vec<_>>(),
                    "focused_view": host.focused.0,
                    "split": host.split(),
                    "panels": {"browser": panels.browser, "parts": panels.parts, "nets": panels.nets, "properties":panels.properties,
                        "selection_filters": app.views.view(host.focused).is_some_and(|view| !view.auxiliary() && view.filters_open)},
                    "catalog_mode": panels.catalog_mode,
                    "repeat_placement": panels.repeat_placement,
                    "ribbon_tab": app.inspect_library_view(host.focused).map(|v|v["ribbon_tab"].clone()).unwrap_or_else(|| json!(match panels.ribbon {
                        0 if app.views.view(host.focused).is_some_and(|view| view.workspace == Workspace::Board) => "layout",
                        0 => "capture",
                        slot => crate::ribbon::tab_name(slot),
                    })),
                })
            })
            .collect();
        let views: Vec<Value> = app
            .views
            .hosts
            .iter()
            .flat_map(|(id, host)| host.layout.views().into_iter().map(move |view| (*id, host.focused == view, view)))
            .filter_map(|(window, focused, id)| {
                let view = app.views.view(id)?;
                Some(json!({
                    "id": id.0,
                    "window": window.0,
                    "focused": focused,
                    "primary": id == app.views.primary(),
                    "workspace": if view.library.is_some() {"library"} else {view.definition.and_then(|id|app.library_kind(id)).map(|kind|if kind==pcbcraft_engine::authoring::Kind::Symbol {"symbol_editor"} else {"footprint_editor"}).unwrap_or(match view.workspace {Workspace::Schematic=>"schematic",Workspace::Board=>"board"})},
                    "definition":view.definition,
                    "library_view":app.inspect_library_view(id),
                    "library_workspace":view.library.as_ref().map(|state|state.inspect()),
                    "sheet": view.sheet_path(&app.session),
                    "sheet_id": match view.target() {Some(crate::editor_view::DocumentTarget::Sheet(sheet))=>Some(sheet.id()),_=>None},
                    "grid_spacing_nm": view.camera.snap_nm(),
                    "selection_filters": view.filters.inspect(view.workspace),
                    "selection_filters_collapsed": view.filters_collapsed,
                    "selection_filters_open": !view.auxiliary() && view.filters_open,
                    "panel_positions": crate::workspace_panels::inspect(&view.panels),
                    "zoom_percent": (view.camera.zoom() * 100.0).round(),
                    "center": view.camera.center(),
                    "selection": view.schematic.selection,
                    "selected_segment": view.schematic.selected_leg.as_ref().map(|leg| json!({"wire": leg.wire, "from": leg.route.get(leg.leg), "to": leg.route.get(leg.leg + 1)})),
                    "tool": view.schematic.tool.as_ref().map(tool_name),
                    "board": (view.workspace == Workspace::Board).then(|| json!({
                        "selection": view.board.selection,
                        "reference_selection": view.board.reference.selected,
                        "hidden_layers": view.board.hidden_layers,
                        "ratsnest_visible": view.board.target.and_then(|target| app.session.project().and_then(|project| project.board_view(target)))
                            .filter(|board| board.has_accepted_connectivity()).map(|_| !view.board.hidden_layers.contains(crate::editor_view::RATSNEST_KEY)),
                        "active_layer": view.board.active_layer,
                        "flipped": view.camera.mirrored(),
                        "layer_colors": view.board.target.and_then(|target|app.session.project().and_then(|project|project.board_view(target)))
                            .map(|board|board.layers().iter().map(|layer|json!({"layer":layer.id,"color":crate::board_appearance::hex(app.board_appearance.color(board,layer)),"overridden":app.board_appearance.overridden(board.target,&layer.id.to_string())})).collect::<Vec<_>>()),
                        "copper_colors": view.board.target.and_then(|target|app.session.project().and_then(|project|project.board_view(target)))
                            .map(|board|board.layers().iter().filter(|layer|layer.purpose==pcbcraft_engine::board::setup::LayerPurpose::Copper)
                                .map(|layer|json!({"layer":layer.id,"color":crate::board_appearance::hex(app.board_appearance.color(board,layer)),"overridden":app.board_appearance.overridden(board.target,&layer.id.to_string())})).collect::<Vec<_>>()),
                        "drawing_selection": view.board.drawing.selected,
                        "drawing_tool": view.board.drawing.tool.map(|tool| match tool {crate::board_drawing::Tool::Lines=>"lines",crate::board_drawing::Tool::Rectangle=>"rectangle"}),
                    })),
                }))
            })
            .collect();
        json!({
            "revision": app.session.revision(),
            "unsaved": app.files.dirty(&app.session),
            "file_operation": app.files.busy(),
            "dialog_open": !matches!(app.dialog, Dialog::None),
            "text_editing": app.views.editing_text(),
            "home": app.home,
            "workspace": app.workspace_key(),
            "message": app.message,
            "windows": windows,
            "library_editors":app.library_editors.inspect(),
            "views": views,
            "arrangements": {
                "active": app.arrangements.active(),
                "tabs": app.arrangements.tabs().into_iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
            },
            "preferences": inspect_preferences(&app.preferences),
        })
    }

    impl PcbApp {
        /// The window that shows `id`, or a reason naming the stale ID.
        fn view_window(&self, id: ViewId) -> Result<HostId, String> {
            self.views.owner(id).ok_or_else(|| format!("There is no open view {}. Inspect the editor for the current view IDs.", id.0))
        }

        /// Check an arrangement command as the tabs and their forms do, so a refusal names its
        /// reason; the accepted action is applied after the windows draw, like a tab click.
        fn arrangement_action(&self, id: &str, params: &Value) -> Result<Action, String> {
            if let Some(error) = self.arrangement_unavailable() {
                return Err(error.into());
            }
            if self.home {
                return Err("Home is showing. Return to the workspace first (app.set_home).".to_owned());
            }
            let tabs = self.arrangements.tabs();
            if tabs.is_empty() {
                return Err("The project's arrangements aren't ready yet. Try again.".to_owned());
            }
            let known = |key: &str| -> Result<Option<u64>, String> {
                match params.get(key).and_then(Value::as_u64) {
                    Some(arrangement) if !tabs.iter().any(|(tab, _)| *tab == arrangement) => {
                        Err(format!("There is no arrangement {arrangement}. Inspect the editor for the current arrangement IDs."))
                    }
                    found => Ok(found),
                }
            };
            let target = known("arrangement")?;
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let named = |except| self.arrangements.name_error(name, except).map_or(Ok(()), |error| Err(error.to_owned()));
            match (id, target) {
                ("arrangement.select", Some(arrangement)) => Ok(Action::Select(arrangement)),
                ("arrangement.rename", Some(arrangement)) => {
                    named(Some(arrangement))?;
                    Ok(Action::Rename { id: arrangement, name: name.trim().to_owned() })
                }
                ("arrangement.delete", Some(_)) if tabs.len() <= 1 => Err("This is the only arrangement, which stays.".to_owned()),
                ("arrangement.delete", Some(arrangement)) => Ok(Action::Remove(arrangement)),
                ("arrangement.create", _) => {
                    named(None)?;
                    if tabs.len() >= 32 {
                        return Err("An arrangement collection can contain up to 32 arrangements.".to_owned());
                    }
                    let single = params.get("single_view").and_then(Value::as_bool) == Some(true);
                    let start = match known("copy_of")? {
                        Some(_) if single => return Err("Give single_view or copy_of, not both.".to_owned()),
                        Some(source) => Start::Copy(source),
                        None if single => Start::SingleView,
                        None => Start::Current,
                    };
                    Ok(Action::Create { name: name.trim().to_owned(), start })
                }
                _ => Err(format!("unknown command `{id}`")),
            }
        }

        /// Run `act` with `host`'s panels and focused view bound, then rebind the main window,
        /// as drawing a detached window does.
        fn in_window(&mut self, host: HostId, act: impl FnOnce(&mut Self)) {
            self.bind_editor_host(host);
            act(self);
            self.bind_editor_host(self.views.main);
        }

        /// Validate and run one window command through the handler its control uses. Call it on
        /// the editor thread with the main window bound and no dialog or file operation pending;
        /// the live connection's refusal enforces that. A refused command changes nothing.
        pub(crate) fn run_window_command(&mut self, ctx: &Context, id: &str, params: &Value) -> Result<(), String> {
            let command = CATALOG.iter().find(|command| command.id == id).ok_or_else(|| format!("unknown command `{id}`"))?;
            command.check(params)?;
            if id == "app.set_workspace" {
                let source = ViewId(params["view"].as_u64().unwrap_or_else(|| self.views.hosts[&self.views.main].focused.0));
                let host = self.views.owner(source).ok_or("Unknown source view ID.")?;
                self.bind_editor_host(host);
                self.activate_editor(ctx, source);
                let placement = match params["placement"].as_str() {
                    Some("beside") => pcbcraft_chrome::workspace::Placement::Beside,
                    Some("window") => pcbcraft_chrome::workspace::Placement::Window,
                    _ => pcbcraft_chrome::workspace::Placement::Here,
                };
                let result = self.enter_workspace(ctx, params["workspace"].as_str().unwrap_or_default(), placement);
                self.bind_editor_host(self.views.main);
                return result;
            }
            if matches!(id, "app.library_source" | "app.library_browse") {
                return self.library_workspace_command(ctx, id, params);
            }
            if id.starts_with("app.library_") {
                return self.library_command(ctx, id, params);
            }
            if (id.starts_with("arrangement.")
                || id == "window.set_panel"
                || id == "window.set_catalog_mode"
                || (id.starts_with("view.")
                    && !matches!(
                        id,
                        "view.focus"
                            | "view.split"
                            | "view.reorder"
                            | "view.open_beside"
                            | "view.open_window"
                            | "view.detach"
                            | "view.dock"
                            | "view.close"
                            | "view.fit"
                            | "view.set_panel_position"
                    )))
                && self.session.project().is_none()
            {
                return Err("No project is open. Open or create one first.".to_owned());
            }
            self.bind_editor_host(self.views.main);
            let view = params.get("view").and_then(Value::as_u64).map(ViewId);
            let window = params.get("window").and_then(Value::as_u64).map(HostId);
            let window = match window {
                Some(window) if !self.views.hosts.contains_key(&window) => {
                    return Err(format!("There is no open window {}. Inspect the editor for the current window IDs.", window.0));
                }
                window => window,
            };
            let view = match view {
                Some(view) => Some((view, self.view_window(view)?)),
                None => None,
            };
            if let Some((view, _)) = view {
                if !self.views.view(view).is_some_and(|v| v.auxiliary()) && self.session.project().is_none() {
                    return Err("No project is open.".into());
                }
                if self.views.view(view).is_some_and(|v| v.auxiliary())
                    && !matches!(
                        id,
                        "view.focus"
                            | "view.split"
                            | "view.reorder"
                            | "view.open_beside"
                            | "view.open_window"
                            | "view.detach"
                            | "view.dock"
                            | "view.close"
                            | "view.fit"
                            | "view.set_panel_position"
                            | "view.set_workspace"
                    )
                {
                    return Err("Use app.library_view for definition selection and navigation.".into());
                }
            }
            match (id, view, window) {
                ("app.update_pcb", Some((view, _)), _) => self.begin_board_update(view)?,
                ("view.focus", Some((view, host)), _) => self.in_window(host, |app| app.activate_editor(ctx, view)),
                ("view.split", Some((view, host)), _) => {
                    let split = params["split"].as_bool() == Some(true);
                    self.in_window(host, |app| {
                        app.activate_editor(ctx, view);
                        app.split_editors(ctx, split);
                    });
                }
                ("view.reorder", Some((view, host)), _) => {
                    let target = ViewId(params["target"].as_u64().unwrap_or_default());
                    if self.view_window(target)? != host
                        || self.views.hosts.get(&host).and_then(|host| host.layout.group(view)).is_some_and(|group| group.views().contains(&target))
                    {
                        return Err("Choose another pane in the same window.".into());
                    }
                    let target =
                        self.views.view(target).and_then(|view| view.control_input(&self.session)).ok_or("The destination pane is unavailable.")?;
                    self.apply_view_action(ctx, view, ViewAction::Reorder(target));
                }
                ("view.close", Some((view, host)), _) if host == self.views.main && !self.views.hosts.get(&host).is_some_and(|host| host.split()) => {
                    return Err(format!("View {} is the main window's only view, which stays open. Close another view or window instead.", view.0));
                }
                ("view.dock", Some((view, host)), _) if params.get("beside").is_some() => {
                    let target = ViewId(params["beside"].as_u64().unwrap_or_default());
                    if self.view_window(target)? == host {
                        return Err("Docking requires a destination pane in another window of this instance.".into());
                    }
                    let target =
                        self.views.view(target).and_then(|view| view.control_input(&self.session)).ok_or("The destination pane is unavailable.")?;
                    self.apply_view_action(ctx, view, ViewAction::MoveBeside(target));
                }
                ("view.dock", Some((view, host)), _) if host == self.views.main => {
                    return Err(format!("View {} is already in the main window.", view.0));
                }
                ("view.open_beside" | "view.close" | "view.open_window" | "view.detach" | "view.dock", Some((view, _)), _) => {
                    let action = match id {
                        "view.open_beside" => ViewAction::OpenBeside,
                        "view.close" => ViewAction::Close,
                        "view.open_window" => ViewAction::OpenWindow,
                        "view.detach" => ViewAction::Detach,
                        _ => ViewAction::Dock,
                    };
                    self.apply_view_action(ctx, view, action);
                }
                ("view.fit", Some((view, _)), _) => {
                    if let Some(document) = self.bind_library_view(view) {
                        self.fit_library_document(document);
                    } else {
                        self.fit_view(view);
                    }
                }
                ("view.center", Some((view, _)), _) => {
                    let at = point(&params["at"]).ok_or("`at` must be a point")?;
                    if let Some(view) = self.views.view_mut(view) {
                        view.camera.center_on(at);
                    }
                }
                ("view.zoom", Some((view, _)), _) => {
                    let percent = params["zoom_percent"].as_u64().unwrap_or(100);
                    if let Some(view) = self.views.view_mut(view) {
                        let max_zoom = view.workspace.max_zoom();
                        let max_percent = (max_zoom * 100.0) as u64;
                        if percent > max_percent {
                            return Err(format!("`zoom_percent` must be at most {max_percent} for this workspace."));
                        }
                        view.camera.set_zoom_with_limit(percent as f32 / 100.0, max_zoom);
                    }
                }
                ("view.navigate_layer", Some((view, _)), _) => self.navigate_board_layer(view, params["direction"].as_str().unwrap_or_default())?,
                ("view.set_ratsnest_visibility", Some((view, _)), _) => {
                    let state =
                        self.views.view(view).filter(|view| view.workspace == Workspace::Board).ok_or("The addressed view must show a board")?;
                    let board = state
                        .board
                        .target
                        .and_then(|target| self.session.project().and_then(|project| project.board_view(target)))
                        .ok_or("The view's board is unavailable")?;
                    if !board.has_accepted_connectivity() {
                        return Err("This board has no accepted connectivity".into());
                    }
                    self.views
                        .view_mut(view)
                        .ok_or("The addressed view closed")?
                        .board
                        .set_ratsnest_visibility(params["visible"].as_bool() == Some(true));
                }
                ("view.set_board_flipped", Some((view, _)), _) => {
                    let state =
                        self.views.view_mut(view).filter(|view| view.workspace == Workspace::Board).ok_or("The addressed view must show a board")?;
                    state.camera.set_mirrored(params["flipped"].as_bool() == Some(true));
                }
                ("view.set_layer_color", Some((view, _)), _) => {
                    let rgb = params["color"].as_str().map(crate::board_appearance::parse_color).transpose()?;
                    self.set_layer_color(view, params["layer"].as_str().unwrap_or_default(), rgb)?;
                }
                ("view.set_layer_visibility", Some((view, _)), _) => {
                    let state =
                        self.views.view(view).filter(|view| view.workspace == Workspace::Board).ok_or("The addressed view must show a board")?;
                    let target = state.board.target.ok_or("No board is bound to this view")?;
                    let board = self.session.project().and_then(|project| project.board_view(target)).ok_or("The view's board is unavailable")?;
                    let active_only = params["mode"].as_str() == Some("active");
                    if active_only && !board.layers().iter().any(|layer| state.board.active_layer.as_deref() == Some(layer.id.to_string().as_str())) {
                        return Err("Select an active layer first".into());
                    }
                    let layers = board.layers().iter().map(|layer| layer.id.to_string()).collect::<Vec<_>>();
                    self.views.view_mut(view).ok_or("The addressed view closed")?.board.set_layer_visibility(layers, active_only);
                }
                ("view.set_layer", Some((view, _)), _) => {
                    let target = self
                        .views
                        .view(view)
                        .filter(|view| view.workspace == Workspace::Board)
                        .and_then(|view| view.board.target)
                        .ok_or("The addressed view must show a board")?;
                    let layer = params["layer"].as_str().unwrap_or_default();
                    if !self
                        .session
                        .project()
                        .and_then(|project| project.board_view(target))
                        .is_some_and(|board| board.layers().iter().any(|record| record.id.to_string() == layer))
                    {
                        return Err("Unknown layer on this view's board".into());
                    }
                    self.views.view_mut(view).ok_or("The addressed view closed")?.board.set_layer(
                        layer,
                        params["active"].as_bool() == Some(true),
                        params["visible"].as_bool(),
                    );
                }
                ("view.set_panel_position", Some((view, _)), _) => self.set_panel_position(view, params)?,
                ("view.set_selection_filters", Some((view, _)), _) => self.set_selection_filters(view, params)?,
                ("view.set_grid", Some((view, _)), _) => self.set_view_grid(view, params["spacing_nm"].as_i64().unwrap_or_default()),
                ("view.select" | "view.select_footprints", Some((view, _)), _)
                    if self.views.view(view).map(|view| view.workspace)
                        != Some(if id == "view.select" { Workspace::Schematic } else { Workspace::Board }) =>
                {
                    return Err(if id == "view.select" {
                        format!("View {} shows the PCB layout. Use view.select_footprints, or view.set_workspace to show its schematic.", view.0)
                    } else {
                        format!("View {} shows the schematic. Use view.select, or view.set_workspace to show the PCB layout.", view.0)
                    });
                }
                ("view.select_footprints", Some((view, _)), _) => {
                    let wanted = footprints(&params["footprints"]).ok_or("`footprints` must be a list of distinct footprint IDs")?;
                    let reference = params["element"].as_str() == Some("reference");
                    if reference && wanted.len() != 1 {
                        return Err("Reference selection requires exactly one footprint ID".into());
                    }
                    let placed: std::collections::BTreeSet<String> = self
                        .views
                        .view(view)
                        .and_then(|view| view.board.target)
                        .zip(self.session.project())
                        .and_then(|(target, project)| project.board_view(target))
                        .into_iter()
                        .flat_map(|board| board.footprints())
                        .map(|footprint| footprint.id)
                        .collect();
                    if !wanted.is_subset(&placed) {
                        return Err("Some of those footprints aren't on this view's board. Inspect the project for current IDs.".to_owned());
                    }
                    if !wanted.is_empty() && self.views.view(view).is_some_and(|v| !v.filters.footprints) {
                        return Err("Footprint selection is disabled in this view".into());
                    }
                    let previous = self.views.active.id();
                    self.views.bind(view);
                    self.views.active.cancel_gesture();
                    if reference {
                        if let Some(footprint) = wanted.into_iter().next() {
                            self.views.select_board_reference(footprint, &self.session, self.preferences.selection_follow);
                        }
                    } else {
                        self.views.select_board(wanted, &self.session, self.preferences.selection_follow);
                    }
                    self.views.bind(previous);
                }
                ("view.open_sheet", Some((view, _)), _) => {
                    let project = self.session.project().ok_or("no project is open")?;
                    let sheet = project.resolve_sheet(params["sheet"].as_str()).map_err(|e| e.to_string())?;
                    self.apply_browser_action(view, crate::project_browser::Action::OpenDocument(crate::editor_view::DocumentTarget::Sheet(sheet)));
                }
                ("view.set_workspace", Some((view, _)), _) => {
                    let workspace = if params["workspace"].as_str() == Some("board") { Workspace::Board } else { Workspace::Schematic };
                    let placement = match params["placement"].as_str() {
                        Some("beside") => Placement::Beside,
                        Some("window") => Placement::Window,
                        _ => Placement::Here,
                    };
                    self.apply_view_action(ctx, view, ViewAction::Workspace { workspace, placement });
                }
                ("view.select", Some((view, _)), _) => {
                    let items = Selection::parse(&params["items"])
                        .ok_or("`items` must contain symbol/wire lists and optional named-net anchors or no-connect targets")?;
                    let mut filtered = items.clone();
                    if let Some(state) = self.views.view(view) {
                        state.filters.retain(&mut filtered, state.snapshot(&self.session).as_deref());
                    }
                    if filtered != items {
                        return Err("Some requested object types are disabled by this view's selection filters".into());
                    }
                    let mut present = items.clone();
                    if let Some(project) = self.views.view(view).and_then(|view| view.snapshot(&self.session)) {
                        present.retain(&project);
                    }
                    if present != items {
                        return Err("Some of those items aren't on this view's sheet. Inspect the project for current IDs.".to_owned());
                    }
                    let previous = self.views.active.id();
                    self.views.bind(view);
                    self.views.active.cancel_gesture();
                    self.select(items);
                    self.views.bind(previous);
                }
                (arrangement, _, _) if arrangement.starts_with("arrangement.") => {
                    let action = self.arrangement_action(id, params)?;
                    self.queue_arrangement(ctx, action);
                }
                ("window.set_panel", _, Some(host)) => {
                    let panel = match params["panel"].as_str() {
                        Some("browser") => Panel::Browser,
                        Some("parts") => Panel::Parts,
                        Some("properties") => Panel::Properties,
                        Some("selection_filters") => Panel::Filters,
                        _ => Panel::Nets,
                    };
                    let open = params["open"].as_bool() == Some(true);
                    let mut result = Ok(());
                    self.in_window(host, |app| result = app.set_panel(ctx, panel, open));
                    result?;
                }
                ("window.set_catalog_mode", _, Some(host)) => {
                    if !self
                        .views
                        .hosts
                        .get(&host)
                        .and_then(|host| self.views.view(host.focused))
                        .is_some_and(|view| view.definition.is_none() && view.workspace == Workspace::Schematic)
                    {
                        return Err("The Parts Catalog is available in a schematic view.".into());
                    }
                    self.in_window(host, |app| {
                        app.parts.set_compact(ctx, params["mode"] == "compact");
                        if let Some(repeat) = params["repeat"].as_bool() {
                            app.parts.set_repeat(repeat);
                        }
                    });
                }
                ("window.set_ribbon_tab", _, Some(host)) => {
                    if self.views.hosts.get(&host).and_then(|host| self.views.view(host.focused)).is_some_and(|view| view.definition.is_some()) {
                        return Err("Use app.library_view to select the authoring ribbon tab.".into());
                    }
                    let name = params["tab"].as_str().unwrap_or_default();
                    let tab = crate::ribbon::tab_slot(name);
                    let board = self
                        .views
                        .hosts
                        .get(&host)
                        .and_then(|host| self.views.view(host.focused))
                        .is_some_and(|view| view.workspace == Workspace::Board);
                    let allowed: &[usize] = if board { &crate::ribbon::BOARD_TAB_ORDER } else { &crate::ribbon::TAB_ORDER };
                    if !allowed.contains(&tab) || (board && name == "capture") || (!board && matches!(name, "layout" | "route")) {
                        return Err(format!("This ribbon tab is available in the {} workspace.", if board { "schematic" } else { "PCB" }));
                    }
                    self.in_window(host, |app| app.ribbon_tab = tab);
                }
                ("window.close", _, Some(host)) => {
                    if self.views.hosts.len() == 1 {
                        return Err("This is the last editor window. Close it in the editor, which asks about unsaved changes.".to_owned());
                    }
                    self.close_editor_host(host, ctx);
                }
                ("app.set_preferences", _, _) => self.preferences = updated_preferences(self.preferences, params),
                ("app.set_home", _, _) => {
                    self.library_editors.focus(None)?;
                    if params["shown"].as_bool() == Some(true) {
                        if !self.home {
                            self.show_home();
                        }
                    } else {
                        self.home = false;
                    }
                }
                ("file.save", _, _) => {
                    if self.session.project().is_none() {
                        return Err("No project is open.".to_owned());
                    }
                    if !self.files.has_location(&self.session) {
                        return Err("This project hasn't been saved to a location yet. Choose Save in the editor to pick one.".to_owned());
                    }
                    self.save_project_file(None, ctx);
                    if let Some(message) = self.message.clone() {
                        return Err(message);
                    }
                }
                _ => return Err(format!("`{id}` can't run now")),
            }
            debug_assert!(self.views.host == self.views.main);
            self.sync_library_focus();
            self.repaint_editors(ctx);
            ctx.request_repaint_of(egui::ViewportId::ROOT);
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Every window command sits in a namespace that MCP routes to the window, and each one
        /// has a handler, so headless sessions refuse them and live discovery runs them all.
        #[test]
        fn window_commands_use_window_namespaces_and_unique_ids() {
            let mut seen = std::collections::BTreeSet::new();
            for command in CATALOG {
                assert!(pcbcraft_mcp::is_window_command(command.id), "{}", command.id);
                assert!(seen.insert(command.id), "{}", command.id);
                assert!(command.params.iter().all(|param| !param.summary.is_empty()));
            }
            assert!(!pcbcraft_engine::catalog().iter().any(|command| pcbcraft_mcp::is_window_command(command.id)));
        }

        /// Parameters are checked before anything changes, including step and bounds.
        #[test]
        fn parameters_are_checked_strictly() {
            let preferences = CATALOG.iter().find(|command| command.id == "app.set_preferences").unwrap();
            assert!(preferences.check(&json!({"theme": "light", "scale_percent": 125})).is_ok());
            assert!(preferences.check(&Value::Null).is_ok());
            for bad in [json!({"theme": "blue"}), json!({"scale_percent": 126}), json!({"zoom_speed_percent": 40}), json!({"other": 1}), json!([])] {
                assert!(preferences.check(&bad).is_err(), "{bad}");
            }
            let center = CATALOG.iter().find(|command| command.id == "view.center").unwrap();
            assert!(center.check(&json!({"view": 0, "at": {"x_nm": 0, "y_nm": 5}})).is_ok());
            assert!(center.check(&json!({"view": 0, "at": {"x_nm": 0}})).is_err());
            assert!(center.check(&json!({"view": -1, "at": {"x_nm": 0, "y_nm": 0}})).is_err());
            assert!(center.check(&json!({"at": {"x_nm": 0, "y_nm": 0}})).is_err());
        }

        /// The live selection schema advertises the same named-anchor limit its validator
        /// enforces, while legacy symbol/wire requests continue to work without the optional list.
        #[test]
        fn selection_schema_and_validation_agree_on_named_anchor_limit() {
            let schema = Kind::Selection.schema();
            let limit = pcbcraft_engine::net_labels::MAX_LABELS;
            assert_eq!(schema["properties"]["net_labels"]["maxItems"], json!(limit));
            assert!(schema["properties"]["symbols"].get("maxItems").is_none());
            assert!(schema["properties"]["wires"].get("maxItems").is_none());
            let select = CATALOG.iter().find(|command| command.id == "view.select").unwrap();
            assert!(select.check(&json!({"view":0,"items":{"symbols":[],"wires":[]}})).is_ok());
            let mut labels: Vec<_> = (1..=limit).map(|id| format!("net-label-{id}")).collect();
            assert!(select.check(&json!({"view":0,"items":{"symbols":[],"wires":[],"net_labels":labels}})).is_ok());
            labels.push(format!("net-label-{}", limit + 1));
            assert!(select.check(&json!({"view":0,"items":{"symbols":[],"wires":[],"net_labels":labels}})).is_err());
        }

        /// Invalid library identifiers must direct recovery to library inspection, with the
        /// existing nonnegative-integer schema retained for every library command parameter.
        #[test]
        fn library_parameter_refusals_identify_the_correct_inspection_state() {
            for command in CATALOG.iter().filter(|c| c.id.starts_with("app.library_")) {
                for param in command.params {
                    let expected = match param.name {
                        "document" => "must be a library document ID from inspect_editor.library_editors",
                        "expected_revision" => "must be the library document revision from inspect_editor.library_editors",
                        "selected" => "must be a primitive ID from library inspection",
                        _ => continue,
                    };
                    assert_eq!(param.kind.schema(), json!({"type":"integer","minimum":0}));
                    assert!(param.kind.validate(&json!(0)).is_ok());
                    assert!(param.kind.validate(&json!(u64::MAX)).is_ok());
                    for bad in [json!(-1), json!("1"), json!(true), json!(1.5), Value::Null] {
                        assert_eq!(param.kind.validate(&bad).unwrap_err(), expected, "{} {}", command.id, param.name);
                    }
                }
            }
        }

        /// The names inspection reports are the names the command takes, and unnamed settings stay.
        #[test]
        fn preferences_round_trip_through_their_command_names() {
            let changed = updated_preferences(
                Preferences::default(),
                &json!({"theme": "light", "units": "mm", "selection_follow": "both", "scroll": "pan", "zoom_around": "center", "grid_opacity_percent": 40, "board_grid_style": "dots"}),
            );
            let inspected = inspect_preferences(&changed);
            assert_eq!(updated_preferences(Preferences::default(), &inspected), changed);
            assert_eq!(updated_preferences(changed, &json!({})), changed);
            assert_eq!(changed.scale_percent, Preferences::default().scale_percent);
            let hidden = updated_preferences(changed, &json!({"schematic_grid_visible":false,"board_grid_visible":false,"grid_opacity_percent":0}));
            assert!(!hidden.grid_visible(false) && !hidden.grid_visible(true));
            assert_eq!(updated_preferences(Preferences::default(), &inspect_preferences(&hidden)), hidden);
        }
    }
}
