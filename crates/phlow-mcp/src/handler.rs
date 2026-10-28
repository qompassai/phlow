//! The rmcp SDK surface: a thin framing shell over [`McpServer`].
//!
//! The domain server is stateful (`&mut self`: the runtime is a
//! single-owner pipeline, the board is shared state) and the
//! hand-rolled transport served strictly sequentially; a mutex
//! preserves exactly that execution contract under the SDK's
//! concurrent dispatch. A poisoned lock is recovered rather than
//! propagated: the panic that poisoned it has already been
//! reported by the panicking call, and the domain state behind
//! the lock is the runtime + a rebuildable board.
//!
//! Era posture (fleet standardization, D-113): modern only.
//! `supported_protocol_versions` pins 2026-07-28, so
//! `server/discover` advertises exactly one version and the SDK
//! validates every request's `_meta` against it. Legacy-era
//! contact never reaches this handler — the era guard
//! ([`crate::guard`]) refuses `initialize` at the byte level.
//!
//! Result shapes preserve the hand-rolled modern path
//! (FLOWGUARD-MAPPING A8/A9): tool payloads are carried as
//! ASCII-escaped JSON text built by [`crate::json_ascii::dumps`]
//! (GAP-3 option (i): the wire bytes the rose↔phlow seam was
//! built against), plus `structuredContent`, `isError`, and
//! `_meta` serverInfo; list/discover results carry the catalog's
//! caching hints (`ttlMs`, `cacheScope: "public"`).

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    DiscoverResult, ErrorCode, ErrorData, Implementation, ListToolsResult, MetaObject,
    PaginatedRequestParams, ProtocolVersion, RequestMetaObject, ServerCapabilities,
    ServerConfig as RmcpServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;

use crate::json_ascii::{DumpsError, dumps};
use crate::protocol::{
    CACHE_SCOPE, CACHE_TTL_MS, INSTRUCTIONS, INTERNAL_ERROR, INVALID_PARAMS, META_SERVER_INFO,
    SERVER_NAME, SERVER_VERSION, rmcp_tools,
};
use crate::server::{CallOutcome, McpRuntime, McpServer};

/// The rmcp handler over a shared domain server. The runtime
/// must be `Send`: the SDK serves from spawned tasks, and the
/// mutex below is what keeps the single-owner domain dispatch
/// sequential across them.
pub struct PhlowHandler<R: McpRuntime + Send + 'static> {
    server: Arc<Mutex<McpServer<R>>>,
}

impl<R: McpRuntime + Send + 'static> PhlowHandler<R> {
    /// Wrap a built domain server for SDK serving.
    pub fn new(server: Arc<Mutex<McpServer<R>>>) -> PhlowHandler<R> {
        PhlowHandler { server }
    }
}

/// The `_meta` every modern result carries: this server's identity
/// (FLOWGUARD-MAPPING A9 — the same name/version in discover,
/// list, and call results).
fn server_info_meta() -> MetaObject {
    let mut meta = MetaObject::new();
    meta.insert(
        META_SERVER_INFO.to_owned(),
        serde_json::json!({"name": SERVER_NAME, "version": SERVER_VERSION}),
    );
    meta
}

/// The caller's attention identity in a request's `_meta`: the
/// clientInfo name. This is the modern branch of the hand-rolled
/// server's `caller_identity`, and it is the specification
/// (FLOWGUARD-MAPPING B4): no `_meta` clientInfo → no identity →
/// attention tools fail closed in the domain dispatch.
fn caller_name(meta: &RequestMetaObject) -> Option<String> {
    meta.client_info().map(|info| info.name)
}

/// Map a domain refusal to the SDK's error carriage, preserving
/// the refusal's code and message verbatim.
fn refusal_error(code: i32, message: String) -> ErrorData {
    match code {
        INVALID_PARAMS => ErrorData::invalid_params(message, None),
        INTERNAL_ERROR => ErrorData::internal_error(message, None),
        other => ErrorData::new(ErrorCode(other), message, None),
    }
}

impl<R: McpRuntime + Send + 'static> ServerHandler for PhlowHandler<R> {
    /// Modern era only: `server/discover` advertises exactly this
    /// one version and the SDK validates every request's `_meta`
    /// against it.
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28])
    }

    fn get_info(&self) -> RmcpServerConfig {
        RmcpServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, SERVER_VERSION))
            .with_instructions(INSTRUCTIONS)
    }

    /// Discover, with the catalog's caching hints restored: the
    /// SDK's default `from_server_info` reports `ttlMs: 0` /
    /// `cacheScope: "private"`, where the hand-rolled server
    /// reported the catalog constants (A9).
    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<DiscoverResult, ErrorData> {
        let result = DiscoverResult::from_server_info(
            self.supported_protocol_versions().into_owned(),
            self.get_info(),
        )
        .with_ttl_ms(CACHE_TTL_MS)
        .with_cache_scope(CacheScope::Public);
        debug_assert_eq!(CACHE_SCOPE, "public");
        Ok(result)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut result = ListToolsResult::with_all_items(rmcp_tools());
        result.ttl_ms = Some(CACHE_TTL_MS);
        result.cache_scope = Some(CacheScope::Public);
        result.meta = Some(server_info_meta());
        Ok(result)
    }

    /// Tool failures stay MCP `isError` results (never RPC
    /// errors), with the hand-rolled payloads: ASCII-escaped JSON
    /// text (json_ascii bytes, GAP-3 (i)) + `structuredContent`.
    /// Dispatch refusals (unknown tool, schema-invalid arguments)
    /// stay protocol errors with their original codes/messages.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| serde_json::json!({}));
        let caller = caller_name(&context.meta);
        let outcome = {
            let mut server = self
                .server
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            server.call_tool(&request.name, &args, caller.as_deref())
        };
        match outcome {
            CallOutcome::Refusal { code, message } => Err(refusal_error(code, message)),
            CallOutcome::Result { result, is_error } => {
                let text = match dumps(&result) {
                    Ok(text) => text,
                    Err(DumpsError::NonFiniteFloat) => {
                        return Err(ErrorData::internal_error(
                            "Runtime returned non-JSON data",
                            None,
                        ));
                    }
                };
                let mut call_result = if is_error {
                    CallToolResult::error(vec![ContentBlock::text(text)])
                } else {
                    CallToolResult::success(vec![ContentBlock::text(text)])
                };
                call_result.structured_content = Some(result);
                call_result.meta = Some(server_info_meta());
                Ok(CallToolResponse::Complete(call_result))
            }
        }
    }
}

