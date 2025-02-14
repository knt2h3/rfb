use std::net::ToSocketAddrs;
use clap::Parser;
use log::{info, warn, LevelFilter};
use rfb::{
    clientv2::{Client, ServerMessage, VncClientConfig},
    rfb::{Position, ProtoVersion, Resolution},
};
use image::{ImageBuffer, Rgb};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// VNC server address (e.g. "localhost:5900")
    #[arg(value_name = "HOST:PORT")]
    address: String,

    /// Proxy listen address (e.g. "localhost:5901")
    #[arg(value_name = "HOST:PORT")]
    proxy: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::builder()
        .filter_level(LevelFilter::Info)
        .init();

    // Parse command line arguments
    let args = Args::parse();

    /*
    // Parse and resolve the address
    let addr = args.address
        .to_socket_addrs()?
        .next()
        .ok_or("Failed to resolve address")?;

    // Connect to the VNC server
    info!("Connecting to VNC server at {}", addr);
    let stream = tokio::net::TcpStream::connect(addr).await?;

    stream.set_nodelay(true)?;

    // Create client configuration
    let config = VncClientConfig {
        version: ProtoVersion::Rfb38,
        shared: false,
    };

    // Initialize the VNC client
    let mut client = Client::init_conn(config, stream).await?;

    // let initial_resolution = client.get_initial_resolution().clone();
    // let (mut read_half, mut write_half) = client.split();

    */

    Ok(())
}
