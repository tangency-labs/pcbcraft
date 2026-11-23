//! Personal layer and net colors are shared across views of a board, outside engineering files.
use crate::{
    PcbApp, ViewId, Workspace,
    editor_view::{DocumentTarget, ViewInput},
};
use pcbcraft_chrome::board;
use pcbcraft_engine::board::{
    setup::{Layer, LayerPurpose, Side},
    track::Copper,
    view::{BoardTarget, BoardView, CopperLayers},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Snapshot-local connectivity shared by manual net emphasis and the route preview.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Connectivity {
    pub(super) copper: Copper,
    pub(super) names_pending: bool,
    pub(super) nets: BTreeMap<Uuid, String>,
    pub(super) net_ids: Vec<Option<Uuid>>,
}

impl Connectivity {
    /// Resolve copper only when its island has an unambiguous active net identity.
    fn island_net(&self, island: Option<&usize>) -> Option<Uuid> {
        *self.net_ids.get(*island?)?
    }

    /// Resolve a selected track or via without relying on a possibly duplicated display name.
    pub(super) fn copper_net(&self, id: &Uuid) -> Option<Uuid> {
        self.island_net(self.copper.tracks.get(id).or_else(|| self.copper.vias.get(id)))
    }

    /// Resolve an inspected pad at its exact physical board address.
    pub(super) fn pad_net(&self, footprint: &str, number: &str) -> Option<Uuid> {
        self.island_net(self.copper.pads.get(&(footprint.to_owned(), number.to_owned())))
    }
}

/// One board palette and the physical membership of its visible copper.
#[derive(Clone, Copy)]
pub(super) struct NetColors<'a> {
    pub(super) palette: &'a BTreeMap<Uuid, egui::Color32>,
    pub(super) connectivity: &'a Connectivity,
}

impl NetColors<'_> {
    /// Use the track's resolved electrical identity rather than a possibly shared display name.
    pub(super) fn track(self, id: &Uuid) -> Option<egui::Color32> {
        self.connectivity.copper_net(id).and_then(|net| self.palette.get(&net).copied())
    }

    /// Apply a net color only to physical pads on a resolved island.
    pub(super) fn pad(self, footprint: &str, number: &str) -> Option<egui::Color32> {
        self.connectivity.pad_net(footprint, number).and_then(|net| self.palette.get(&net).copied())
    }
}

/// The contact a route started from. A trace or via without a logical net can still have an island.
#[derive(Clone, Debug)]
pub(super) enum RouteContact {
    Pad(Option<String>),
    Copper(Uuid),
}

#[derive(Clone, Copy)]
enum FocusTarget<'a> {
    Net(&'a str),
    Island(usize),
}

/// The copper that should remain prominent in a single board canvas.
#[derive(Clone, Copy)]
pub(super) struct CopperFocus<'a> {
    target: FocusTarget<'a>,
    connectivity: &'a Connectivity,
}

impl<'a> CopperFocus<'a> {
    /// Prefer the current route's net or island, then fall back to the explicit net highlight.
    pub(super) fn new(connectivity: &'a Connectivity, route: Option<&'a RouteContact>, net: Option<&'a str>) -> Option<Self> {
        let route = match route {
            Some(RouteContact::Pad(net)) => net.as_deref().map(FocusTarget::Net),
            Some(RouteContact::Copper(id)) => connectivity
                .nets
                .get(id)
                .map(|net| FocusTarget::Net(net))
                .or_else(|| connectivity.copper.tracks.get(id).or_else(|| connectivity.copper.vias.get(id)).copied().map(FocusTarget::Island)),
            None => None,
        };
        Some(Self { target: route.or_else(|| net.map(FocusTarget::Net))?, connectivity })
    }

    /// Whether this track or via belongs to the emphasized net or island.
    pub(super) fn contains_copper(self, id: &Uuid) -> bool {
        match self.target {
            FocusTarget::Net(net) => self.connectivity.nets.get(id).is_some_and(|name| name == net),
            FocusTarget::Island(island) => self.connectivity.copper.tracks.get(id).or_else(|| self.connectivity.copper.vias.get(id)) == Some(&island),
        }
    }

    /// Whether this pad belongs to the emphasized net or island.
    pub(super) fn contains_pad(self, footprint: &str, number: &str, net: Option<&str>) -> bool {
        match self.target {
            FocusTarget::Net(target) => net == Some(target),
            FocusTarget::Island(island) => self.connectivity.copper.pads.get(&(footprint.to_owned(), number.to_owned())) == Some(&island),
        }
    }
}

