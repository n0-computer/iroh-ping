# iroh ping

A very simple iroh protocol for pinging a remote node. It's a high level example & easy starting point for new projects:

```rust
// a program that creates two endpoints & sends a ping between them
use anyhow::Result;
use iroh::{Endpoint, protocol::Router};
use iroh_ping::{ALPN as PingALPN, Ping};

#[tokio::main]
async fn main() -> Result<()> {
    // create the receive side
    let recv_ep = Endpoint::builder().discovery_n0().bind().await?;
    let recv_router = Router::builder(recv_ep)
        .accept(PingALPN, Ping::new())
        .spawn();
    let addr = recv_router.endpoint().node_addr().await?;

    // create a send side & send a ping
    let send_ep = Endpoint::builder().discovery_n0().bind().await?;
    let send_pinger = Ping::new();
    let rtt = send_pinger.ping(&send_ep, addr).await?;
    println!("ping took: {rtt:?} to complete");
    Ok(())
}
```

## This is not the "real" ping

Iroh has all sorts of internal ping-type messages, this is a high level demo of a protocol, and in no way necessary for iroh's normal operation.
