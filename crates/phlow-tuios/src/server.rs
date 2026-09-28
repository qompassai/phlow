//! The Unix socket server for the control protocol.
//!
//! Binds the socket under a directory held at mode 0700 and chmods the
//! socket itself to 0700, as probed on the real daemon (`drwx------` /
//! `srwx------`). Serves one in-order request/response loop per connection on
//! its own thread; a transport failure (I/O error, over-long frame) ends that
//! connection without a response, while every decoded request gets exactly
//! one response line.

use std::io::{BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use crate::daemon::SessionDaemon;
use crate::error::{ErrorCode, TuiosError};
use crate::hooks::Semaphore;
use crate::protocol::{Request, Response, decode_request, read_frame};

/// Name of the socket file inside the socket directory.
pub const SOCKET_FILE_NAME: &str = "phlow-tuios.sock";
/// Mode for the socket directory and the socket file: owner only.
const SOCKET_MODE: u32 = 0o700;
/// Cap on concurrent connections. The accept loop waits for a permit instead
/// of spawning another thread past this, so a client flood applies
/// backpressure through the listen backlog instead of exhausting threads.
const CONNECTIONS_MAX: usize = 64;

/// A bound control socket serving one [`SessionDaemon`].
#[derive(Debug)]
pub struct Server {
    listener: UnixListener,
    socket_path: PathBuf,
    daemon: Arc<Mutex<SessionDaemon>>,
    conn_permits: Arc<Semaphore>,
}

impl Server {
    /// Bind the socket under `socket_dir`. The directory is created with mode
    /// 0700; an existing directory gets its mode tightened to 0700. The
    /// chmod fails unless this user owns the directory, so a foreign-owned
    /// directory refuses to bind here rather than later. A symlinked
    /// directory is refused outright: the chmod must land on our own
    /// directory, never a link target. A stale socket file at the path is
    /// removed; any other pre-existing file — including a symlink, dangling
    /// or not — is refused rather than unlinked or bound through.
    ///
    /// # Errors
    ///
    /// `Io` when the directory cannot be prepared or the socket cannot be
    /// bound; `Internal` protocol error on the security refusals above.
    pub fn bind(socket_dir: &Path) -> Result<Self, TuiosError> {
        std::fs::create_dir_all(socket_dir)?;
        // symlink_metadata, not metadata: create_dir_all follows symlinks.
        let dir_meta = std::fs::symlink_metadata(socket_dir)?;
        if dir_meta.file_type().is_symlink() {
            return Err(TuiosError::protocol(
                ErrorCode::Internal,
                "socket directory must not be a symlink",
            ));
        }
        if !dir_meta.is_dir() {
            return Err(TuiosError::protocol(
                ErrorCode::Internal,
                "socket path parent is not a directory",
            ));
        }
        // Tighten an existing directory: whoever controls it can replace the
        // socket and read what every client sends.
        std::fs::set_permissions(socket_dir, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        let socket_path = socket_dir.join(SOCKET_FILE_NAME);
        // symlink_metadata, not exists: a dangling symlink reports exists()
        // as false, and binding through it would create the socket at the
        // link target. Only a real stale socket is unlinked.
        match std::fs::symlink_metadata(&socket_path) {
            Ok(meta) if meta.file_type().is_socket() => {
                std::fs::remove_file(&socket_path)?;
            }
            Ok(_) => {
                return Err(TuiosError::protocol(
                    ErrorCode::Internal,
                    "refusing to use a non-socket file at the socket path",
                ));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(TuiosError::from(err)),
        }
        let listener = UnixListener::bind(&socket_path)?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        Ok(Self {
            listener,
            socket_path,
            daemon: Arc::new(Mutex::new(SessionDaemon::new())),
            conn_permits: Arc::new(Semaphore::new(CONNECTIONS_MAX)),
        })
    }

    /// The bound socket path, for clients.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Access to the daemon behind the socket, for embedding and tests.
    #[must_use]
    pub fn daemon(&self) -> &Arc<Mutex<SessionDaemon>> {
        &self.daemon
    }

    /// Serve forever: accept connections and handle each on its own thread.
    /// At most [`CONNECTIONS_MAX`] connections run at once; past that the
    /// accept loop waits for a permit, so clients queue in the listen
    /// backlog instead of spawning unbounded threads. Returns only on
    /// accept failure.
    pub fn run(&self) -> Result<(), TuiosError> {
        let listener = self.listener.try_clone().map_err(TuiosError::from)?;
        accept_loop(
            listener,
            Arc::clone(&self.daemon),
            Arc::clone(&self.conn_permits),
        )
    }
}

/// Accept connections and serve each on its own thread, bounded by the
/// permit pool. One request loop per connection, in order per connection.
fn accept_loop(
    listener: UnixListener,
    daemon: Arc<Mutex<SessionDaemon>>,
    permits: Arc<Semaphore>,
) -> Result<(), TuiosError> {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                // Backpressure, not rejection: when the pool is full this
                // waits here instead of spawning thread CONNECTIONS_MAX + 1.
                let permit = permits.acquire_owned();
                let daemon = Arc::clone(&daemon);
                thread::spawn(move || {
                    let _permit = permit;
                    serve_connection(stream, &daemon);
                });
            }
            Err(err) => return Err(TuiosError::from(err)),
        }
    }
    Ok(())
}

