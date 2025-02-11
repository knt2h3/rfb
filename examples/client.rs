use std::net::ToSocketAddrs;
use clap::Parser;
use log::{info, LevelFilter};
use rfb::{
    client::{Client, VncClientConfig},
    rfb::ProtoVersion,
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// VNC server address (e.g. "localhost:5900")
    #[arg(value_name = "HOST:PORT")]
    address: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::builder()
        .filter_level(LevelFilter::Info)
        .init();

    // Parse command line arguments
    let args = Args::parse();

    // Parse and resolve the address
    let addr = args.address
        .to_socket_addrs()?
        .next()
        .ok_or("Failed to resolve address")?;

    // Connect to the VNC server
    info!("Connecting to VNC server at {}", addr);
    let stream = tokio::net::TcpStream::connect(addr).await?;

    // Create client configuration
    let config = VncClientConfig {
        version: ProtoVersion::Rfb38,
        shared: false,
    };

    // Initialize the VNC client
    let mut client = Client::init_conn(config, stream).await?;

    // Request a framebuffer update (non-incremental)
    info!("Requesting framebuffer update");
    let update = client.framebuffer_update_request(
        false,
        rfb::rfb::Position { x: 0, y: 0 },
        rfb::rfb::Resolution { width: 1024, height: 768 },
    ).await?;

    // Print the number of rectangles received
    info!("Received {} rectangles in the framebuffer update", update.rectangles.len());

    Ok(())
}
