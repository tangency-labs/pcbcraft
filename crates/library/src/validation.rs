use crate::*;
use std::collections::BTreeSet;
fn bad(message: &str) -> Error {
    Error::Invalid(message.to_owned())
}
fn text(s: &str) -> Result<(), Error> {
    if s.len() > 256 * 1024 || s.contains('\0') {
        Err(bad("text exceeds bounds or contains NUL"))
    } else {
        Ok(())
    }
}
fn scalar(v: f64) -> bool {
    v.is_finite() && v.abs() <= 100_000.0
}
fn dimension(v: f64) -> bool {
    scalar(v) && v >= 0.0
}
fn positive_size(p: Point) -> bool {
    p.valid() && p.x > 0.0 && p.y > 0.0
}
fn metadata(raw: &str, unsupported: &[String]) -> Result<(), Error> {
    if raw.len() > 8 * 1024 * 1024 || unsupported.len() > 10_000 {
        return Err(Error::Limit("definition metadata"));
    }
    for s in unsupported {
        text(s)?;
    }
    Ok(())
}
impl Graphic {
    pub fn validate(&self) -> Result<(), Error> {
        let valid = match self {
            Self::Line {
                start,
                end,
                width,
                layer,
            } => {
                text(layer)?;
                start.valid() && end.valid() && dimension(*width)
            }
            Self::Rectangle {
                start,
                end,
                width,
                fill,
                layer,
            } => {
                text(fill)?;
                text(layer)?;
                start.valid() && end.valid() && dimension(*width)
            }
            Self::Circle {
                center,
                radius,
                width,
                fill,
                layer,
            } => {
                text(fill)?;
                text(layer)?;
                center.valid() && *radius > 0.0 && dimension(*radius) && dimension(*width)
            }
            Self::Arc {
                start,
                mid,
                end,
                width,
                layer,
            } => {
                text(layer)?;
                start.valid() && mid.valid() && end.valid() && dimension(*width)
            }
            Self::Polyline {
                points,
                width,
                fill,
                layer,
            } => {
                text(fill)?;
                text(layer)?;
                points.len() >= 2
                    && points.len() <= 10_000
                    && points.iter().all(|p| p.valid())
                    && dimension(*width)
            }
            Self::Text {
                position,
                angle,
                text: content,
                size,
                layer,
            } => {
                text(content)?;
                text(layer)?;
                position.valid() && scalar(*angle) && positive_size(*size)
            }
        };
        if valid {
            Ok(())
        } else {
            Err(bad("invalid library graphic geometry"))
        }
    }
}
impl SymbolDefinition {
    pub fn validate(&self) -> Result<(), Error> {
        if self.name.is_empty() || self.pins.len() > 20_000 || self.graphics.len() > 20_000 {
            return Err(bad("invalid symbol name or object count"));
        }
        for s in [
            &self.name,
            &self.reference,
            &self.value,
            &self.description,
            &self.keywords,
            &self.footprint,
        ] {
            text(s)?;
        }
        if let Some(parent) = &self.extends {
            text(parent)?;
        }
        metadata(&self.raw_source, &self.unsupported)?;
        let mut keys = BTreeSet::new();
        for p in &self.pins {
            text(&p.number)?;
            text(&p.name)?;
            text(&p.electrical_type)?;
            if p.number.is_empty()
                || p.electrical_type.is_empty()
                || !p.position.valid()
                || !scalar(p.angle)
                || !dimension(p.length)
                || p.unit > 1024
                || p.body_style > 32
                || !keys.insert((p.unit, p.body_style, &p.number))
            {
                return Err(bad("invalid or duplicate symbol pin"));
            }
        }
        for g in &self.graphics {
            if g.unit > 1024 || g.body_style > 32 {
                return Err(bad("invalid graphic unit"));
            }
            g.primitive.validate()?;
        }
        Ok(())
    }
}
impl FootprintDefinition {
    pub fn validate(&self) -> Result<(), Error> {
        if self.name.is_empty() || self.pads.len() > 20_000 || self.graphics.len() > 20_000 {
            return Err(bad("invalid footprint name or object count"));
        }
        for s in [&self.name, &self.description, &self.keywords] {
            text(s)?;
        }
        metadata(&self.raw_source, &self.unsupported)?;
        for p in &self.pads {
            for s in [&p.number, &p.pad_type, &p.shape] {
                text(s)?;
            }
            metadata(&p.raw_source, &[])?;
            if !p.position.valid()
                || !scalar(p.rotation)
                || !positive_size(p.size)
                || p.layers.is_empty()
                || p.layers.len() > 64
                || p.shape.is_empty()
                || p.pad_type.is_empty()
            {
                return Err(bad("invalid footprint pad"));
            }
            for layer in &p.layers {
                text(layer)?;
            }
            if p.roundrect_ratio
                .is_some_and(|r| !r.is_finite() || !(0.0..=0.5).contains(&r))
            {
                return Err(bad("invalid roundrect ratio"));
            }
            if p.drill
                .as_ref()
                .is_some_and(|d| !positive_size(d.size) || !d.offset.valid())
            {
                return Err(bad("invalid pad drill"));
            }
        }
        for g in &self.graphics {
            g.validate()?;
        }
        Ok(())
    }
}
