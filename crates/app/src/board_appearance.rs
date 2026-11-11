//! Personal layer colors are shared across views by board/layer identity, outside engineering files.
use crate::{PcbApp, ViewId, Workspace, editor_view::ViewInput};
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
    pub(super) nets: BTreeMap<Uuid, String>,
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

/// Bounded local settings, keyed by identities rather than display names or stack positions.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Appearance {
    colors: Vec<Override>,
    #[serde(skip)]
    lookup: BTreeMap<(BoardTarget, String), [u8; 3]>,
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
                data.colors.len() <= MAX_COLORS
                    && data.colors.iter().all(|item| {
                        !item.board.project.is_nil()
                            && !item.board.board.is_nil()
                            && valid_id(&item.layer)
                            && keys.insert((item.board, item.layer.clone()))
                    })
            })
            .map(|mut data| {
                data.lookup = data.colors.iter().map(|item| ((item.board, item.layer.clone()), item.rgb)).collect();
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
        if index.is_none() && rgb.is_some() && self.colors.len() >= MAX_COLORS {
            return Err("The saved layer-color limit has been reached. Reset an existing override first.".into());
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
    layer: String,
    rgb: [u8; 3],
    hex: String,
    error: Option<String>,
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
        self.dialog = crate::Dialog::LayerColor(Edit { source, layer, rgb: [r, g, b], hex: hex(color), error: None });
    }
    /// Discard stale drafts and apply validated sRGB/reset through the shared presentation handler.
    pub(super) fn layer_color_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        if !self.views.view(edit.source.view()).is_some_and(|view| view.accepts(edit.source, &self.session)) {
            return;
        }
        let Some(record) = self
            .session
            .project()
            .and_then(|project| self.views.view(edit.source.view()).and_then(|view| view.board.target).and_then(|target| project.board_view(target)))
            .and_then(|board| board.layers().into_iter().find(|item| item.id.to_string() == edit.layer))
        else {
            return;
        };
        let action = board::layer_color_dialog(ctx, &record.name, &mut edit.rgb, &mut edit.hex, edit.error.as_deref());
        let value = match action {
            board::LayerColorAction::Cancel => return,
            board::LayerColorAction::None | board::LayerColorAction::HexChanged => {
                if action == board::LayerColorAction::HexChanged
                    && let Ok(rgb) = parse_color(&edit.hex)
                {
                    edit.rgb = rgb;
                    edit.error = None;
                }
                self.dialog = crate::Dialog::LayerColor(edit);
                return;
            }
            board::LayerColorAction::Apply => parse_color(&edit.hex).map(Some),
            board::LayerColorAction::Reset => Ok(None),
        };
        match value.and_then(|rgb| self.set_layer_color(edit.source.view(), &edit.layer, rgb)) {
            Ok(()) => self.repaint_editors(ctx),
            Err(error) => {
                edit.error = Some(error);
                self.dialog = crate::Dialog::LayerColor(edit);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
