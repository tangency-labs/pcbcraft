use super::*;
use pcbcraft_engine::{PLACE_SYMBOL, schematic::WireEndpoint};
use std::sync::Arc;

/// Click a model point using the live canvas transform and real pointer events.
fn click_point(h: &mut Harness<'_, PcbApp>, point: Point) {
    let pos = screen(h, point);
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// Reproduce the W gesture that used to disappear on completion: the lower-left L has
/// exactly the same routed length as the old upper-right L. Keep the painted new path,
/// remove the branch-free old alternative and undo the entire edit once.
#[test]
fn finishing_a_redraw_keeps_the_path_shown_in_its_preview() {
    let mut h = harness();
    let s = &mut h.state_mut().session;
    s.apply("project.new", &json!({"name":"Redraw"})).unwrap();
    s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":grid(14,12).x_nm,"y_nm":grid(14,12).y_nm,"expected_revision":s.revision()}))
        .unwrap();
    let pin = s.project().unwrap().symbols()[0].terminals()[0];
    s.apply(DRAW_WIRE, &json!({"from":grid(-10,0),"to":WireEndpoint::Terminal(pin).inspect(),"bends":[grid(10,0)],"expected_revision":s.revision()}))
        .unwrap();
    h.state_mut().browser.open = false;
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.key_press(egui::Key::W);
    h.run();
    for point in [grid(-4, 0), grid(-4, 10)] {
        click_point(&mut h, point);
    }
    h.hover_at(screen(&h, grid(10, 10)));
    h.run();
    let expected = vec![grid(-4, 0), grid(-4, 10), grid(10, 10)];
    let points: Vec<_> = expected.iter().map(|&p| screen(&h, p)).collect();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Path(path) if path.stroke.color==egui::epaint::ColorMode::Solid(pcbcraft_chrome::Tokens::get().accent) && path.points==points)));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click_point(&mut h, grid(10, 10));
    assert_eq!(h.state().message, None);
    assert!(routes(&h).contains(&expected));
    assert!(!routes(&h).iter().any(|route| route.contains(&grid(10, 0))));
    let after = h.state().session.snapshot().unwrap();
    crate::tests::click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    crate::tests::click(&mut h, "Redo");
    assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
}
