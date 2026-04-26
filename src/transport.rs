//! ACP transport adapters for `iron-core`.

use crate::connection::{ClientChannel, IronConnection};
use agent_client_protocol as acp;
use agent_client_protocol::schema as acp_schema;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

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
All transports route through the same ACP RPC layer, which enforces identical \
session ownership, durable timeline, permission flow, and cancellation \
semantics. The transport only affects how bytes move between the agent and \
client sides - it does not change runtime/session ownership, durable history \
behavior, permission mediation, or cancellation outcomes.";

pub trait InProcessClientHandler: 'static {
    fn session_notification(
        &self,
        notification: acp_schema::SessionNotification,
    ) -> Pin<Box<dyn Future<Output = agent_client_protocol::Result<()>>>>;

    fn request_permission(
        &self,
        request: acp_schema::RequestPermissionRequest,
    ) -> Pin<
        Box<
            dyn Future<
                Output = agent_client_protocol::Result<acp_schema::RequestPermissionResponse>,
            >,
        >,
    >;
}

struct LocalClientChannel<H> {
    handler: Rc<H>,
}

impl<H> LocalClientChannel<H> {
    fn new(handler: Rc<H>) -> Self {
        Self { handler }
    }
}

impl<H> ClientChannel for LocalClientChannel<H>
where
    H: InProcessClientHandler,
{
    fn send_notification(
        &self,
        notification: acp_schema::SessionNotification,
    ) -> Pin<Box<dyn Future<Output = agent_client_protocol::Result<()>>>> {
        self.handler.session_notification(notification)
    }

    fn request_permission(
        &self,
        request: acp_schema::RequestPermissionRequest,
    ) -> Pin<
        Box<
            dyn Future<
                Output = agent_client_protocol::Result<acp_schema::RequestPermissionResponse>,
            >,
        >,
    > {
        self.handler.request_permission(request)
    }
}

/// In-process ACP transport pairing a local client wrapper with the agent runtime.
pub struct InProcessTransport {
    client: InProcessClient,
}

impl InProcessTransport {
    /// Borrow the in-process ACP client wrapper.
    pub fn client(&self) -> &InProcessClient {
        &self.client
    }
}

/// Local ACP client facade used by the in-process transport.
pub struct InProcessClient {
    connection: Rc<IronConnection>,
}

impl InProcessClient {
    async fn with_client_channel<T>(
        &self,
        op: impl AsyncFnOnce() -> agent_client_protocol::Result<T>,
    ) -> agent_client_protocol::Result<T> {
        op().await
    }

    pub async fn initialize(
        &self,
        request: acp_schema::InitializeRequest,
    ) -> agent_client_protocol::Result<acp_schema::InitializeResponse> {
        self.connection.handle_initialize(request).await
    }

    pub async fn new_session(
        &self,
        request: acp_schema::NewSessionRequest,
    ) -> agent_client_protocol::Result<acp_schema::NewSessionResponse> {
        self.connection.handle_new_session(request).await
    }

    pub async fn prompt(
        &self,
        request: acp_schema::PromptRequest,
    ) -> agent_client_protocol::Result<acp_schema::PromptResponse> {
        self.with_client_channel(|| self.connection.handle_prompt(request))
            .await
    }

    pub async fn cancel(
        &self,
        notification: acp_schema::CancelNotification,
    ) -> agent_client_protocol::Result<()> {
        self.connection.handle_cancel(notification).await
    }

    pub async fn close_session(
        &self,
        request: acp_schema::CloseSessionRequest,
    ) -> agent_client_protocol::Result<acp_schema::CloseSessionResponse> {
        self.connection.handle_close_session(request).await
    }
}

/// Create an in-process ACP transport and its IO driver future.
pub fn create_in_process_transport<C>(
    runtime: crate::runtime::IronRuntime,
    client_handler: C,
) -> (InProcessTransport, impl Future<Output = ()> + 'static)
where
    C: InProcessClientHandler,
{
    let iron_conn = Rc::new(IronConnection::new(runtime));
    let client_handler = Rc::new(client_handler);
    iron_conn.set_client(Rc::new(LocalClientChannel::new(client_handler)));

    (
        InProcessTransport {
            client: InProcessClient {
                connection: iron_conn,
            },
        },
        async {},
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
    impl Future<Output = acp::Result<()>> + 'static,
) {
    let iron_conn = Rc::new(IronConnection::new(runtime));
    let future = async {
        Err(acp::Error::internal_error().data(
            "create_stdio_agent is not yet adapted to the ACP 0.11 Send handler requirements",
        ))
    };
    (iron_conn, future)
}

/// Serve ACP connections over TCP at the provided socket address.
pub async fn serve_tcp_agent(
    _runtime: crate::runtime::IronRuntime,
    _addr: std::net::SocketAddr,
) -> acp::Result<()> {
    Err(acp::Error::internal_error()
        .data("serve_tcp_agent is not yet adapted to the ACP 0.11 Send handler requirements"))
}

pub async fn connect_tcp_client(_addr: std::net::SocketAddr) -> acp::Result<()> {
    Err(acp::Error::internal_error()
        .data("connect_tcp_client is not yet adapted to the ACP 0.11 client builder API"))
}