// Preserve the first palette's local-storage key so existing copper overrides survive.
pub(super) const STORAGE_KEY: &str = "pcbcraft.copper-colors.v1";
const MAX_COLORS: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Override {
    board: BoardTarget,
    layer: String,
    rgb: [u8; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NetOverride {
    board: BoardTarget,
    net: Uuid,
    rgb: [u8; 3],
}

/// Colors resolved once for one canvas pass, including the current theme and personal overrides.
/// This disposable palette never survives a layer, visibility, theme or preference change.
pub(super) struct FootprintColors {
    technical: Vec<(LayerPurpose, Side, Option<egui::Color32>)>,
    copper: BTreeMap<Uuid, egui::Color32>,
    active: Option<Uuid>,
    component: egui::Color32,
    through: egui::Color32,
}

impl FootprintColors {
    /// Capture visible layer colors once rather than rebuilding the layer catalog for each path.
    pub(super) fn new(appearance: &Appearance, view: BoardView<'_>, active: Option<&str>, hidden: &BTreeSet<String>) -> Self {
        let technical = [LayerPurpose::Silkscreen, LayerPurpose::Fabrication, LayerPurpose::Courtyard]
            .into_iter()
            .flat_map(|purpose| {
                [Side::Front, Side::Back].into_iter().map(move |side| (purpose, side, appearance.technical(view, purpose, side, active, hidden)))
            })
            .collect();
        let copper: BTreeMap<_, _> = view
            .setup()
            .into_iter()
            .flat_map(|setup| setup.layers())
            .filter(|layer| layer.purpose == LayerPurpose::Copper && !hidden.contains(&layer.id.to_string()))
            .map(|layer| (layer.id, appearance.color(view, layer)))
            .collect();
        let active = copper.keys().find(|id| active == Some(id.to_string().as_str())).copied();
        Self { technical, copper, active, component: board::copper_color(Some(true)), through: pcbcraft_chrome::Tokens::get().board_through }
    }

    /// Preserve component-side fallback and membership order; active visible copper wins.
    pub(super) fn copper(&self, membership: &CopperLayers) -> Option<egui::Color32> {
        match membership {
            CopperLayers::Unconfigured { binding } => Some(match binding {
                pcbcraft_engine::board::definition::CopperBinding::ComponentSide => self.component,
                pcbcraft_engine::board::definition::CopperBinding::AllCopper => self.through,
            }),
            CopperLayers::Resolved { layers, .. } => self
                .active
                .filter(|id| layers.contains(id))
                .and_then(|id| self.copper.get(&id))
                .or_else(|| layers.iter().find_map(|id| self.copper.get(id)))
                .copied(),
        }
    }

    /// Select the already resolved component-side technical ink for a footprint path.
    pub(super) fn technical(&self, purpose: LayerPurpose, side: Side) -> Option<egui::Color32> {
        self.technical.iter().find(|(p, s, _)| *p == purpose && *s == side).and_then(|(_, _, color)| *color)
    }
}

/// Bounded local settings, keyed by identities rather than display names or stack positions.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Appearance {
    colors: Vec<Override>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    net_colors: Vec<NetOverride>,
    #[serde(skip)]
    lookup: BTreeMap<(BoardTarget, String), [u8; 3]>,
    #[serde(skip)]
    net_lookup: BTreeMap<BoardTarget, BTreeMap<Uuid, egui::Color32>>,
    #[serde(skip)]
    pub(super) dirty: bool,
}

impl Appearance {
    /// Invalid/oversized local settings fall back to defaults without affecting an open project.
    pub(super) fn from_storage(text: Option<String>) -> Self {
        text.filter(|text| text.len() <= 1_048_576)
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .filter(|data| {
                let mut keys = BTreeSet::new();
                let mut net_keys = BTreeSet::new();
                data.colors.len() + data.net_colors.len() <= MAX_COLORS
                    && data.colors.iter().all(|item| {
                        !item.board.project.is_nil()
                            && !item.board.board.is_nil()
                            && valid_id(&item.layer)
                            && keys.insert((item.board, item.layer.clone()))
                    })
                    && data.net_colors.iter().all(|item| {
                        !item.board.project.is_nil() && !item.board.board.is_nil() && !item.net.is_nil() && net_keys.insert((item.board, item.net))
                    })
            })
            .map(|mut data| {
                data.lookup = data.colors.iter().map(|item| ((item.board, item.layer.clone()), item.rgb)).collect();
                for item in &data.net_colors {
                    let [r, g, b] = item.rgb;
                    data.net_lookup.entry(item.board).or_default().insert(item.net, egui::Color32::from_rgb(r, g, b));
                }
                data
            })
            .unwrap_or_default()
    }
    /// Serialization writes only local appearance; colors never enter project documents/archives.
    pub(super) fn storage_text(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }
    /// Whether this exact layer has a personal override rather than a physical-role default.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn overridden(&self, board: BoardTarget, layer: &str) -> bool {
        self.lookup.contains_key(&(board, layer.to_owned()))
    }
    /// Resolve an exact layer override, otherwise use the shared physical-role palette.
    pub(super) fn color(&self, view: BoardView<'_>, layer: &Layer) -> egui::Color32 {
        if let Some(rgb) = self.lookup.get(&(view.target, layer.id.to_string())) {
            return egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        }
        default_color(view, layer)
    }
    /// Replace/reset one validated address atomically. Repeated values do not dirty settings.
    fn set(&mut self, board: BoardTarget, layer: &str, rgb: Option<[u8; 3]>) -> Result<(), String> {
        let index = self.colors.iter().position(|item| item.board == board && item.layer == layer);
        if index.and_then(|i| self.colors.get(i)).map(|item| item.rgb) == rgb {
            return Ok(());
        }
        if index.is_none() && rgb.is_some() && self.colors.len() + self.net_colors.len() >= MAX_COLORS {
            return Err("The saved color limit has been reached. Reset an existing override first.".into());
        }
        if let Some(index) = index {
            self.colors.remove(index);
        }
        if let Some(rgb) = rgb {
            self.colors.push(Override { board, layer: layer.to_owned(), rgb });
        }
        self.colors.sort_by(|a, b| (a.board, &a.layer).cmp(&(b.board, &b.layer)));
        match rgb {
            Some(rgb) => {
                self.lookup.insert((board, layer.to_owned()), rgb);
            }
            None => {
                self.lookup.remove(&(board, layer.to_owned()));
            }
        }
        self.dirty = true;
        Ok(())
    }
    /// Return only the addressed board's custom net palette when this view displays it.
    pub(super) fn net_palette(&self, board: BoardTarget, enabled: bool) -> Option<&BTreeMap<Uuid, egui::Color32>> {
        self.net_lookup.get(&board).filter(|palette| enabled && !palette.is_empty())
    }

    /// Replace or reset a local net override without changing electrical intent or history.
    fn set_net(&mut self, board: BoardTarget, net: Uuid, rgb: Option<[u8; 3]>) -> Result<(), String> {
        let index = self.net_colors.iter().position(|item| item.board == board && item.net == net);
        if index.map(|i| self.net_colors[i].rgb) == rgb {
            return Ok(());
        }
        if index.is_none() && rgb.is_some() && self.colors.len() + self.net_colors.len() >= MAX_COLORS {
            return Err("The saved color limit has been reached. Reset an existing override first.".into());
        }
        if let Some(index) = index {
            self.net_colors.remove(index);
        }
        if let Some(rgb) = rgb {
            let [r, g, b] = rgb;
            self.net_colors.push(NetOverride { board, net, rgb });
            self.net_colors.sort_by_key(|item| (item.board, item.net));
            self.net_lookup.entry(board).or_default().insert(net, egui::Color32::from_rgb(r, g, b));
        } else if let Some(palette) = self.net_lookup.get_mut(&board) {
            palette.remove(&net);
        }
        self.dirty = true;
        Ok(())
    }
    /// Technical footprint graphics follow their current component-side purpose binding. The
    /// active matching visible layer wins; unconfigured boards retain existing preview defaults.
    pub(super) fn technical(
        &self,
        view: BoardView<'_>,
        purpose: LayerPurpose,
        side: Side,
        active: Option<&str>,
        hidden: &BTreeSet<String>,
    ) -> Option<egui::Color32> {
        if view.setup().is_none() {
            return match purpose {
                LayerPurpose::Silkscreen => Some(board::graphic_color(board::GraphicStyle::Silkscreen)),
                LayerPurpose::Fabrication | LayerPurpose::Courtyard => None,
                _ => None,
            };
        }
        let layers: Vec<_> = view
            .layers()
            .into_iter()
            .filter(|layer| layer.purpose == purpose && (layer.side == Some(side) || layer.side.is_none()) && !hidden.contains(&layer.id.to_string()))
            .collect();
        let chosen = layers.iter().find(|layer| active == Some(layer.id.to_string().as_str())).or_else(|| layers.first())?;
        Some(self.color(view, chosen))
    }

    /// Choose only visible copper that this pad actually occupies. Active copper is drawn on top;
    /// otherwise physical stack order wins. Unconfigured compatibility bindings stay explicit.
    #[cfg(test)]
    pub(super) fn copper(
        &self,
        view: BoardView<'_>,
        membership: &CopperLayers,
        active: Option<&str>,
        hidden: &BTreeSet<String>,
    ) -> Option<egui::Color32> {
        match membership {
            CopperLayers::Unconfigured { binding } => Some(match binding {
                pcbcraft_engine::board::definition::CopperBinding::ComponentSide => board::copper_color(Some(true)),
                pcbcraft_engine::board::definition::CopperBinding::AllCopper => pcbcraft_chrome::Tokens::get().board_through,
            }),
            CopperLayers::Resolved { layers, .. } => {
                let visible = |id: &_| !hidden.contains(&format!("{id}"));
                let chosen = layers
                    .iter()
                    .find(|id| visible(id) && active == Some(id.to_string().as_str()))
                    .or_else(|| layers.iter().find(|id| visible(id)))?;
                view.setup()?.layers().iter().find(|layer| layer.id == *chosen).map(|layer| self.color(view, layer))
            }
        }
    }
}

