//! Native loopback connection with the control token on the first JSON request.

use std::{
    fmt::Write as _,
    io::{self, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    time::{Duration, Instant},
};

use serde_json::Value;
use subtle::ConstantTimeEq as _;

use crate::{Frame, read_frame};

const AUTH_LIMIT: usize = 64 * 1024;
const GENERATED_TOKEN_BYTES: usize = 32;

/// A launch credential. Its text is available for distribution through a private app channel.
pub struct Token(String);

impl Token {
    /// Generate a fresh 256-bit token from the operating system.
    pub fn generate() -> io::Result<Self> {
        let mut bytes = [0; GENERATED_TOKEN_BYTES];
        getrandom::fill(&mut bytes).map_err(|error| io::Error::other(format!("could not generate control token: {error}")))?;
        let mut hex = String::with_capacity(2 * bytes.len());
        for byte in bytes {
            write!(&mut hex, "{byte:02x}").map_err(io::Error::other)?;
        }
        Ok(Self(hex))
    }

    /// Load a private token of at least 32 hexadecimal characters, preserving its case.
    pub fn parse(text: &str) -> io::Result<Self> {
        if text.len() < 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid control token"));
        }
        Ok(Self(text.to_owned()))
    }

    /// The credential to pass to the client through an app-controlled private channel.
    pub fn secret(&self) -> &str {
        &self.0
    }
}

/// A listener restricted to the local IPv4 loopback address.
pub struct Listener(TcpListener);

impl Listener {
    /// Bind to the requested port, or let the OS choose one when given zero.
    pub fn bind(port: u16) -> io::Result<Self> {
        TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map(Self)
    }

    /// The actual bound port, including when the OS picked it.
    pub fn port(&self) -> io::Result<u16> {
        self.0.local_addr().map(|address| address.port())
    }

    /// Make acceptance return `WouldBlock` when no peer is waiting. Accepted streams stay blocking.
    pub fn set_nonblocking(&self, enabled: bool) -> io::Result<()> {
        self.0.set_nonblocking(enabled)
    }

    /// Wait for a peer and assign its I/O timeout. Authenticate it on a worker before dispatching
    /// requests so a slow peer does not hold up acceptance of other connections.
    pub fn accept(&self, timeout: Duration) -> io::Result<Connection> {
        let (socket, _) = self.0.accept()?;
        socket.set_nonblocking(false)?;
        Connection::new(socket, timeout)
    }
}

/// A blocking JSON-line connection. Callers own request IDs and dispatch.
pub struct Connection {
    reader: BufReader<SocketInput>,
    timeout: Duration,
}

impl Connection {
    /// Connect to a local port and send the first request with the token.
    /// The peer's first reply is read with `receive`.
    pub fn connect(port: u16, token: &Token, mut first: Value, timeout: Duration) -> io::Result<Self> {
        let request = first.as_object_mut().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "first control request must be an object"))?;
        request.insert("token".to_owned(), Value::String(token.secret().to_owned()));
        let mut line = serde_json::to_vec(&first).map_err(io::Error::other)?;
        if line.len() > AUTH_LIMIT {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "first control request is too large"));
        }
        line.push(b'\n');
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut connection = Self::new(TcpStream::connect_timeout(&address, timeout)?, timeout)?;
        connection.reader.get_mut().socket.write_all(&line)?;
        Ok(connection)
    }

    /// Authenticate an accepted peer's first request and return it with its token removed.
    /// A stalled request returns `TimedOut` as in [Self::receive].
    /// A failed check must end the connection before any request reaches the app.
    pub fn authenticate(&mut self, token: &Token) -> io::Result<Value> {
        let mut first = self.receive(AUTH_LIMIT)?.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed before authenticating"))?;
        let supplied = first.as_object_mut().and_then(|request| request.remove("token"));
        if !supplied.as_ref().and_then(Value::as_str).is_some_and(|supplied| bool::from(supplied.as_bytes().ct_eq(token.secret().as_bytes()))) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid control token"));
        }
        Ok(first)
    }

    /// Send one JSON value. If writing fails, drop the connection because a partial frame may have reached the peer.
    pub fn send(&mut self, value: &Value) -> io::Result<()> {
        let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
        line.push(b'\n');
        self.reader.get_mut().socket.write_all(&line)
    }

    /// Read one JSON value within the configured timeout or return `None` when the peer closes.
    /// A stalled frame returns `io::ErrorKind::TimedOut` on every platform.
    /// Drop the connection on error because some of the frame may already have been consumed.
    pub fn receive(&mut self, limit: usize) -> io::Result<Option<Value>> {
        self.reader.get_mut().deadline =
            Instant::now().checked_add(self.timeout).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "control timeout is too large"))?;
        // Close on oversized socket input. Stdio can drain a line to recover, but a socket
        // peer could keep sending indefinitely.
        let mut bounded = (&mut self.reader).take(limit.saturating_add(1) as u64);
        match read_frame(&mut bounded, limit)? {
            Some(Frame::Message(bytes)) => serde_json::from_slice(&bytes).map(Some).map_err(io::Error::other),
            Some(Frame::Oversized) => Err(io::Error::new(io::ErrorKind::InvalidData, "control frame is too large")),
            None => Ok(None),
        }
    }

    /// Apply the socket write timeout and create the reader used for per-frame deadlines.
    fn new(socket: TcpStream, timeout: Duration) -> io::Result<Self> {
        socket.set_write_timeout(Some(timeout))?;
        Ok(Self { reader: BufReader::new(SocketInput { socket, deadline: Instant::now() }), timeout })
    }
}

struct SocketInput {
    socket: TcpStream,
    deadline: Instant,
}

impl Read for SocketInput {
    /// Use the remaining frame budget for each socket read so partial input cannot renew it.
    /// Normalize the timeout from this blocking socket to match an expired frame deadline.
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.socket.set_read_timeout(Some(remaining))?;
        match self.socket.read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Err(io::ErrorKind::TimedOut.into()),
            result => result,
        }
    }
}
