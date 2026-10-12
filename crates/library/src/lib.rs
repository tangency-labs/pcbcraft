//! Bounded KiCad library readers. Geometry is millimetres in screen (y-down) coordinates.
mod paths;
mod reader;
mod sexpr;
mod validation;
use pcbcraft_geom::Point;
pub use reader::{read_footprint, read_symbols};
use serde::{Deserialize, Serialize};
pub use sexpr::{Error, Limits};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolDefinition {
    pub name: String,
    pub reference: String,
    pub value: String,
    pub description: String,
    pub keywords: String,
    pub footprint: String,
    pub pins: Vec<SymbolPin>,
    pub graphics: Vec<UnitGraphic>,
    pub extends: Option<String>,
    pub raw_source: String,
    pub unsupported: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolPin {
    pub number: String,
    pub name: String,
    pub position: Point,
    /// KiCad angle, counterclockwise in a y-up Cartesian frame.
    pub angle: f64,
    pub length: f64,
    pub electrical_type: String,
    pub unit: u32,
    pub body_style: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitGraphic {
    pub unit: u32,
    pub body_style: u32,
    pub primitive: Graphic,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Graphic {
    Line {
        start: Point,
        end: Point,
        width: f64,
        layer: String,
    },
    Rectangle {
        start: Point,
        end: Point,
        width: f64,
        fill: String,
        layer: String,
    },
    Circle {
        center: Point,
        radius: f64,
        width: f64,
        fill: String,
        layer: String,
    },
    Arc {
        start: Point,
        mid: Point,
        end: Point,
        width: f64,
        layer: String,
    },
    Polyline {
        points: Vec<Point>,
        width: f64,
        fill: String,
        layer: String,
    },
    Text {
        position: Point,
        angle: f64,
        text: String,
        size: Point,
        layer: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FootprintDefinition {
    pub name: String,
    pub description: String,
    pub keywords: String,
    pub pads: Vec<FootprintPad>,
    pub graphics: Vec<Graphic>,
    pub raw_source: String,
    pub unsupported: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FootprintPad {
    pub number: String,
    pub pad_type: String,
    pub shape: String,
    pub position: Point,
    pub rotation: f64,
    pub size: Point,
    pub layers: Vec<String>,
    pub drill: Option<Drill>,
    pub roundrect_ratio: Option<f64>,
    pub raw_source: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drill {
    pub size: Point,
    pub offset: Point,
}
