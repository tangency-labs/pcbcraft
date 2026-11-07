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
}

impl PcbApp {
    /// Show or hide one of the bound window's panels, as the View ribbon's toggles do. Opening
    /// Parts focuses its search, like Capture › Symbol; an already open card is left alone.
    pub(super) fn set_panel(&mut self, ctx: &Context, panel: Panel, open: bool) {
        match panel {
            Panel::Browser => self.browser.open = open,
            Panel::Parts if open => {
                if !self.parts.open {
                    self.parts.focus_search();
                }
            }
            Panel::Parts => self.parts.close(ctx),
            Panel::Nets => self.net_inspector.open = open,
        }
    }

    /// Whether the bound window shows `panel`.
    pub(super) fn panel_open(&self, panel: Panel) -> bool {
        match panel {
            Panel::Browser => self.browser.open,
            Panel::Parts => self.parts.open,
            Panel::Nets => self.net_inspector.open,
        }
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
        Dialog, PcbApp, RIBBON_TABS, ViewId, Workspace,
        arrangements::{Action, Start},
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
        /// A window ID from inspection.
        Window,
        /// An arrangement ID from inspection.
        Arrangement,
        /// An arrangement name: 1 to 80 characters on one line.
        Name,
        Bool,
        Choice(&'static [&'static str]),
        Integer {
            min: i64,
            max: i64,
            step: i64,
        },
        /// A schematic point in nanometres within the coordinate bounds.
        Point,
        /// Symbol and wire ID lists, as the project's group commands take.
        Selection,
        /// A list of distinct footprint IDs.
        Footprints,
        /// Persistent sheet UUID, validated against the current project before navigation.
        Sheet,
    }

    impl Kind {
        /// The JSON schema published to MCP clients.
        fn schema(self) -> Value {
            let coordinate = json!({"type": "integer", "minimum": -MAX_CENTER_NM, "maximum": MAX_CENTER_NM});
            let ids = json!({"type": "array", "uniqueItems": true, "items": {"type": "string"}});
            match self {
                Self::View | Self::Window | Self::Arrangement => json!({"type": "integer", "minimum": 0}),
                Self::Sheet => json!({"type":"string","format":"uuid"}),
                Self::Name => json!({"type": "string", "minLength": 1, "maxLength": 80}),
                Self::Bool => json!({"type": "boolean"}),
                Self::Choice(values) => json!({"type": "string", "enum": values}),
                Self::Integer { min, max, step } => json!({"type": "integer", "minimum": min, "maximum": max, "multipleOf": step}),
                Self::Point => {
                    json!({"type": "object", "properties": {"x_nm": coordinate, "y_nm": coordinate}, "required": ["x_nm", "y_nm"], "additionalProperties": false})
                }
                Self::Selection => {
                    json!({"type": "object", "properties": {"symbols": ids, "wires": ids}, "required": ["symbols", "wires"], "additionalProperties": false})
                }
                Self::Footprints => ids,
            }
        }

        /// Check a raw value without coercion, giving the reason it doesn't fit.
        fn validate(self, value: &Value) -> Result<(), String> {
            let fits = match self {
                Self::View | Self::Window | Self::Arrangement => value.as_u64().is_some(),
                Self::Name => value.as_str().is_some(),
                Self::Bool => value.is_boolean(),
                Self::Choice(values) => value.as_str().is_some_and(|value| values.contains(&value)),
                Self::Integer { min, max, step } => value.as_i64().is_some_and(|n| (min..=max).contains(&n) && n % step == 0),
                Self::Point => point(value).is_some(),
                Self::Sheet => value.as_str().is_some_and(|id| !id.is_empty() && id.len() <= 64),
                Self::Selection => Selection::parse(value).is_some(),
                Self::Footprints => footprints(value).is_some(),
            };
            if fits {
                return Ok(());
            }
            Err(match self {
                Self::View => "must be a view ID from inspect_editor".to_owned(),
                Self::Window => "must be a window ID from inspect_editor".to_owned(),
                Self::Arrangement => "must be an arrangement ID from inspect_editor".to_owned(),
                Self::Sheet => "must be a sheet UUID from project inspection".to_owned(),
                Self::Name => "must be a name".to_owned(),
                Self::Bool => "must be true or false".to_owned(),
                Self::Choice(values) => format!("must be one of {}", values.join(", ")),
                Self::Integer { min, max, step: 1 } => format!("must be a whole number from {min} to {max}"),
                Self::Integer { min, max, step } => format!("must be a multiple of {step} from {min} to {max}"),
                Self::Point => "must be a point with whole-nanometre x_nm and y_nm within the schematic bounds".to_owned(),
                Self::Selection => "must contain unique symbol and wire ID lists".to_owned(),
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
    const PANELS: &[&str] = &["browser", "parts", "nets"];
    const TABS: &[&str] = &["capture", "layout", "view"];
    const WORKSPACES: &[&str] = &["schematic", "board"];
    const PLACEMENTS: &[&str] = &["here", "beside", "window"];
    const BOARD_GRIDS: &[&str] = &["off", "dots", "lines"];

    /// Optional, so one call can change any subset; a call with none changes nothing.
    const fn setting(name: &'static str, summary: &'static str, kind: Kind) -> Param {
        Param { name, summary, required: false, kind }
    }

    const CATALOG: &[WindowCommand] = &[
        WindowCommand {
            id: "view.open_sheet",
            title: "Open Sheet",
            summary: "Open an explicitly addressed sheet in this pane, as its Browser row does. Other panes retain their targets.",
            params: &[VIEW, Param { name: "sheet", summary: "Sheet UUID from project inspection", required: true, kind: Kind::Sheet }],
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
            summary: "Move a view from another window beside the main window's focused pane.",
            params: &[VIEW],
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
            summary: "Zoom the view about its center. 100 shows the schematic at its default size.",
            params: &[
                VIEW,
                Param { name: "zoom_percent", summary: "The zoom in percent", required: true, kind: Kind::Integer { min: 1, max: 800, step: 1 } },
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
            id: "view.select",
            title: "Select",
            summary: "Replace a schematic view's selection with these symbols and wires, as clicking or box selection does. Empty lists clear it. Other views highlight it under the Follow selection setting. PCB views take view.select_footprints.",
            params: &[VIEW, Param { name: "items", summary: "IDs from inspect_project", required: true, kind: Kind::Selection }],
        },
        WindowCommand {
            id: "view.select_footprints",
            title: "Select Footprints",
            summary: "Replace a PCB view's selection with these footprints, as clicking them or their Browser rows does. An empty list clears it. Other views highlight the counterpart symbols under the Follow selection setting.",
            params: &[
                VIEW,
                Param {
                    name: "footprints",
                    summary: "Footprint IDs on the view's board, from inspect_project",
                    required: true,
                    kind: Kind::Footprints,
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
            summary: "Show or hide one of the window's panels: the Browser, the Parts library or the Net Inspector.",
            params: &[
                WINDOW,
                Param { name: "panel", summary: "Which panel", required: true, kind: Kind::Choice(PANELS) },
                Param { name: "open", summary: "Whether it shows", required: true, kind: Kind::Bool },
            ],
        },
        WindowCommand {
            id: "window.set_ribbon_tab",
            title: "Ribbon Tab",
            summary: "Show the window's first ribbon tab (Capture, or Layout when its focused view shows the PCB) or its View tab.",
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

    /// The settings with the given (already validated) changes applied; the rest are kept.
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
                    "panels": {"browser": panels.browser, "parts": panels.parts, "nets": panels.nets},
                    "ribbon_tab": match panels.ribbon {
                        0 if app.views.view(host.focused).is_some_and(|view| view.workspace == Workspace::Board) => "layout",
                        0 => "capture",
                        _ => "view",
                    },
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
                    "workspace": match view.workspace {
                        Workspace::Schematic => "schematic",
                        Workspace::Board => "board",
                    },
                    "sheet": view.sheet_path(&app.session),
                    "sheet_id": match view.target() {Some(crate::editor_view::DocumentTarget::Sheet(sheet))=>Some(sheet.id()),_=>None},
                    "grid_spacing_nm": view.camera.snap_nm(),
                    "zoom_percent": (view.camera.zoom() * 100.0).round(),
                    "center": view.camera.center(),
                    "selection": view.schematic.selection,
                    "selected_segment": view.schematic.selected_leg.as_ref().map(|leg| json!({"wire": leg.wire, "from": leg.route.get(leg.leg), "to": leg.route.get(leg.leg + 1)})),
                    "tool": view.schematic.tool.as_ref().map(tool_name),
                    "board": (view.workspace == Workspace::Board).then(|| json!({
                        "selection": view.board.selection,
                        "hidden_layers": view.board.hidden_layers,
                        "active_layer": view.board.active_layer,
                    })),
                }))
            })
            .collect();
        json!({
            "revision": app.session.revision(),
            "unsaved": app.files.dirty(&app.session),
            "file_operation": app.files.busy(),
            "dialog_open": !matches!(app.dialog, Dialog::None),
            "home": app.home,
            "message": app.message,
            "windows": windows,
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
            if (id.starts_with("view.") || id.starts_with("arrangement.") || id == "window.set_panel") && self.session.project().is_none() {
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
            match (id, view, window) {
                ("view.focus", Some((view, host)), _) => self.in_window(host, |app| app.activate_editor(ctx, view)),
                ("view.split", Some((view, host)), _) => {
                    let split = params["split"].as_bool() == Some(true);
                    self.in_window(host, |app| {
                        app.activate_editor(ctx, view);
                        app.split_editors(ctx, split);
                    });
                }
                ("view.close", Some((view, host)), _) if host == self.views.main && !self.views.hosts.get(&host).is_some_and(|host| host.split()) => {
                    return Err(format!("View {} is the main window's only view, which stays open. Close another view or window instead.", view.0));
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
                ("view.fit", Some((view, _)), _) => self.fit_view(view),
                ("view.center", Some((view, _)), _) => {
                    let at = point(&params["at"]).ok_or("`at` must be a point")?;
                    if let Some(view) = self.views.view_mut(view) {
                        view.camera.center_on(at);
                    }
                }
                ("view.zoom", Some((view, _)), _) => {
                    let percent = params["zoom_percent"].as_u64().unwrap_or(100);
                    if let Some(view) = self.views.view_mut(view) {
                        view.camera.set_zoom(percent as f32 / 100.0);
                    }
                }
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
                    let previous = self.views.active.id();
                    self.views.bind(view);
                    self.views.active.cancel_gesture();
                    self.views.select_board(wanted, &self.session, self.preferences.selection_follow);
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
                    let items = Selection::parse(&params["items"]).ok_or("`items` must contain symbol and wire ID lists")?;
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
                        _ => Panel::Nets,
                    };
                    let open = params["open"].as_bool() == Some(true);
                    self.in_window(host, |app| app.set_panel(ctx, panel, open));
                }
                ("window.set_ribbon_tab", _, Some(host)) => {
                    let tab = if params["tab"].as_str() == Some("view") { RIBBON_TABS.len() - 1 } else { 0 };
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
        }
    }
}
