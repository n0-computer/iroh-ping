use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use iroh_metrics::{Counter, MetricsGroup};

/// Each protocol is identified by its ALPN string.
///
/// The ALPN, or application-layer protocol negotiation, is exchanged in the connection handshake,
/// and the connection is aborted unless both nodes pass the same bytestring.
pub const ALPN: &[u8] = b"iroh/ping/0";

/// Ping is our protocol struct.
///
/// We'll implement [`ProtocolHandler`] on this struct so we can use it with
/// an [`iroh::protocol::Router`].
/// It's also fine to keep state in this struct for use across many incoming
/// connections, in this case we'll keep metrics about the amount of pings we
/// sent or received.
#[derive(Debug, Clone)]
pub struct Ping {
    metrics: Arc<Metrics>,
}

impl Default for Ping {
    fn default() -> Self {
        Self::new()
    }
}

impl Ping {
    /// Creates new ping state.
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(Metrics::default()),
        }
    }

    /// Returns a handle to ping metrics.
    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    /// Sends a ping on the provided endpoint to a given node address.
    pub async fn ping(&self, endpoint: &Endpoint, addr: EndpointAddr) -> anyhow::Result<Duration> {
        // Open a connection to the accepting node
        let conn = endpoint.connect(addr, ALPN).await?;

        // Open a bidirectional QUIC stream
        let (mut send, mut recv) = conn.open_bi().await?;

        let start = Instant::now();
        // Send some data to be pinged
        send.write_all(b"PING").await?;

        // Signal the end of data for this particular stream
        send.finish()?;

        // read the response, which must be PONG as bytes
        let response = recv.read_to_end(4).await?;
        assert_eq!(&response, b"PONG");

        let ping = start.elapsed();

        // at this point we've successfully pinged, mark the metric
        self.metrics.pings_sent.inc();

        // Explicitly close the whole connection, as we're the last ones to receive data
        // and know there's nothing else more to do in the connection.
        conn.close(0u32.into(), b"bye!");

        Ok(ping)
    }
}

impl ProtocolHandler for Ping {
    /// The `accept` method is called for each incoming connection for our ALPN.
    ///
    /// The returned future runs on a newly spawned tokio task, so it can run as long as
    /// the connection lasts.
    async fn accept(&self, connection: Connection) -> n0_error::Result<(), AcceptError> {
        let metrics = self.metrics.clone();

        // We can get the remote's node id from the connection.
        let node_id = connection.remote_id();
        println!("accepted connection from {node_id}");

        // Our protocol is a simple request-response protocol, so we expect the
        // connecting peer to open a single bi-directional stream.
        let (mut send, mut recv) = connection.accept_bi().await?;

        let req = recv.read_to_end(4).await.map_err(AcceptError::from_err)?;
        assert_eq!(&req, b"PING");

        // increment count of pings we've received
        metrics.pings_recv.inc();

        // send back "PONG" bytes
        send.write_all(b"PONG")
            .await
            .map_err(AcceptError::from_err)?;

        // By calling `finish` on the send stream we signal that we will not send anything
        // further, which makes the receive stream on the other end terminate.
        send.finish()?;

        // Wait until the remote closes the connection, which it does once it
        // received the response.
        connection.closed().await;

        Ok(())
    }
}

/// Enum of metrics for the module
#[derive(Debug, Default, MetricsGroup)]
#[metrics(name = "ping")]
pub struct Metrics {
    /// count of valid ping messages sent
    pub pings_sent: Counter,
    /// count of valid ping messages received
    pub pings_recv: Counter,
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use iroh::{Endpoint, protocol::Router};

    use super::*;

    #[tokio::test]
    async fn test_ping() -> Result<()> {
        let server_endpoint = Endpoint::builder().bind().await?;
        let server_ping = Ping::new();
        let server_metrics = server_ping.metrics().clone();
        let server_router = Router::builder(server_endpoint)
            .accept(ALPN, server_ping)
            .spawn();
        let server_addr = server_router.endpoint().addr();

        let client_endpoint = Endpoint::builder().bind().await?;
        let client_ping = Ping::new();
        let client_metrics = client_ping.metrics().clone();

        let res = client_ping
            .ping(&client_endpoint, server_addr.clone())
            .await?;
        println!("ping response: {res:?}");
        assert_eq!(server_metrics.pings_recv.get(), 1);
        assert_eq!(client_metrics.pings_sent.get(), 1);

        let res = client_ping
            .ping(&client_endpoint, server_addr.clone())
            .await?;
        println!("ping response: {res:?}");
        assert_eq!(server_metrics.pings_recv.get(), 2);
        assert_eq!(client_metrics.pings_sent.get(), 2);

        client_endpoint.close().await;
        server_router.shutdown().await?;

        Ok(())
    }
}
