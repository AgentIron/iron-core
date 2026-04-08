//! ACP transport adapters for `iron-core`.

use crate::connection::{ClientChannel, IronConnection};
use agent_client_protocol as acp;
use agent_client_protocol::Client;
use futures::{
    channel::mpsc::{self, UnboundedReceiver, UnboundedSender},
    future::LocalBoxFuture,
    join, AsyncRead, AsyncWrite, StreamExt,
};
use std::io::{self, Error, ErrorKind};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// Transport families supported by `iron-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// In-process ACP transport used for embeddings and tests.
    InProcess,
    /// ACP transport over stdio.
    Stdio,
    /// ACP transport over TCP.
    Tcp,
}

/// Declares the ACP methods and capabilities supported by this release line.
pub struct AcpSupport {
    /// Supported ACP protocol version.
    pub protocol_version: u16,
    /// Stable methods implemented by the runtime.
    pub stable_methods: &'static [&'static str],
    /// Opt-in unstable methods currently implemented.
    pub unstable_methods: &'static [&'static str],
    /// Deferred methods that are not implemented yet.
    pub deferred_methods: &'static [&'static str],
    /// Optional client capability overrides understood by the runtime.
    pub client_capabilities: &'static [&'static str],
}

/// Public constant describing ACP surface support for this crate version.
pub const ACP_SUPPORT: AcpSupport = AcpSupport {
    protocol_version: 1,
    stable_methods: &[
        "initialize",
        "newSession",
        "prompt",
        "cancel",
        "session/update",
        "requestPermission",
    ],
    unstable_methods: &["closeSession"],
    deferred_methods: &[
        "loadSession",
        "listSessions",
        "forkSession",
        "resumeSession",
        "setSessionConfigOption",
        "authenticate",
        "logout",
    ],
    client_capabilities: &[
        "fs.writeTextFile",
        "fs.readTextFile",
        "terminal/create",
        "terminal/output",
        "terminal/release",
        "terminal/waitForExit",
        "terminal/kill",
    ],
};

/// Metadata about a configured transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportMetadata {
    /// The transport family.
    pub kind: TransportKind,
}

impl TransportMetadata {
    /// Create transport metadata for the given kind.
    pub fn new(kind: TransportKind) -> Self {
        Self { kind }
    }
}

#[allow(dead_code)]
const TRANSPORT_CONSISTENCY_NOTE: &str = "\
All transports route through the same ACP RPC layer (AgentSideConnection / \
ClientSideConnection), which enforces identical session ownership, durable \
timeline, permission flow, and cancellation semantics. The transport only \
affects how bytes move between the agent and client sides — it does not \
change runtime/session ownership, durable history behavior, permission \
mediation, or cancellation outcomes.";

struct PipeWrite {
    tx: UnboundedSender<Vec<u8>>,
}

struct PipeRead {
    rx: UnboundedReceiver<Vec<u8>>,
    buffer: Vec<u8>,
}

fn create_pipe() -> (PipeWrite, PipeRead) {
    let (tx, rx) = mpsc::unbounded();
    (
        PipeWrite { tx },
        PipeRead {
            rx,
            buffer: Vec::new(),
        },
    )
}

impl AsyncWrite for PipeWrite {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.tx.unbounded_send(buf.to_vec()) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(_) => Poll::Ready(Err(Error::new(ErrorKind::BrokenPipe, "channel closed"))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.tx.close_channel();
        Poll::Ready(Ok(()))
    }
}

impl AsyncRead for PipeRead {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        if !this.buffer.is_empty() {
            let n = buf.len().min(this.buffer.len());
            buf[..n].copy_from_slice(&this.buffer[..n]);
            this.buffer.drain(..n);
            return Poll::Ready(Ok(n));
        }

        match this.rx.poll_next_unpin(cx) {
            Poll::Ready(Some(data)) => {
                let n = buf.len().min(data.len());
                buf[..n].copy_from_slice(&data[..n]);
                if n < data.len() {
                    this.buffer.extend_from_slice(&data[n..]);
                }
                Poll::Ready(Ok(n))
            }
            Poll::Ready(None) => Poll::Ready(Ok(0)),
            Poll::Pending => Poll::Pending,
        }
    }
}

struct AcpClientChannel {
    inner: Rc<acp::AgentSideConnection>,
}

