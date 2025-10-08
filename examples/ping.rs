use anyhow::Result;
use iroh::{protocol::Router, Endpoint};
use iroh_ping::Ping;

#[tokio::main]
async fn main() -> Result<()> {
    // create the receive side
    let recv_ep = Endpoint::builder().discovery_n0().bind().await?;
    let recv_router = Router::builder(recv_ep.clone())
        .accept(iroh_ping::ALPN, Ping::new())
        .spawn();
    recv_ep.online().await;
    let addr = recv_router.endpoint().node_addr();

    // create a send side & send a ping
    let send_ep = Endpoint::builder().discovery_n0().bind().await?;
    let send_pinger = Ping::new();
    let rtt = send_pinger.ping(&send_ep, addr).await?;
    println!("ping took: {rtt:?} to complete");
    Ok(())
}
