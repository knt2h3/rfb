use std::net::ToSocketAddrs;
use clap::Parser;
use log::{info, warn, error, LevelFilter};
use rfb::encodings::EncodingType;
use rfb::rfb::{ClientInit, SecurityResult, SecurityType, SecurityTypes, ServerInit, FRAMEBUFFER_UPDATE_MESSAGE_TYPE};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncWriteExt, AsyncWrite};
use rfb::{
    clientv2::{Client, ServerMessage, VncClientConfig, ClientReadHalf, ClientWriteHalf},
    rfb::{Position, ProtoVersion, Resolution, ClientMessage, ReadMessage, WriteMessage, RectangleEncoded},
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// VNC server address (e.g. "localhost:5900")
    #[arg(value_name = "SERVER_HOST:SERVER_PORT")]
    address: String,

    /// Proxy listen address (e.g. "localhost:5901")
    #[arg(value_name = "PROXY_HOST:PROXY_PORT")]
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

    // Parse and resolve the server address
    let server_addr = args.address
        .to_socket_addrs()?
        .next()
        .ok_or("Failed to resolve server address")?;

    // Create TCP listener for proxy
    let listener = TcpListener::bind(&args.proxy).await?;
    info!("RFB proxy listening on {}", args.proxy);

    // Accept incoming connections
    loop {
        let (client_stream, client_addr) = listener.accept().await?;
        info!("New client connection from {}", client_addr);

        // Clone server address for this connection
        let server_addr = server_addr;

        // Spawn a new task to handle this connection
        tokio::spawn(async move {
            if let Err(e) = handle_connection(client_stream, server_addr).await {
                error!("Error handling connection from {}: {}", client_addr, e);
            }
        });
    }
}

