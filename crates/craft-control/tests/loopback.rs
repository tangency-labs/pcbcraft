#![cfg(not(target_arch = "wasm32"))]

use std::{
    io::{self, Write},
    net::{Ipv4Addr, TcpStream},
    thread,
    time::Duration,
};

use craft_control::loopback::{Connection, Listener, Token};
use serde_json::json;

const TIMEOUT: Duration = Duration::from_secs(2);

/// A real socket carries the authenticated first request and ordinary subsequent requests.
#[test]
fn authenticates_then_exchanges_frames() -> io::Result<()> {
    let token = Token::generate()?;
    let listener = Listener::bind(0)?;
    let port = listener.port()?;
    thread::scope(|scope| -> io::Result<()> {
        let mut client = Connection::connect(port, &token, json!({"id": 1, "method": "inspect"}), TIMEOUT)?;
        let server = scope.spawn(|| -> io::Result<()> {
            let mut peer = listener.accept(TIMEOUT)?;
            assert_eq!(peer.authenticate(&token)?, json!({"id": 1, "method": "inspect"}));
            peer.send(&json!({"id": 1, "ok": true}))?;
            assert_eq!(peer.receive(256)?, Some(json!({"id": 2, "method": "execute"})));
            Ok(())
        });
        assert_eq!(client.receive(256)?, Some(json!({"id": 1, "ok": true})));
        client.send(&json!({"id": 2, "method": "execute"}))?;
        server.join().map_err(|_| io::Error::other("server thread panicked"))?
    })
}

/// A different token is refused and a later connection can still authenticate.
#[test]
fn rejects_wrong_token_without_poisoning_listener() -> io::Result<()> {
    let token = Token::generate()?;
    let other = Token::generate()?;
    let listener = Listener::bind(0)?;
    let port = listener.port()?;

    let _wrong = Connection::connect(port, &other, json!({"id": 1}), TIMEOUT)?;
    let mut rejected = listener.accept(TIMEOUT)?;
    assert!(matches!(rejected.authenticate(&token), Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
    drop(rejected);

    let _correct = Connection::connect(port, &token, json!({"id": 2}), TIMEOUT)?;
    let mut accepted = listener.accept(TIMEOUT)?;
    assert_eq!(accepted.authenticate(&token)?, json!({"id": 2}));
    Ok(())
}

/// A peer that exceeds the frame limit is refused even if it keeps the socket open.
#[test]
fn socket_does_not_wait_for_newline_on_oversized_input() -> io::Result<()> {
    let listener = Listener::bind(0)?;
    let mut sender = TcpStream::connect((Ipv4Addr::LOCALHOST, listener.port()?))?;
    sender.write_all(b"12345")?;
    let mut peer = listener.accept(TIMEOUT)?;
    assert!(matches!(peer.receive(4), Err(error) if error.kind() == io::ErrorKind::InvalidData));
    Ok(())
}
