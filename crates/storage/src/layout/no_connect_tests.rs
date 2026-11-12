use pcbcraft_engine::{
    CONNECT_PINS, PLACE_RESISTOR, Session, nets, no_connect,
    persistence::{encode_documents, encode_packed},
};
use serde_json::json;

/// The actual folder reader must discover intent under both instance vocabularies. Folder
/// recovery and archive reopening retain markers, net identity and repeated-save byte equality.
#[test]
fn no_connect_folder_and_archive_keep_both_instance_vocabularies() {
    for board in [false, true] {
        for registry in [false, true] {
            let mut s = Session::default();
            s.apply("project.new", &json!({"create_board":board})).unwrap();
            for x in [0, 25_400_000] {
                s.apply(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
            }
            s.apply(
                CONNECT_PINS,
                &json!({"from":{"symbol":"symbol-2","terminal":"1"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":s.revision()}),
            )
            .unwrap();
            if registry {
                s.apply(nets::PROMOTE, &json!({"anchor":{"symbol":"symbol-2","terminal":"1"},"expected_revision":s.revision()})).unwrap();
            }
            let before = encode_documents(s.snapshot().unwrap()).unwrap();
            let p = s.snapshot().unwrap();
            let target = p.no_connect_target(p.symbols()[0].terminals()[1]).unwrap();
            s.apply(no_connect::SET, &json!({"target":target,"marked":true,"expected_revision":s.revision()})).unwrap();
            let after = encode_documents(s.snapshot().unwrap()).unwrap();
            for (path, bytes) in &before {
                if path != "project.json" {
                    assert_eq!(&after[path], bytes, "unrelated document {path}");
                }
            }
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("project");
            crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
            let mut checkpoints = Vec::new();
            crate::save(&root, Some(&before), &after, &mut |step| {
                checkpoints.push(step.to_string());
                Ok(())
            })
            .unwrap();
            assert_eq!(crate::load(&root).unwrap(), after);
            assert_eq!(encode_documents(crate::open(&root).unwrap()).unwrap(), after);
            crate::save(&root, Some(&after), &after, &mut |_| Err("No-op staged a write".into())).unwrap();
            let archive = dir.path().join("project.pc");
            std::fs::write(&archive, encode_packed(s.snapshot().unwrap()).unwrap()).unwrap();
            assert_eq!(encode_documents(crate::open(&archive).unwrap()).unwrap(), after);
            for (failed, checkpoint) in checkpoints.iter().enumerate() {
                let root = dir.path().join(format!("interrupted-{failed}"));
                crate::save(&root, None, &before, &mut |_| Ok(())).unwrap();
                let mut step = 0;
                assert!(
                    crate::save(&root, Some(&before), &after, &mut |_| {
                        let stop = step == failed;
                        step += 1;
                        if stop { Err("interrupted".into()) } else { Ok(()) }
                    })
                    .is_err()
                );
                let committed = root.join(".pcbcraft-save").exists() || checkpoint == "completed";
                let recovered = crate::load(&root).unwrap();
                assert_eq!(recovered, if committed { after.clone() } else { before.clone() });
                crate::save(&root, Some(&recovered), &recovered, &mut |_| Err("recovery no-op wrote".into())).unwrap();
            }
        }
    }
}
