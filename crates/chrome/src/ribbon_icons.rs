//! Bounded SVG adapter ported from PCBcraft Next's qualified ribbon icon consumer.
//! Masters, exact pins, hashes and retained licenses live in assets/ribbon.

use crate::{Tokens, workspace::ToolIcon};
use egui::{Color32, Painter, Pos2, Rect, Vec2};
use std::hash::{Hash, Hasher};

/// The shared ribbon reserves an icon slot while PCBcraft paints its qualified command glyph.
pub(crate) const HOST_PAINTED: &str = "pcbcraft-host-painted";

/// Extend the shared icon set with an empty, explicitly registered host-painted slot. The
/// pinned ribbon always draws an icon; registering this slot avoids its unknown-icon marker.
pub(crate) fn install() {
    use solidcraft_ui_kit::icons::{IconExtension, IconSet};
    static ICONS: IconSet = IconSet::extended(
        solidcraft_look::icons::DATA,
        IconExtension {
            masters: &[(HOST_PAINTED, "flat", "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"></svg>")],
            ..IconExtension::NONE
        },
    );
    solidcraft_ui_kit::icons::install(&ICONS);
}

/// Return a registered master; other existing chrome glyphs retain their native painter.
fn source(icon: ToolIcon) -> Option<(&'static str, &'static str)> {
    macro_rules! svg {
        ($name:literal) => {
            Some(($name, include_str!(concat!("../assets/ribbon/", $name, ".svg"))))
        };
    }
    match icon {
        ToolIcon::Sheet => svg!("sheet"),
        ToolIcon::Subsheet => svg!("subsheet"),
        ToolIcon::HierarchicalLabel => svg!("port"),
        ToolIcon::Text => svg!("text"),
        ToolIcon::Power => svg!("power"),
        ToolIcon::Symbol => svg!("place-symbol"),
        ToolIcon::Wire => svg!("wire"),
        ToolIcon::Bus => svg!("bus"),
        ToolIcon::NetLabel => svg!("net-label"),
        ToolIcon::Move => svg!("move"),
        ToolIcon::Rotate => svg!("spin"),
        ToolIcon::Mirror => svg!("mirror"),
        ToolIcon::Rules => svg!("rules"),
        ToolIcon::Erc => svg!("erc"),
        ToolIcon::Annotate => svg!("list"),
        ToolIcon::Bom => svg!("configuration_table"),
        ToolIcon::Sync => svg!("sync"),
        _ => None,
    }
}

/// Two-stage role substitution preserves source artwork and prevents palette colors cascading
/// into other source roles. Mask paints remain untouched.
fn resolved(svg: &str, t: Tokens) -> String {
    let roles = [
        ("currentColor", t.icon_line),
        ("#34465b", t.icon_ink),
        ("#e2e8ef", t.icon_face_1),
        ("#b8c3cf", t.icon_face_2),
        ("#8e9faf", t.icon_face_3),
        ("#b9ddf8", t.icon_active_1),
        ("#328bd3", t.icon_active_2),
        ("#236ead", t.icon_active_3),
        ("#6f879c", t.text_dim),
    ];
    let mut result = svg.to_owned();
    for (index, (source, _)) in roles.iter().enumerate() {
        result = result.replace(source, &format!("__pcbcraft_role_{index}__"));
    }
    for (index, (_, color)) in roles.iter().enumerate() {
        result = result.replace(&format!("__pcbcraft_role_{index}__"), &format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b()));
    }
    result
}

/// Paint a registered immutable master using an in-memory cache keyed by its full palette.
/// SVG decoding handles only embedded assets; this installs no file or HTTP loader.
pub(crate) fn paint(p: &Painter, icon: ToolIcon, center: Pos2, size: f32, t: Tokens, enabled: bool) -> bool {
    let Some((name, svg)) = source(icon) else {
        return false;
    };
    egui_extras::install_image_loaders(p.ctx());
    let svg = resolved(svg, t);
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    svg.hash(&mut hash);
    let image = egui::Image::from_bytes(format!("bytes://pcbcraft/ribbon/{name}-{:016x}.svg", hash.finish()), svg.into_bytes())
        .fit_to_exact_size(Vec2::splat(size));
    if let Ok(egui::load::TexturePoll::Ready { texture }) = image.load_for_size(p.ctx(), Vec2::splat(size)) {
        p.image(
            texture.id,
            Rect::from_center_size(center, Vec2::splat(size)),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
            if enabled { Color32::WHITE } else { Color32::from_white_alpha(85) },
        );
    }
    true
}

/// Retain the source notices with embedded artwork in native and browser binaries.
pub(crate) const NOTICES: &str = concat!(
    include_str!("../assets/ribbon/README.md"),
    "\n\n",
    include_str!("../assets/ribbon/PCBCraft-Next-LICENSE-MIT"),
    "\n\n",
    include_str!("../assets/ribbon/PCBCraft-Next-LICENSE-APACHE"),
    "\n\n",
    include_str!("../assets/ribbon/SOLIDCRAFT-LICENSE-MIT"),
    "\n\n",
    include_str!("../assets/ribbon/SOLIDCRAFT-LICENSE-APACHE"),
    "\n\n",
    include_str!("../assets/ribbon/SOLIDCRAFT-NOTICE"),
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Every embedded master must decode visibly in both palettes through the production loader.
    #[test]
    fn registered_artwork_decodes_in_both_palettes() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        for (theme, tokens) in [("light", Tokens::light()), ("dark", Tokens::dark())] {
            for icon in [
                ToolIcon::Sheet,
                ToolIcon::Subsheet,
                ToolIcon::HierarchicalLabel,
                ToolIcon::Text,
                ToolIcon::Power,
                ToolIcon::Symbol,
                ToolIcon::Wire,
                ToolIcon::Bus,
                ToolIcon::NetLabel,
                ToolIcon::Move,
                ToolIcon::Rotate,
                ToolIcon::Mirror,
                ToolIcon::Rules,
                ToolIcon::Erc,
                ToolIcon::Annotate,
                ToolIcon::Bom,
                ToolIcon::Sync,
            ] {
                let Some((name, svg)) = source(icon) else { panic!("unregistered icon {icon:?}") };
                let uri = format!("bytes://test/{theme}/{name}.svg");
                ctx.include_bytes(uri.clone(), resolved(svg, tokens).into_bytes());
                let Ok(egui::load::ImagePoll::Ready { image }) = ctx.try_load_image(&uri, egui::load::SizeHint::Width(48)) else {
                    panic!("failed to decode {theme}/{name}");
                };
                assert_eq!(image.size, [48, 48]);
                assert!(image.pixels.iter().any(|pixel| pixel.a() > 0), "empty {theme}/{name}");
            }
        }
    }

    /// A palette value equal to another source role must not cascade; mask paints stay literal.
    #[test]
    fn palette_substitution_preserves_roles_and_masks() {
        let mut tokens = Tokens::dark();
        tokens.icon_line = Color32::from_rgb(0x34, 0x46, 0x5b);
        tokens.icon_ink = Color32::from_rgb(1, 2, 3);
        assert_eq!(resolved("currentColor #34465b #fff #000", tokens), "#34465b #010203 #fff #000",);
        assert_ne!(resolved("currentColor", Tokens::dark()), resolved("currentColor", Tokens::light()));
    }
}
