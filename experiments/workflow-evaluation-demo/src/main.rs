#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub(crate) mod adapters;
pub(crate) mod composition;
mod mcp;

use std::error::Error;

use composition::Composition;
use mcp::FactoryMcp;
use mcp_transport::BoundedStdioTransport;
use rmcp::ServiceExt;

/// The unified `factory_*` MCP server process for this project.
///
/// Builds the static demo composition once, then serves it over one bounded
/// stdio transport until the input stream closes. There is no second router
/// and no direct `rmcp` stdio helper: every byte on the wire goes through
/// [`BoundedStdioTransport`].
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let composition = Composition::new()?;
    let handler = FactoryMcp::new(composition);
    let transport = BoundedStdioTransport::new(tokio::io::stdin(), tokio::io::stdout());
    let running = handler.serve(transport).await?;
    running.waiting().await?;
    Ok(())
}