/// Duplex buffer between the era guard and the SDK service. The
/// SDK consumes incrementally, so this only needs to cover
/// request/response bursts, not whole payloads.
const DUPLEX_BUFFER_BYTES: usize = 1024 * 1024;

/// Serve until stdin EOF: the era guard owns stdio, the SDK
/// service sits behind an in-process duplex, and a pump copies
/// its responses back to stdout through the same lock the guard
/// writes with. The `RunningService` is kept alive on
/// `waiting()` for the whole session — dropping it would close
/// the server after the first request.
async fn serve_modern<R: McpRuntime + Send + 'static>(
    handler: PhlowHandler<R>,
) -> std::io::Result<()> {
    let (client_side, server_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let (server_read, server_write) = tokio::io::split(server_side);
    let (client_read, client_write) = tokio::io::split(client_side);
    let stdout: crate::guard::SharedStdout = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));

    let ordering = crate::guard::Ordering::new();
    let serve = tokio::spawn(async move {
        let transport = AsyncRwTransport::new_server(server_read, server_write);
        match handler.serve(transport).await {
            Ok(running) => running
                .waiting()
                .await
                .map(|_| ())
                .map_err(|e| std::io::Error::other(e.to_string())),
            // The client closed the connection before selecting a
            // lifecycle (a probe, a spawn check, a client that only
            // ever wanted the refusal): bare EOF is a clean end of
            // session, as it was when the transport was hand-rolled.
            Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)) => Ok(()),
            Err(e) => Err(std::io::Error::other(e.to_string())),
        }
    });
    let mut pump = tokio::spawn(crate::guard::pump(
        client_read,
        Arc::clone(&stdout),
        Arc::clone(&ordering),
    ));

    // When stdin reaches EOF the guard shuts the duplex write
    // side down, which ends the SDK service; the pump ends when
    // the service's transport drops. The pump wait is bounded as
    // a backstop: a response still in flight when the service
    // ends gets its chance to drain, but a wedged pump cannot
    // hang exit.
    let guard_result = crate::guard::run(tokio::io::stdin(), client_write, stdout, ordering).await;
    let serve_result = match serve.await {
        Ok(result) => result,
        Err(e) => Err(std::io::Error::other(e.to_string())),
    };
    if tokio::time::timeout(std::time::Duration::from_secs(2), &mut pump)
        .await
        .is_err()
    {
        pump.abort();
    }
    guard_result.and(serve_result)
}

/// Run the stdio MCP server until EOF on stdin.
///
/// stdout is the protocol channel — diagnostics go to stderr
/// only. The domain server is built synchronously (it wraps the
/// runtime); only the framing runs on the async runtime, so the
/// CLI's synchronous command paths are untouched. The wrapped
/// runtime is closed exactly once when serving ends, on every
/// exit path, mirroring the hand-rolled `serve`'s `finally`.
pub fn serve_stdio<R: McpRuntime + Send + 'static>(server: McpServer<R>) -> std::io::Result<()> {
    let shared = Arc::new(Mutex::new(server));
    let handler = PhlowHandler::new(Arc::clone(&shared));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(serve_modern(handler));
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .close();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::RequestMetaObject;

    #[test]
    fn caller_name_comes_from_meta_client_info_only() {
        // B4: identity is per-request `_meta` clientInfo. No
        // clientInfo → no identity (attention fails closed in
        // the domain); a named clientInfo is the identity.
        let meta = RequestMetaObject::new();
        assert_eq!(caller_name(&meta), None);

        let mut meta = RequestMetaObject::new();
        meta.set_client_info(Implementation::new("rose", "0.1.0"));
        assert_eq!(caller_name(&meta).as_deref(), Some("rose"));
    }

    #[test]
    fn server_info_meta_names_flow() {
        let meta = server_info_meta();
        let info = &meta[META_SERVER_INFO];
        assert_eq!(info["name"], "flow");
        assert_eq!(info["version"], SERVER_VERSION);
    }

    #[test]
    fn refusal_codes_are_preserved() {
        let error = refusal_error(INVALID_PARAMS, "Unknown Phlow tool".to_owned());
        assert_eq!(error.code.0, INVALID_PARAMS);
        assert_eq!(error.message, "Unknown Phlow tool");
        let error = refusal_error(INTERNAL_ERROR, "x".to_owned());
        assert_eq!(error.code.0, INTERNAL_ERROR);
    }
}