impl ClientChannel for AcpClientChannel {
    fn send_notification(
        &self,
        notification: acp::SessionNotification,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = agent_client_protocol::Result<()>>>>
    {
        let inner = self.inner.clone();
        Box::pin(async move { inner.session_notification(notification).await })
    }

    fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                Output = agent_client_protocol::Result<
                    agent_client_protocol::RequestPermissionResponse,
                >,
            >,
        >,
    > {
        let inner = self.inner.clone();
        Box::pin(async move { inner.request_permission(request).await })
    }
}

/// In-process ACP transport pairing a client-side connection with an IO driver.
pub struct InProcessTransport {
    client: acp::ClientSideConnection,
}

impl InProcessTransport {
    /// Borrow the ACP client-side connection.
    pub fn client(&self) -> &acp::ClientSideConnection {
        &self.client
    }
}

/// Create an in-process ACP transport and its IO driver future.
pub fn create_in_process_transport<C>(
    runtime: crate::runtime::IronRuntime,
    client_handler: C,
) -> (
    InProcessTransport,
    impl std::future::Future<Output = ()> + 'static,
)
where
    C: acp::Client + 'static,
{
    let (c2a_write, c2a_read) = create_pipe();
    let (a2c_write, a2c_read) = create_pipe();

    let iron_conn = Rc::new(IronConnection::new(runtime));

    let spawn_fn = |fut: LocalBoxFuture<'static, ()>| {
        tokio::task::spawn_local(fut);
    };

    let (agent_side, agent_io) =
        acp::AgentSideConnection::new(iron_conn.clone(), a2c_write, c2a_read, spawn_fn);

    let acp_client = Rc::new(AcpClientChannel {
        inner: Rc::new(agent_side),
    });
    iron_conn.set_client(acp_client);

    let (client_conn, client_io) =
        acp::ClientSideConnection::new(client_handler, c2a_write, a2c_read, spawn_fn);

    let io_driver = async move {
        let _ = join!(agent_io, client_io);
    };

    (
        InProcessTransport {
            client: client_conn,
        },
        io_driver,
    )
}

impl std::fmt::Debug for InProcessTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessTransport").finish()
    }
}

/// Create an ACP agent bound to stdio.
pub fn create_stdio_agent(
    runtime: crate::runtime::IronRuntime,
) -> (
    Rc<IronConnection>,
    impl std::future::Future<Output = acp::Result<()>> + 'static,
) {
    let outgoing = tokio::io::stdout().compat_write();
    let incoming = tokio::io::stdin().compat();

    let iron_conn = Rc::new(IronConnection::new(runtime));

    let spawn_fn = |fut: LocalBoxFuture<'static, ()>| {
        tokio::task::spawn_local(fut);
    };

    let (agent_side, agent_io) =
        acp::AgentSideConnection::new(iron_conn.clone(), outgoing, incoming, spawn_fn);

    let acp_client = Rc::new(AcpClientChannel {
        inner: Rc::new(agent_side),
    });
    iron_conn.set_client(acp_client);

    (iron_conn, agent_io)
}

/// Serve ACP connections over TCP at the provided socket address.
pub async fn serve_tcp_agent(
    runtime: crate::runtime::IronRuntime,
    addr: std::net::SocketAddr,
) -> acp::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| acp::Error::into_internal_error(&e))?;

    loop {
        let (stream, _peer) = listener
            .accept()
            .await
            .map_err(|e| acp::Error::into_internal_error(&e))?;

        let (read_half, write_half) = tokio::io::split(stream);
        let outgoing: tokio_util::compat::Compat<tokio::io::WriteHalf<tokio::net::TcpStream>> =
            write_half.compat_write();
        let incoming: tokio_util::compat::Compat<tokio::io::ReadHalf<tokio::net::TcpStream>> =
            read_half.compat();

        let rt = runtime.clone();
        std::thread::spawn(move || {
            let thread_rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create per-connection runtime");
            let local_set = tokio::task::LocalSet::new();
            local_set.block_on(&thread_rt, async move {
                let iron_conn = Rc::new(IronConnection::new(rt));

                let spawn_fn = |fut: LocalBoxFuture<'static, ()>| {
                    tokio::task::spawn_local(fut);
                };

                let (agent_side, agent_io) =
                    acp::AgentSideConnection::new(iron_conn.clone(), outgoing, incoming, spawn_fn);

                let acp_client = Rc::new(AcpClientChannel {
                    inner: Rc::new(agent_side),
                });
                iron_conn.set_client(acp_client);

                let _ = agent_io.await;
            });
        });
    }
}

pub async fn connect_tcp_client<C>(
    client_handler: C,
    addr: std::net::SocketAddr,
) -> acp::Result<(
    acp::ClientSideConnection,
    impl std::future::Future<Output = acp::Result<()>> + 'static,
)>
where
    C: acp::Client + 'static,
{
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .map_err(|e| acp::Error::into_internal_error(&e))?;

    let (read_half, write_half) = tokio::io::split(stream);
    let outgoing = write_half.compat_write();
    let incoming = read_half.compat();

    let spawn_fn = |fut: LocalBoxFuture<'static, ()>| {
        tokio::task::spawn_local(fut);
    };

    Ok(acp::ClientSideConnection::new(
        client_handler,
        outgoing,
        incoming,
        spawn_fn,
    ))
}