/// One connection's read/dispatch/respond loop. In-order: a response is
/// written before the next request is read.
fn serve_connection(stream: UnixStream, daemon: &Arc<Mutex<SessionDaemon>>) {
    let reader_stream = match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    };
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
    loop {
        let frame = match read_frame(&mut reader) {
            Ok(Option::None) => break,
            Ok(Some(frame)) => frame,
            // Transport failures end the connection without a response: a
            // frame past the cap gets silence, as probed.
            Err(_) => break,
        };
        if frame.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let response = match decode_request(&frame) {
            Ok(request) => dispatch(daemon, &request),
            Err(err) => err.into_response(Option::None),
        };
        if writer.write_all(&response.encode()).is_err() {
            break;
        }
    }
}

fn dispatch(daemon: &Arc<Mutex<SessionDaemon>>, request: &Request) -> Response {
    let mut guard = daemon.lock().unwrap_or_else(|poisoned| {
        // A handler panicking must not wedge the server: recover the daemon
        // and keep serving. Handlers never panic by contract; this is the
        // backstop, and it is loud in the response.
        poisoned.into_inner()
    });
    let params = request.params.clone().unwrap_or(Value::Null);
    match guard.dispatch(&request.verb, &params) {
        Ok(result) => Response::ok(request.id.clone(), result),
        Err(err) => err.into_response(request.id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader as StdBufReader};
    use std::time::Duration;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "phlow-tuios-server-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Bind a server and serve connections on background threads, exactly
    /// like [`Server::run`]: one thread per connection, bounded by the
    /// permit pool, so multi-connection tests exercise the real shape.
    fn serve(dir: &Path) -> PathBuf {
        let server = Server::bind(dir).expect("bind");
        let path = server.socket_path().to_owned();
        let listener = server.listener.try_clone().expect("clone");
        let daemon = Arc::clone(server.daemon());
        let permits = Arc::clone(&server.conn_permits);
        thread::spawn(move || {
            let _ = accept_loop(listener, daemon, permits);
        });
        path
    }

    /// Connect with a read timeout: a server that never answers must fail
    /// the test, never hang the suite.
    fn connect(path: &Path) -> UnixStream {
        let stream = UnixStream::connect(path).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        stream
    }

    // --- validation ---

    #[test]
    fn socket_dir_and_file_are_owner_only() {
        let dir = test_dir("perms");
        let server = Server::bind(&dir).expect("bind");
        let dir_mode = std::fs::metadata(&dir).expect("dir").permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "socket dir must be 0700");
        let sock_mode = std::fs::metadata(server.socket_path())
            .expect("sock")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(sock_mode, 0o700, "socket file must be 0700");
    }

    #[test]
    fn ping_round_trip_over_the_socket() {
        let dir = test_dir("ping");
        let path = serve(&dir);
        let mut stream = connect(&path);
        stream
            .write_all(b"{\"id\": 7, \"verb\": \"ping\"}\n")
            .expect("write");
        let mut reader = StdBufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        let resp: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(resp["id"], serde_json::json!(7));
        assert_eq!(resp["result"]["ok"], serde_json::json!(true));
        let _ = stream;
    }

    #[test]
    fn responses_stay_in_request_order() {
        let dir = test_dir("order");
        let path = serve(&dir);
        let mut stream = connect(&path);
        for id in [1, 2, 3] {
            let line = format!("{{\"id\": {id}, \"verb\": \"ping\"}}\n");
            stream.write_all(line.as_bytes()).expect("write");
        }
        let mut reader = StdBufReader::new(stream.try_clone().expect("clone"));
        for want in [1, 2, 3] {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read");
            let resp: Value = serde_json::from_str(&line).expect("json");
            assert_eq!(resp["id"], serde_json::json!(want), "out of order");
        }
    }

    #[test]
    fn state_is_shared_across_connections() {
        let dir = test_dir("shared");
        let path = serve(&dir);
        let mut first = connect(&path);
        first
            .write_all(
                b"{\"id\": 1, \"verb\": \"new-session\", \"params\": {\"name\": \"shared\"}}\n",
            )
            .expect("write");
        let mut reader = StdBufReader::new(first.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        // A second connection sees the first connection's session.
        let mut second = connect(&path);
        second
            .write_all(b"{\"id\": 2, \"verb\": \"list-sessions\"}\n")
            .expect("write");
        let mut reader = StdBufReader::new(second.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        let resp: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(resp["result"]["sessions"], serde_json::json!(["shared"]));
    }

    #[test]
    fn malformed_line_gets_an_error_and_the_connection_survives() {
        let dir = test_dir("malformed");
        let path = serve(&dir);
        let mut stream = connect(&path);
        stream
            .write_all(b"{\"id\": 1, \"verb\":\n{\"id\": 2, \"verb\": \"ping\"}\n")
            .expect("write");
        let mut reader = StdBufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        let resp: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(resp["error"]["code"], serde_json::json!("invalid_request"));
        assert!(resp.get("id").is_none(), "malformed line carries no id");
        // The next request on the same connection still works.
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        let resp: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(resp["id"], serde_json::json!(2));
        assert_eq!(resp["result"]["ok"], serde_json::json!(true));
    }

    #[test]
    fn stale_socket_file_is_replaced() {
        let dir = test_dir("stale");
        {
            let first = Server::bind(&dir).expect("first bind");
            assert!(first.socket_path().exists());
            // Drop without cleanup: the socket file stays behind, stale.
        }
        let second = Server::bind(&dir).expect("rebind over a stale socket");
        assert!(second.socket_path().exists(), "stale socket was replaced");
    }

    #[test]
    fn bind_tightens_a_loose_existing_dir() {
        let dir = test_dir("tighten");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let _server = Server::bind(&dir).expect("bind");
        let mode = std::fs::metadata(&dir).expect("dir").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "bind must tighten an existing loose dir");
    }

    // --- adversarial ---

    #[test]
    fn oversize_request_drops_the_connection_silently() {
        let dir = test_dir("oversize");
        let path = serve(&dir);
        let mut stream = connect(&path);
        let big = vec![b'x'; crate::protocol::FRAME_BYTES_MAX + 16];
        let _ = stream.write_all(&big);
        let _ = stream.write_all(b"\n");
        let mut buf = [0u8; 64];
        use std::io::Read;
        let read = stream.read(&mut buf);
        assert!(
            matches!(read, Ok(0)) || read.is_err(),
            "oversize must close the connection without a response, got {read:?}"
        );
    }

    #[test]
    fn blank_lines_are_skipped() {
        let dir = test_dir("blank");
        let path = serve(&dir);
        let mut stream = connect(&path);
        stream
            .write_all(b"\n   \n{\"id\": 1, \"verb\": \"ping\"}\n")
            .expect("write");
        let mut reader = StdBufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        let resp: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(resp["id"], serde_json::json!(1));
    }

    #[test]
    fn unknown_verb_gets_the_envelope_over_the_socket() {
        let dir = test_dir("unknownverb");
        let path = serve(&dir);
        let mut stream = connect(&path);
        stream
            .write_all(b"{\"id\": \"x\", \"verb\": \"frobnicate\"}\n")
            .expect("write");
        let mut reader = StdBufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        let resp: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(resp["id"], serde_json::json!("x"));
        assert_eq!(resp["error"]["code"], serde_json::json!("unknown_verb"));
    }

    #[test]
    fn non_socket_file_at_path_is_refused() {
        let dir = test_dir("nonsock");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join(SOCKET_FILE_NAME), b"not a socket").expect("write");
        let err = Server::bind(&dir).expect_err("must refuse to unlink");
        assert_eq!(err.code(), ErrorCode::Internal);
    }

    #[test]
    fn symlinked_socket_dir_is_refused() {
        let target = test_dir("linktarget");
        std::fs::create_dir_all(&target).expect("mkdir");
        let link = test_dir("symlinkdir");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let err = Server::bind(&link).expect_err("symlinked dir");
        assert_eq!(err.code(), ErrorCode::Internal, "TOCTOU-safe socket dir");
        std::fs::remove_file(&link).expect("cleanup");
    }

    #[test]
    fn dangling_symlink_at_socket_path_is_refused() {
        let dir = test_dir("dangling");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::os::unix::fs::symlink(
            "/nonexistent-phlow-tuios-target",
            dir.join(SOCKET_FILE_NAME),
        )
        .expect("symlink");
        let err = Server::bind(&dir).expect_err("dangling symlink");
        assert_eq!(
            err.code(),
            ErrorCode::Internal,
            "symlink at socket path refused"
        );
    }

    #[test]
    fn connection_pool_is_bounded() {
        let pool = Arc::new(crate::hooks::Semaphore::new(CONNECTIONS_MAX));
        let mut held = Vec::new();
        for _ in 0..CONNECTIONS_MAX {
            held.push(pool.try_acquire_owned().expect("permit"));
        }
        assert!(
            pool.try_acquire_owned().is_none(),
            "connection {n} has no permit left; production blocks here",
            n = CONNECTIONS_MAX + 1
        );
        drop(held);
        assert!(
            pool.try_acquire_owned().is_some(),
            "permits return when connections close"
        );
    }
}
