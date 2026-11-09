use std::io;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use pretty_assertions::assert_eq;
use rmcp::model::ClientCapabilities;
use rmcp::model::Implementation;
use rmcp::model::InitializeRequestParams;
use rmcp::model::ProtocolVersion;
use rmcp::model::ServerPeerInfo;
use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::io::DuplexStream;

use super::ElicitationResponse;
use super::RmcpClient;
use crate::ElicitationAction;
use crate::in_process_transport::InProcessTransportFactory;
use crate::protocol_mode::McpProtocolMode;

/// Codex's default MCP startup timeout.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
enum TestServer {
    /// Speaks 2026-07-28 and answers `server/discover` after `discover_delay`.
    Modern { discover_delay: Duration },
    /// Rejects `server/discover` as an unknown method and answers `initialize`.
    Legacy,
}

/// Opens one in-memory server per transport and records the requests each one received.
struct TestServerFactory {
    server: TestServer,
    connections: StdMutex<Vec<Arc<StdMutex<Vec<String>>>>>,
}

impl TestServerFactory {
    fn requests_per_connection(&self) -> Vec<Vec<String>> {
        self.connections
            .lock()
            .unwrap()
            .iter()
            .map(|requests| requests.lock().unwrap().clone())
            .collect()
    }
}

impl InProcessTransportFactory for TestServerFactory {
    fn open(&self) -> BoxFuture<'static, io::Result<DuplexStream>> {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let requests = Arc::new(StdMutex::new(Vec::new()));
        self.connections
            .lock()
            .unwrap()
            .push(Arc::clone(&requests));
        tokio::spawn(serve(server, self.server, requests));
        async move { Ok(client) }.boxed()
    }
}

async fn serve(stream: DuplexStream, server: TestServer, requests: Arc<StdMutex<Vec<String>>>) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let message: Value = serde_json::from_str(&line).expect("client sent JSON");
        let (Some(method), Some(id)) = (message["method"].as_str(), message.get("id").cloned())
        else {
            continue;
        };
        requests.lock().unwrap().push(method.to_string());
        let reply = match (server, method) {
            (TestServer::Modern { discover_delay }, "server/discover") => {
                tokio::time::sleep(discover_delay).await;
                json!({"jsonrpc": "2.0", "id": id, "result": {
                    "resultType": "complete",
                    "supportedVersions": ["2026-07-28"],
                    "capabilities": {"tools": {}},
                    "_meta": {
                        "io.modelcontextprotocol/serverInfo": {
                            "name": "modern-test-server",
                            "version": "1.0.0",
                        },
                    },
                    "ttlMs": 0,
                    "cacheScope": "private",
                }})
            }
            (TestServer::Legacy, "server/discover") => json!({"jsonrpc": "2.0", "id": id, "error": {
                "code": -32601,
                "message": "Method not found: server/discover",
            }}),
            (_, "initialize") => json!({"jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "serverInfo": {"name": "legacy-test-server", "version": "1.0.0"},
            }}),
            (_, other) => json!({"jsonrpc": "2.0", "id": id, "error": {
                "code": -32601,
                "message": format!("unexpected method {other}"),
            }}),
        };
        let mut bytes = serde_json::to_vec(&reply).expect("serialize reply");
        bytes.push(b'\n');
        if writer.write_all(&bytes).await.is_err() {
            break;
        }
    }
}

/// Starts an opted-in 2026-07-28 client against `server`; returns the handshake
/// result and the requests each opened transport received.
async fn start(server: TestServer) -> (anyhow::Result<ServerPeerInfo>, Vec<Vec<String>>) {
    let factory = Arc::new(TestServerFactory {
        server,
        connections: StdMutex::new(Vec::new()),
    });
    let client = RmcpClient::new_in_process_client_with_protocol_mode(
        Arc::clone(&factory) as Arc<dyn InProcessTransportFactory>,
        McpProtocolMode::V20260728,
    )
    .await
    .expect("in-process client");
    let result = client
        .initialize(
            InitializeRequestParams::new(
                ClientCapabilities::default(),
                Implementation::new("lifecycle-test", "1.0.0"),
            )
            .with_protocol_version(ProtocolVersion::V_2025_06_18),
            Some(STARTUP_TIMEOUT),
            Box::new(|_, _| {
                async {
                    Ok(ElicitationResponse {
                        action: ElicitationAction::Decline,
                        content: None,
                        meta: None,
                    })
                }
                .boxed()
            }),
        )
        .await;
    client.shutdown().await;
    (result, factory.requests_per_connection())
}

fn server_name(info: &ServerPeerInfo) -> Option<&str> {
    info.server_info.as_ref().map(|info| info.name.as_str())
}

// #1950: rmcp's `Auto` lifecycle gives up on `server/discover` after a fixed
// 10 s and sends `initialize` on the same transport, where the late discover
// reply then fails startup with `ConflictInitResponseId`.
#[tokio::test(start_paused = true)]
async fn modern_server_answering_discover_after_ten_seconds_starts_modern() {
    let (result, requests) = start(TestServer::Modern {
        discover_delay: Duration::from_secs(15),
    })
    .await;

    let info = result.expect("a discover reply inside the startup timeout must start the server");
    assert_eq!(info.protocol_version, ProtocolVersion::V_2026_07_28);
    assert_eq!(server_name(&info), Some("modern-test-server"));
    assert_eq!(requests, vec![vec!["server/discover".to_string()]]);
}

#[tokio::test(start_paused = true)]
async fn modern_server_answering_discover_promptly_starts_modern() {
    let (result, requests) = start(TestServer::Modern {
        discover_delay: Duration::ZERO,
    })
    .await;

    let info = result.expect("a prompt modern server must start");
    assert_eq!(info.protocol_version, ProtocolVersion::V_2026_07_28);
    assert_eq!(server_name(&info), Some("modern-test-server"));
    assert_eq!(requests, vec![vec!["server/discover".to_string()]]);
}

#[tokio::test(start_paused = true)]
async fn legacy_server_rejecting_discover_falls_back_to_initialize() {
    let (result, requests) = start(TestServer::Legacy).await;

    let info = result.expect("a legacy server must start through the initialize fallback");
    assert_eq!(info.protocol_version, ProtocolVersion::V_2025_06_18);
    assert_eq!(server_name(&info), Some("legacy-test-server"));
    assert_eq!(
        requests.concat(),
        vec!["server/discover".to_string(), "initialize".to_string()]
    );
}
