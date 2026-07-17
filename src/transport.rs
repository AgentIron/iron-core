//! ACP transport adapters for `iron-core`.
//!
//! Every transport enters the same ACP RPC layer, so transport selection only
//! changes how messages move between client and agent. Session ownership,
//! durable history, permission mediation, and cancellation semantics remain
//! transport-independent. The in-process adapter is operational; the stdio and
//! TCP entry points currently return ACP compatibility errors.
//!
//! ```
//! use iron_core::transport::{TransportKind, TransportMetadata, ACP_SUPPORT};
//!
//! let transport = TransportMetadata::new(TransportKind::InProcess);
//! assert_eq!(transport.kind, TransportKind::InProcess);
//! assert!(ACP_SUPPORT.stable_methods.contains(&"prompt"));
//! assert!(ACP_SUPPORT.stable_methods.contains(&"cancel"));
//! ```

use crate::connection::{ClientChannel, IronConnection};
use agent_client_protocol as acp;
use agent_client_protocol::schema::v1 as acp_schema;
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

/// Handles agent-to-client ACP callbacks for an in-process transport.
///
/// Implementations receive session notifications and permission requests on
/// the same local executor as the agent. Returned futures therefore need not
/// be [`Send`], but handlers must remain valid for the lifetime of the
/// transport.
pub trait InProcessClientHandler: 'static {
    /// Delivers an ACP session notification to the embedded client.
    ///
    /// The returned future completes when the client has accepted the
    /// notification or returns an ACP error.
    fn session_notification(
        &self,
        notification: acp_schema::SessionNotification,
    ) -> Pin<Box<dyn Future<Output = agent_client_protocol::Result<()>>>>;

    /// Asks the embedded client to decide an ACP tool permission request.
    ///
    /// The response is returned to the waiting agent operation; an ACP error
    /// aborts that permission exchange.
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

    /// Initializes the ACP connection and negotiates protocol capabilities.
    pub async fn initialize(
        &self,
        request: acp_schema::InitializeRequest,
    ) -> agent_client_protocol::Result<acp_schema::InitializeResponse> {
        self.connection.handle_initialize(request).await
    }

    /// Creates a new ACP session owned by this connection.
    pub async fn new_session(
        &self,
        request: acp_schema::NewSessionRequest,
    ) -> agent_client_protocol::Result<acp_schema::NewSessionResponse> {
        self.connection.handle_new_session(request).await
    }

    /// Runs a prompt request in an existing session.
    ///
    /// Agent notifications and permission requests are delivered through the
    /// [`InProcessClientHandler`] installed when the transport was created.
    pub async fn prompt(
        &self,
        request: acp_schema::PromptRequest,
    ) -> agent_client_protocol::Result<acp_schema::PromptResponse> {
        self.with_client_channel(|| self.connection.handle_prompt(request, None))
            .await
    }

    /// Requests cancellation of the session operation named by the notification.
    pub async fn cancel(
        &self,
        notification: acp_schema::CancelNotification,
    ) -> agent_client_protocol::Result<()> {
        self.connection.handle_cancel(notification).await
    }

    /// Closes a session through the unstable ACP `closeSession` method.
    pub async fn close_session(
        &self,
        request: acp_schema::CloseSessionRequest,
    ) -> agent_client_protocol::Result<acp_schema::CloseSessionResponse> {
        self.connection.handle_close_session(request).await
    }
}

/// Creates an in-process ACP transport and its IO driver future.
///
/// The handler mediates agent-to-client notifications and permission
/// requests. The returned driver currently performs no work and completes
/// immediately; callers may still spawn or await it to use the same ownership
/// pattern as stream-backed transports.
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

/// Creates an ACP agent intended to be bound to standard input and output.
///
/// The connection is created immediately. The returned driver currently
/// resolves to an ACP internal error because ACP 0.11 requires a `Send`
/// handler that this adapter does not yet provide.
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

/// Serves ACP connections over TCP at the provided socket address.
///
/// # Errors
///
/// Currently always returns an ACP internal error because the TCP adapter has
/// not yet been updated for the ACP 0.11 `Send` handler requirement.
pub async fn serve_tcp_agent(
    _runtime: crate::runtime::IronRuntime,
    _addr: std::net::SocketAddr,
) -> acp::Result<()> {
    Err(acp::Error::internal_error()
        .data("serve_tcp_agent is not yet adapted to the ACP 0.11 Send handler requirements"))
}

/// Connects an ACP client to a TCP agent at the provided socket address.
///
/// # Errors
///
/// Currently always returns an ACP internal error because the client adapter
/// has not yet been updated for the ACP 0.11 client builder API.
pub async fn connect_tcp_client(_addr: std::net::SocketAddr) -> acp::Result<()> {
    Err(acp::Error::internal_error()
        .data("connect_tcp_client is not yet adapted to the ACP 0.11 client builder API"))
}
