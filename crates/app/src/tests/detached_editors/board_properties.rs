//! #281: a detached board view's Properties drafts stay in its own window and revision.

use super::*;

/// The accessible value of a child control, from the child's own accessibility output.
fn child_value(peer: &Rc<RefCell<Peer>>, label: &str) -> Option<String> {
    peer.borrow().nodes.values().find(|node| node.label() == Some(label)).and_then(|node| node.value().map(str::to_owned))
}

/// Type into a child field through its independent input stream, without confirming.
fn child_type(h: &mut Harness<'_, PcbApp>, peer: &Rc<RefCell<Peer>>, label: &str, text: &str) {
    let at = child_rect(peer, label).center();
    child_click_at(h, peer, at);
    peer.borrow_mut().events.push(Event::Text(text.to_owned()));
    h.run();
}

/// Text typed in a detached view's Footprint X is not shown or submitted by the root view of
/// the same footprint, is dropped by a peer's revision, and a fresh entry applies once to both.
#[test]
fn detached_length_drafts_stay_in_their_window_and_revision() {
    let (mut h, peer) = multi_window();
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":revision}),
    );
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let views = h.state().views.hosts[&HostId::ROOT].layout.visible_views();
    let ctx = h.ctx.clone();
    h.state_mut().activate_editor(&ctx, views[1]);
    detach(&mut h);
    assert_eq!(h.state().views.owner(views[1]), Some(HostId(1)));
    let id = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap().id.clone();
    for view in &views {
        h.state_mut().views.view_mut(*view).unwrap().board.selection.insert(id.clone());
    }
    h.run();
    let x = |h: &Harness<'_, PcbApp>| h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap().position.x_nm;

    child_type(&mut h, &peer, "Footprint X", "100 mil");
    assert_eq!(child_value(&peer, "Footprint X").as_deref(), Some("100 mil"));
    assert_eq!(h.get_by_label("Footprint X").value().as_deref(), Some("0 mm"), "the root view shows the model value");
    h.state_mut().session.apply("project.rename", &json!({"name":"Peer revision"})).unwrap();
    h.run();
    let peer_revision = h.state().session.revision();
    child_key(&mut h, &peer, Key::Enter, Modifiers::NONE);
    assert_eq!((x(&h), h.state().session.revision()), (0, peer_revision));
    assert_eq!(child_value(&peer, "Footprint X").as_deref(), Some("0 mm"));

    child_type(&mut h, &peer, "Footprint X", "100 mil");
    child_key(&mut h, &peer, Key::Enter, Modifiers::NONE);
    assert_eq!((x(&h), h.state().session.revision()), (2_540_000, peer_revision + 1));
    focus(&mut h, &peer, false);
    assert_eq!(h.get_by_label("Footprint X").value().as_deref(), Some("2.54 mm"));
    assert_eq!(child_value(&peer, "Footprint X").as_deref(), Some("2.54 mm"));
}