/// Validate canonical layer UUID text in untrusted local settings, without assigning any layer.
fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id != "00000000-0000-0000-0000-000000000000"
        && id.bytes().enumerate().all(
            |(i, b)| {
                if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() && !b.is_ascii_uppercase() }
            },
        )
}

/// Defaults follow layer purpose and side; alternating internal copper keys remain distinct.
pub(super) fn default_color(view: BoardView<'_>, layer: &Layer) -> egui::Color32 {
    if layer.purpose != LayerPurpose::Copper {
        return match layer.purpose {
            LayerPurpose::Silkscreen => board::graphic_color(board::GraphicStyle::Silkscreen),
            LayerPurpose::Fabrication => board::graphic_color(board::GraphicStyle::Fabrication),
            LayerPurpose::Courtyard => board::graphic_color(board::GraphicStyle::Courtyard),
            LayerPurpose::Outline | LayerPurpose::Mechanical | LayerPurpose::User => pcbcraft_chrome::Tokens::get().board_ink,
            _ => pcbcraft_chrome::Tokens::get().text_dim,
        };
    }
    match layer.side {
        Some(Side::Front) => board::copper_color(Some(true)),
        Some(Side::Back) => board::copper_color(Some(false)),
        None => {
            let inner = view
                .setup()
                .map(|setup| {
                    setup
                        .stackup()
                        .strata
                        .iter()
                        .filter_map(|stratum| {
                            let pcbcraft_engine::board::setup::Stratum::Copper { layer, .. } = stratum else { return None };
                            setup.layers().iter().find(|item| item.id == *layer && item.side.is_none()).map(|item| item.id)
                        })
                        .position(|id| id == layer.id)
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            if inner.is_multiple_of(2) { pcbcraft_chrome::Tokens::get().board_inner } else { pcbcraft_chrome::Tokens::get().board_inner_alt }
        }
    }
}

/// Canonical opaque sRGB input shared by the dialog and live presentation command.
pub(super) fn parse_color(text: &str) -> Result<[u8; 3], String> {
    if text.len() != 7 || !text.starts_with('#') || !text.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err("Use a color in #RRGGBB format.".into());
    }
    let mut rgb = [0; 3];
    for (index, value) in rgb.iter_mut().enumerate() {
        *value = u8::from_str_radix(&text[1 + 2 * index..3 + 2 * index], 16).map_err(|_| "Use a color in #RRGGBB format.")?;
    }
    Ok(rgb)
}

/// Format the RGB channels as canonical #RRGGBB for the opaque layer-color controls.
pub(super) fn hex(color: egui::Color32) -> String {
    let [r, g, b, _] = color.to_array();
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// A disposable color draft captures its view authority; Apply never edits board geometry/history.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    target: ColorTarget,
    return_rules: Option<Box<crate::net_class_editor::Edit>>,
    label: String,
    rgb: [u8; 3],
    hex: String,
    error: Option<String>,
}

impl Edit {
    /// Keep the modal in the window that owns the input authority.
    pub(super) fn source(&self) -> ViewInput {
        self.source
    }
}

#[derive(Debug)]
enum ColorTarget {
    Layer(String),
    Net { board: BoardTarget, net: Uuid },
}

impl PcbApp {
    /// Validate the addressed layer before changing any local settings. All editor views
    /// share this board/layer palette; unknown and foreign layers are refused atomically.
    pub(super) fn set_layer_color(&mut self, view: ViewId, layer: &str, rgb: Option<[u8; 3]>) -> Result<(), String> {
        let target = self
            .views
            .view(view)
            .filter(|view| view.workspace == Workspace::Board)
            .and_then(|view| view.board.target)
            .ok_or("The addressed view must show a board")?;
        let board = self.session.project().and_then(|project| project.board_view(target)).ok_or("The view's board is unavailable")?;
        if !board.layers().iter().any(|item| item.id.to_string() == layer) {
            return Err("Unknown layer on this view's board".into());
        }
        self.board_appearance.set(target, layer, rgb)
    }
    /// Set a personal color for an active persistent net, enabling net colors in the initiating view.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn set_net_color(&mut self, view: ViewId, net: Uuid, rgb: Option<[u8; 3]>) -> Result<(), String> {
        let editor = self.views.view(view).filter(|view| view.workspace == Workspace::Board).ok_or("The addressed view must show a board")?;
        let target = editor.board.target.ok_or("No board is bound to this view")?;
        self.set_board_net_color(view, target, net, rgb)
    }

    /// Save settings for the validated board. Only an initiating PCB view changes its toggle.
    fn set_board_net_color(&mut self, view: ViewId, target: BoardTarget, net: Uuid, rgb: Option<[u8; 3]>) -> Result<(), String> {
        let board = self.session.project().and_then(|project| project.board_view(target)).ok_or("The view's board is unavailable")?;
        if board.zone_net_state(net) != "active" {
            return Err("Choose an active persistent net on this view's board.".into());
        }
        self.board_appearance.set_net(target, net, rgb)?;
        if rgb.is_some()
            && let Some(editor) = self.views.view_mut(view)
            && editor.target() == Some(DocumentTarget::Board(target))
        {
            editor.board.set_net_colors_visible(true);
        }
        Ok(())
    }

    /// Open the same themed color dialog from a layer swatch without activating that layer.
    pub(super) fn begin_layer_color(&mut self, view: ViewId, layer: String) {
        let Some(editor) = self.views.view(view) else { return };
        let Some(source) = editor.input(&self.session) else { return };
        let Some(board) = self.session.project().and_then(|project| editor.board.target.and_then(|target| project.board_view(target))) else {
            return;
        };
        let Some(record) = board.layers().into_iter().find(|item| item.id.to_string() == layer) else {
            return;
        };
        let color = self.board_appearance.color(board, &record);
        let [r, g, b, _] = color.to_array();
        self.dialog = crate::Dialog::BoardColor(Edit {
            source,
            target: ColorTarget::Layer(layer),
            return_rules: None,
            label: record.name,
            rgb: [r, g, b],
            hex: hex(color),
            error: None,
        });
    }

    /// Open the shared picker from PCB Properties using the exact active net identity.
    pub(super) fn begin_board_net_color(&mut self, view: ViewId, net: Uuid, layer: Option<Uuid>) {
        let Some(editor) = self.views.view(view) else { return };
        let Some(source) = editor.input(&self.session) else { return };
        let Some(DocumentTarget::Board(target)) = editor.target() else { return };
        let Some(board) = self.session.project().and_then(|project| project.board_view(target)) else { return };
        let label = board
            .inspect_zone_nets()
            .ok()
            .and_then(|rows| crate::board_net_choice::net_choices(&rows).into_iter().find(|choice| choice.id == Some(net)))
            .map_or_else(|| net.to_string(), |choice| choice.label);
        self.begin_net_color_for(source, target, net, label, layer, None);
    }

    /// The picker shares one palette across Properties and Rules. Closing it resumes the Rules form.
    pub(super) fn begin_net_color_for(
        &mut self,
        source: ViewInput,
        target: BoardTarget,
        net: Uuid,
        label: String,
        layer: Option<Uuid>,
        return_rules: Option<Box<crate::net_class_editor::Edit>>,
    ) -> bool {
        let edit = self.net_color_edit(source, target, net, label, layer);
        if let Some(mut edit) = edit {
            edit.return_rules = return_rules;
            self.dialog = crate::Dialog::BoardColor(edit);
            true
        } else {
            if let Some(rules) = return_rules {
                self.dialog = crate::Dialog::Rules(rules);
            }
            false
        }
    }

    /// Bind the picker to the view's current revision and an explicit board in the same project.
    fn net_color_edit(&self, source: ViewInput, target: BoardTarget, net: Uuid, label: String, layer: Option<Uuid>) -> Option<Edit> {
        let editor = self.views.view(source.view()).filter(|editor| editor.accepts(source, &self.session))?;
        let project = self.session.project()?;
        match editor.target()? {
            DocumentTarget::Board(board) if board == target => {}
            DocumentTarget::Sheet(sheet) if project.for_sheet(sheet).is_some() => {}
            _ => return None,
        }
        let board = project.board_view(target)?;
        if board.zone_net_state(net) != "active" {
            return None;
        }
        let layer = layer.or_else(|| editor.board.active_layer.as_ref().and_then(|id| Uuid::try_parse(id).ok()));
        let color = self.board_appearance.net_palette(target, true).and_then(|palette| palette.get(&net)).copied().unwrap_or_else(|| {
            let layers = board.layers();
            layers
                .iter()
                .filter(|record| record.purpose == LayerPurpose::Copper)
                .find(|record| Some(record.id) == layer)
                .or_else(|| layers.iter().find(|record| record.purpose == LayerPurpose::Copper && record.side == Some(Side::Front)))
                .map_or_else(|| board::copper_color(Some(true)), |layer| self.board_appearance.color(board, layer))
        });
        let [r, g, b, _] = color.to_array();
        Some(Edit {
            source,
            target: ColorTarget::Net { board: target, net },
            return_rules: None,
            label,
            rgb: [r, g, b],
            hex: hex(color),
            error: None,
        })
    }

    /// Reject a stale draft before applying local sRGB settings or a reset.
    pub(super) fn board_color_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        if !self.views.view(edit.source.view()).is_some_and(|view| view.accepts(edit.source, &self.session)) {
            if let Some(rules) = edit.return_rules {
                self.dialog = crate::Dialog::Rules(rules);
            }
            return;
        }
        let (title, reset) = match &edit.target {
            ColorTarget::Layer(_) => ("Layer Color", "Reset to default"),
            ColorTarget::Net { .. } => ("Net Color", "Use layer color"),
        };
        let action = board::color_dialog(ctx, title, &edit.label, reset, &mut edit.rgb, &mut edit.hex, edit.error.as_deref());
        let value = match action {
            board::LayerColorAction::Cancel => {
                if let Some(rules) = edit.return_rules {
                    self.dialog = crate::Dialog::Rules(rules);
                }
                return;
            }
            board::LayerColorAction::None | board::LayerColorAction::HexChanged => {
                if action == board::LayerColorAction::HexChanged
                    && let Ok(rgb) = parse_color(&edit.hex)
                {
                    edit.rgb = rgb;
                    edit.error = None;
                }
                self.dialog = crate::Dialog::BoardColor(edit);
                return;
            }
            board::LayerColorAction::Apply => parse_color(&edit.hex).map(Some),
            board::LayerColorAction::Reset => Ok(None),
        };
        let result = value.and_then(|rgb| match &edit.target {
            ColorTarget::Layer(layer) => self.set_layer_color(edit.source.view(), layer, rgb),
            ColorTarget::Net { board, net } => self.set_board_net_color(edit.source.view(), *board, *net, rgb),
        });
        match result {
            Ok(()) => {
                if let Some(rules) = edit.return_rules {
                    self.dialog = crate::Dialog::Rules(rules);
                }
                self.repaint_editors(ctx);
            }
            Err(error) => {
                edit.error = Some(error);
                self.dialog = crate::Dialog::BoardColor(edit);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-pass colors agree with the existing per-object resolver across themes, sides,
    /// hidden/active layers and personal overrides; rebuilding reflects changed preferences.
    #[test]
    fn footprint_palette_preserves_visible_layer_and_theme_rules() {
        let ctx = egui::Context::default();
        let mut session = pcbcraft_engine::Session::default();
        session.apply("project.new", &serde_json::json!({"create_board":true})).unwrap();
        for configured in [false, true] {
            if configured {
                session
                    .apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &serde_json::json!({"copper_layers":4,"expected_revision":session.revision()}))
                    .unwrap();
            }
            let board = session.project().unwrap().boards().next().unwrap();
            let layers = board.layers();
            let mut appearance = Appearance::default();
            for (index, layer) in layers.iter().enumerate() {
                appearance.set(board.target, &layer.id.to_string(), Some([index as u8, 43, 91])).unwrap();
            }
            let memberships = [
                CopperLayers::Unconfigured { binding: pcbcraft_engine::board::definition::CopperBinding::ComponentSide },
                CopperLayers::Unconfigured { binding: pcbcraft_engine::board::definition::CopperBinding::AllCopper },
                CopperLayers::Resolved {
                    binding: pcbcraft_engine::board::definition::CopperBinding::AllCopper,
                    layers: layers.iter().filter(|l| l.purpose == LayerPurpose::Copper).map(|l| l.id).rev().collect(),
                },
            ];
            let defaults = Appearance::default();
            for appearance in [&defaults, &appearance] {
                for dark in [true, false] {
                    pcbcraft_chrome::theme::apply(&ctx, dark);
                    for hidden in [
                        BTreeSet::new(),
                        layers.iter().step_by(2).map(|l| l.id.to_string()).collect(),
                        layers.iter().map(|l| l.id.to_string()).collect(),
                    ] {
                        for active in std::iter::once(None).chain(layers.iter().map(|l| Some(l.id.to_string()))) {
                            let resolved = FootprintColors::new(appearance, board, active.as_deref(), &hidden);
                            for membership in &memberships {
                                assert_eq!(resolved.copper(membership), appearance.copper(board, membership, active.as_deref(), &hidden));
                            }
                            for purpose in [LayerPurpose::Silkscreen, LayerPurpose::Fabrication, LayerPurpose::Courtyard] {
                                for side in [Side::Front, Side::Back] {
                                    assert_eq!(
                                        resolved.technical(purpose, side),
                                        appearance.technical(board, purpose, side, active.as_deref(), &hidden)
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        pcbcraft_chrome::theme::apply(&ctx, true);
    }
    /// Invalid settings and non-ASCII color input cannot panic or introduce ambiguous addresses.
    #[test]
    fn local_settings_and_hex_inputs_are_bounded_and_validated() {
        for bad in ["#éxxxx", "#💥xx", "#GG0011", "112233", "#1234567", ""] {
            assert!(parse_color(bad).is_err());
        }
        assert_eq!(parse_color("#aB12fF").unwrap(), [171, 18, 255]);
        for bad in ["{}", "{\"colors\":null}", "not json"] {
            assert_eq!(Appearance::from_storage(Some(bad.into())).colors.len(), 0);
        }
        assert!(Appearance::from_storage(Some("x".repeat(1_048_577))).colors.is_empty());
        let mut session = pcbcraft_engine::Session::default();
        session.apply("project.new", &serde_json::json!({"create_board":true})).unwrap();
        session
            .apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &serde_json::json!({"copper_layers":2,"expected_revision":session.revision()}))
            .unwrap();
        let board = session.project().unwrap().boards().next().unwrap();
        let layer = board.layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Copper).unwrap();
        let mut appearance = Appearance::default();
        appearance.set(board.target, &layer.id.to_string(), Some([4, 5, 6])).unwrap();
        let saved = appearance.storage_text().unwrap();
        let restored = Appearance::from_storage(Some(saved.clone()));
        assert_eq!(restored.color(board, &layer).to_array(), [4, 5, 6, 255]);
        assert!(!restored.dirty);
        let mut bad: serde_json::Value = serde_json::from_str(&saved).unwrap();
        let copy = bad["colors"][0].clone();
        bad["colors"].as_array_mut().unwrap().push(copy);
        assert!(Appearance::from_storage(Some(bad.to_string())).colors.is_empty());
        let before = appearance.storage_text();
        appearance.dirty = false;
        appearance.set(board.target, &layer.id.to_string(), Some([4, 5, 6])).unwrap();
        assert!(!appearance.dirty);
        assert_eq!(appearance.storage_text(), before);
    }
}