async fn handle_connection(mut client_stream: TcpStream, server_addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    // Connect to VNC server
    let mut server_stream = TcpStream::connect(server_addr).await?;
    server_stream.set_nodelay(true)?;

    // TODO: proxy handshake
    let server_version = ProtoVersion::read_from(&mut server_stream).await?;
    info!("Rx: ServerVersion={:?}", server_version);
    server_version.write_to(&mut client_stream).await?;

    let client_version = ProtoVersion::read_from(&mut client_stream).await?;
    info!("Tx: ClientVersion={:?}", client_version);
    client_version.write_to(&mut server_stream).await?;

    let supported_types = SecurityTypes::read_from(&mut server_stream).await?;
    info!("Rx: Server SupportedSecurityTypes={:?}", supported_types);
    supported_types.write_to(&mut client_stream).await?;

    let client_choice = SecurityType::read_from(&mut client_stream).await?;
    client_choice.write_to(&mut server_stream).await?;

    let security_result = SecurityResult::read_from(&mut server_stream).await?;
    info!("Rx: SecurityResult={:?}", security_result);
    // assert!(matches!(res, SecurityResult::Success));
    security_result.write_to(&mut client_stream).await?;

    // TODO: proxy init
    let client_init = ClientInit::read_from(&mut client_stream).await?;
    info!("Rx: ClientInit={:?}", client_init);
    client_init.clone().write_to(&mut server_stream).await?;

    let server_init = ServerInit::read_from(&mut server_stream).await?;
    info!("Rx: ServerInit={:?}", server_init);
    server_init.clone().write_to(&mut client_stream).await?;

    // Create VNC client config
    let config = VncClientConfig {
        version: client_version,
        shared: client_init.shared,
    };

    // Initialize VNC client connection
    // let client = Client::create_from_initialized_conn(config, server_stream, server_init).await?;
    // let (mut server_read, mut server_write) = client.split();
    let (mut server_read, mut server_write) = tokio::io::split(server_stream);

    // Split client stream
    let (mut client_read, mut client_write) = tokio::io::split(client_stream);

    // Spawn task to forward client -> server messages
    let client_to_server = tokio::spawn(async move {
        loop {
            match ClientMessage::read_from(&mut client_read).await {
                Ok(msg) => {
                    match msg {
                        ClientMessage::SetPixelFormat(pf) => {
                            info!("Forwarding (Client->Server) SetPixelFormat: {:?}", pf);
                            ClientMessage::SetPixelFormat(pf).write_to(&mut server_write).await.unwrap();
                            // if let Err(e) = server_write.set_pixel_format(pf).await {
                            //     error!("Error forwarding SetPixelFormat: {}", e);
                            //     break;
                            // }
                        }
                        ClientMessage::SetEncodings(mut enc) => {
                            info!("Forwarding (Client->Server) SetEncodings: {:?}", enc);


                            // remove last n encodings
                            enc.truncate(enc.len() - 9);

                            // remove first 5 encodings
                            enc.drain(..12);

                            let encodings = enc.iter().map(|e| e.clone()).collect::<Vec<_>>();

                            for e in encodings {
                                info!("Forwarding (Client->Server) Encoding: {:?}", e);
                            }

                            // whoa; hack
                            // enc.clear();
                            // enc.push(EncodingType::QemuExtendedKeyEvents);

                            ClientMessage::SetEncodings(enc).write_to(&mut server_write).await.unwrap();
                            // if let Err(e) = server_write.set_encodings(enc).await {
                            //     error!("Error forwarding SetEncodings: {}", e);
                            //     break;
                            // }
                        }
                        ClientMessage::FramebufferUpdateRequest(req) => {
                            info!("Forwarding (Client->Server) FramebufferUpdateRequest: {:?}", req);
                            ClientMessage::FramebufferUpdateRequest(req).write_to(&mut server_write).await.unwrap();
                            // warn!("SKIPPING Forwarding (Client->Server) FramebufferUpdateRequest: {:?}", req);
                            // if let Err(e) = server_write.framebuffer_update_request(req.incremental, req.position, req.resolution).await {
                            //     error!("Error forwarding FramebufferUpdateRequest: {}", e);
                            //     break;
                            // }
                        }
                        ClientMessage::KeyEvent(ke) => {
                            info!("Forwarding (Client->Server) KeyEvent: {:?}", ke);
                            ClientMessage::KeyEvent(ke).write_to(&mut server_write).await.unwrap();
                            // if let Err(e) = server_write.key_event(ke.is_pressed, ke.keysym_raw).await {
                            //     error!("Error forwarding KeyEvent: {}", e);
                            //     break;
                            // }
                        }
                        ClientMessage::PointerEvent(pe) => {
                            info!("Forwarding (Client->Server) PointerEvent: {:?}", pe);
                            ClientMessage::PointerEvent(pe).write_to(&mut server_write).await.unwrap();
                            // if let Err(e) = server_write.pointer_event(pe.position, pe.pressed).await {
                            //     error!("Error forwarding PointerEvent: {}", e);
                            //     break;
                            // }
                        }
                        ClientMessage::ClientCutText(text) => {
                            info!("Forwarding (Client->Server) ClientCutText: {:?}", text);
                            ClientMessage::ClientCutText(text).write_to(&mut server_write).await.unwrap();
                            // if let Err(e) = server_write.client_cut_text(text).await {
                            //     error!("Error forwarding ClientCutText: {}", e);
                            //     break;
                            // }
                        }
                    }
                }
                Err(e) => {
                    error!("Error reading client message: {}", e);
                    break;
                }
            }
        }
    });

    // Forward server -> client messages
    let server_to_client = tokio::spawn(async move {
        loop {
            match ServerMessage::read_from(&mut server_read).await {
                Ok(msg) => {
                    match msg {
                        ServerMessage::FramebufferUpdate(update) => {
                            info!("Forwarding (Server->Client) FramebufferUpdate: rectangles={}, encoding_types={:?}", update.rectangles.len(), update.rectangles.iter().map(|r| r.encoding_type).collect::<Vec<_>>());

                            // Write message type and padding
                            if let Err(e) = client_write.write_u8(FRAMEBUFFER_UPDATE_MESSAGE_TYPE).await {
                                error!("Error writing message type: {}", e);
                                break;
                            }
                            if let Err(e) = client_write.write_u8(0).await {
                                error!("Error writing padding: {}", e);
                                break;
                            }

                            // Write number of rectangles
                            if let Err(e) = client_write.write_u16(update.rectangles.len() as u16).await {
                            // if let Err(e) = client_write.write_u16_le(update.rectangles.len() as u16).await {
                                error!("Error writing rectangle count: {}", e);
                                break;
                            }

                            // Write rectangles
                            for rect in update.rectangles {
                                info!("Forwarding Rectangle: {:?}", rect.encoding_type);

                                // Write position
                                if let Err(e) = client_write.write_u16(rect.position.x).await {
                                    error!("Error writing rectangle x: {}", e);
                                    break;
                                }
                                if let Err(e) = client_write.write_u16(rect.position.y).await {
                                    error!("Error writing rectangle y: {}", e);
                                    break;
                                }
                                
                                // Write dimensions
                                if let Err(e) = client_write.write_u16(rect.dimensions.width).await {
                                    error!("Error writing rectangle width: {}", e);
                                    break;
                                }
                                if let Err(e) = client_write.write_u16(rect.dimensions.height).await {
                                    error!("Error writing rectangle height: {}", e);
                                    break;
                                }

                                // Write encoding type
                                if let Err(e) = client_write.write_i32(rect.encoding_type.into()).await {
                                    error!("Error writing encoding type: {}", e);
                                    break;
                                }

                                if let EncodingType::QemuExtendedKeyEvents = rect.encoding_type {
                                    info!("QemuExtendedKeyEvents - no data to write");
                                    continue;
                                }

                                // Write rectangle data
                                if let Err(e) = client_write.write_all(&rect.data).await {
                                    error!("Error writing rectangle data: {}", e);
                                    break;
                                }
                            }
                        }
                        ServerMessage::Bell => {
                            info!("Forwarding Bell");
                            if let Err(e) = client_write.write_u8(2).await {
                                error!("Error writing bell message: {}", e);
                                break;
                            }
                        }
                        ServerMessage::ServerCutText(cut_text) => {
                            info!("Forwarding ServerCutText: {:?}", cut_text);
                            if let Err(e) = client_write.write_u8(3).await {
                                error!("Error writing server cut text message: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Error reading server message: {}", e);
                    break;
                }
            }
        }
    });

    // Wait for either direction to complete
    tokio::select! {
        _ = client_to_server => info!("Client to server connection closed"),
        _ = server_to_client => info!("Server to client connection closed"),
    }

    Ok(())
}
