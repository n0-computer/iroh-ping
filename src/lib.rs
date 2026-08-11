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

/// Request tag: ping once, get a `PONG` back.
const PING: &[u8; 4] = b"PING";

/// Request tag: send bytes until the stream ends, get the received byte count back.
const FLOOD: &[u8; 4] = b"FLOD";

/// Size of the buffer we write/read in one go while flooding.
const CHUNK: usize = 64 * 1024;

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
        send.write_all(PING).await?;

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

    /// Sends bytes as fast as the connection allows for roughly `duration`,
    /// returning how many bytes the remote actually received and how long that
    /// took, so the data rate is `bytes / elapsed`.
    ///
    /// Bytes are counted on the receiving side, and the elapsed time includes
    /// draining whatever is still in flight when `duration` runs out, so the
    /// result measures the path rather than how fast we can fill a send buffer.
    ///
    /// Pass [`Duration::MAX`] to flood until the future is dropped. Nothing is
    /// returned in that case, so watch [`Metrics::bytes_sent`] to see progress.
    pub async fn flood(
        &self,
        endpoint: &Endpoint,
        addr: EndpointAddr,
        duration: Duration,
    ) -> anyhow::Result<(u64, Duration)> {
        let conn = endpoint.connect(addr, ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(FLOOD).await?;

        let buf = vec![0u8; CHUNK];
        let start = Instant::now();
        while start.elapsed() < duration {
            let n = send.write(&buf).await?;
            self.metrics.bytes_sent.inc_by(n as u64);
        }

        // Signal the end of the flood, then wait for the remote to report back
        // how much of it arrived.
        send.finish()?;
        let report = recv.read_to_end(8).await?;
        let elapsed = start.elapsed();
        let report: [u8; 8] = report
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("malformed flood report: {} bytes", report.len()))?;

        conn.close(0u32.into(), b"bye!");

        Ok((u64::from_le_bytes(report), elapsed))
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
        // connecting peer to open a single bi-directional stream, starting with
        // a four byte tag saying what it wants.
        let (mut send, mut recv) = connection.accept_bi().await?;

        let mut req = [0u8; 4];
        recv.read_exact(&mut req)
            .await
            .map_err(AcceptError::from_err)?;

        match &req {
            PING => {
                // increment count of pings we've received
                metrics.pings_recv.inc();

                // send back "PONG" bytes
                send.write_all(b"PONG")
                    .await
                    .map_err(AcceptError::from_err)?;

                // By calling `finish` on the send stream we signal that we will not send anything
                // further, which makes the receive stream on the other end terminate.
                send.finish()?;
            }
            FLOOD => {
                // Drain the stream until the sender finishes it, then report
                // back how much arrived so they can work out the data rate.
                let mut buf = vec![0u8; CHUNK];
                let mut recvd = 0u64;
                // A read error means the sender went away mid-flood, which is a
                // normal way for an open-ended flood to end.
                while let Ok(Some(n)) = recv.read(&mut buf).await {
                    recvd += n as u64;
                }
                metrics.bytes_recv.inc_by(recvd);

                // Best effort: there may be nobody left to hear the report.
                let _ = send.write_all(&recvd.to_le_bytes()).await;
                let _ = send.finish();
            }
            _ => return Err(std::io::Error::other(format!("unknown request {req:?}")).into()),
        }

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
    /// total bytes written in flood requests
    pub bytes_sent: Counter,
    /// total bytes read from flood requests
    pub bytes_recv: Counter,
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use iroh::{Endpoint, endpoint::presets, protocol::Router};

    use super::*;

    #[tokio::test]
    async fn test_ping() -> Result<()> {
        let server_endpoint = Endpoint::bind(presets::N0).await?;
        let server_ping = Ping::new();
        let server_metrics = server_ping.metrics().clone();
        let server_router = Router::builder(server_endpoint)
            .accept(ALPN, server_ping)
            .spawn();
        let server_addr = server_router.endpoint().addr();

        let client_endpoint = Endpoint::bind(presets::N0).await?;
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

        let (bytes, elapsed) = client_ping
            .flood(&client_endpoint, server_addr, Duration::from_millis(500))
            .await?;
        println!(
            "flooded {bytes} bytes in {elapsed:?}: {:.2} MiB/s",
            bytes as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0)
        );
        assert!(bytes > 0);
        assert_eq!(server_metrics.bytes_recv.get(), bytes);
        assert!(client_metrics.bytes_sent.get() >= bytes);

        client_endpoint.close().await;
        server_router.shutdown().await?;

        Ok(())
    }
}
