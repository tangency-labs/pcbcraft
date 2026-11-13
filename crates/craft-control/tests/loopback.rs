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

/// An open peer that stalls mid-request produces the same timeout kind on every platform.
#[test]
fn stalled_first_request_reports_timed_out() -> io::Result<()> {
    let listener = Listener::bind(0)?;
    let token = Token::generate()?;
    let mut sender = TcpStream::connect((Ipv4Addr::LOCALHOST, listener.port()?))?;
    sender.write_all(b"{\"id\":")?;
    let mut peer = listener.accept(Duration::from_millis(100))?;

    let result = peer.authenticate(&token);
    assert!(matches!(&result, Err(error) if error.kind() == io::ErrorKind::TimedOut), "{result:?}");
    Ok(())
}

/// Single-request peers reject both reader-buffered and subsequently arriving bytes without
/// changing the ordinary multi-frame connection's shutdown semantics.
#[test]
fn single_request_rejects_buffered_and_socket_trailing_input() -> io::Result<()> {
    for buffered in [false, true] {
        let listener = Listener::bind(0)?;
        let mut sender = TcpStream::connect((Ipv4Addr::LOCALHOST, listener.port()?))?;
        if buffered {
            sender.write_all(b"{}\nx")?;
        } else {
            sender.write_all(b"{}\n")?;
        }
        let mut peer = listener.accept(TIMEOUT)?;
        assert_eq!(peer.receive(16)?, Some(json!({})));
        if !buffered {
            assert!(!peer.peer_closed_or_pending_input()?);
            sender.write_all(b"x")?;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !peer.peer_closed_or_pending_input()? {
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
        assert!(!peer.peer_closed()?, "ordinary multi-frame clients can still receive trailing input");
        drop(sender);
        assert!(peer.peer_closed_or_pending_input()?);
    }
    Ok(())
}

/// Observing a peer shutdown never waits for the socket's two-second frame timeout.
#[test]
fn observes_disconnect_without_waiting_for_a_frame() -> io::Result<()> {
    let listener = Listener::bind(0)?;
    let sender = TcpStream::connect((Ipv4Addr::LOCALHOST, listener.port()?))?;
    let peer = listener.accept(TIMEOUT)?;
    assert!(!peer.peer_closed()?);
    drop(sender);
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !peer.peer_closed()? {
        assert!(std::time::Instant::now() < deadline);
        thread::yield_now();
    }
    Ok(())
}
