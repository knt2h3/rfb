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
}

/*
async fn request_framebuffer(
    client: &mut Client<tokio::net::TcpStream>,
    frame_number: usize,
    resolution: Resolution,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Requesting framebuffer update");

    let width = resolution.width as u32;
    let height = resolution.height as u32;

    let update = client.framebuffer_update_request(
        false,
        Position { x: 0, y: 0 },
        resolution,
    ).await?;

    // Create an image buffer to store the pixels
    let mut img = ImageBuffer::new(width, height);

    // Print information about the received rectangles
    info!("Received {} rectangles in the framebuffer update", update.rectangles.len());
    for (i, rect) in update.rectangles.iter().enumerate() {
        let num_pixels = rect.dimensions.width as u64 * rect.dimensions.height as u64;
        info!(
            "Rectangle {}: position=({}, {}), size={}x{}, pixels={}, encoding={:?}",
            i,
            rect.position.x,
            rect.position.y,
            rect.dimensions.width,
            rect.dimensions.height,
            num_pixels,
            rect.encoding_type
        );

        // Extract pixel data from the rectangle and write it to the image buffer
        match rect.encoding_type {
            rfb::encodings::EncodingType::Raw => {
                // Assuming RGB888 format (32-bit pixels, 8 bits per color)
                let bytes_per_pixel = 4;
                let expected_data_size = rect.dimensions.width as usize * rect.dimensions.height as usize * bytes_per_pixel;
                
                if rect.data.len() != expected_data_size {
                    warn!(
                        "Rectangle data size mismatch: expected {}, got {}",
                        expected_data_size,
                        rect.data.len()
                    );
                    continue;
                }

                for y in 0..rect.dimensions.height {
                    for x in 0..rect.dimensions.width {
                        // Calculate pixel index safely using checked arithmetic
                        let row_offset = match (y as usize).checked_mul(rect.dimensions.width as usize) {
                            Some(offset) => offset,
                            None => {
                                warn!("Integer overflow in row offset calculation");
                                continue;
                            }
                        };
                        let pixel_offset = match row_offset.checked_add(x as usize) {
                            Some(offset) => offset,
                            None => {
                                warn!("Integer overflow in pixel offset calculation");
                                continue;
                            }
                        };
                        let pixel_idx = match pixel_offset.checked_mul(bytes_per_pixel) {
                            Some(idx) => idx,
                            None => {
                                warn!("Integer overflow in pixel index calculation");
                                continue;
                            }
                        };

                        if pixel_idx + 2 >= rect.data.len() {
                            warn!("Pixel index out of bounds");
                            continue;
                        }

                        // Convert coordinates safely
                        let img_x = rect.position.x.max(0) as u32 + x as u32;
                        let img_y = rect.position.y.max(0) as u32 + y as u32;
                        if img_x < width && img_y < height {
                            // RGB888 format, assuming little-endian
                            let b = rect.data[pixel_idx];
                            let g = rect.data[pixel_idx + 1];
                            let r = rect.data[pixel_idx + 2];
                            img.put_pixel(img_x, img_y, Rgb([r, g, b]));
                        }
                    }
                }
            }
            _ => {
                info!("Skipping non-RAW encoded rectangle");
            }
        }
    }

    // Save the image
    let filename = format!("vnc_frame_{}.png", frame_number);
    info!("Saving frame to {}", filename);
    img.save(filename)?;

    Ok(())
}
*/

async fn handle_continuous_updates(
    read_half: &mut rfb::clientv2::ClientReadHalf<tokio::net::TcpStream>,
    write_half: &mut rfb::clientv2::ClientWriteHalf<tokio::net::TcpStream>,
    resolution: Resolution,
) -> Result<(), Box<dyn std::error::Error>> {
    // Enable continuous updates
    // write_half.set_continuous_updates(true, Position { x: 0, y: 0 }, resolution.clone()).await?;

    // Initial request to get the full framebuffer
    write_half.framebuffer_update_request(
        false,
        Position { x: 0, y: 0 },
        resolution.clone(),
    ).await?;

    let mut count = 0;

    // Now just receive updates continuously
    loop {
        let server_message = read_half.read_message().await?;
        count += 1;
        match server_message {
            ServerMessage::FramebufferUpdate(update) => {
                let pixels = update.rectangles.iter()
                    .map(|rect| &rect.dimensions)
                    .map(|resolution| resolution.width as f64 * resolution.height as f64)
                    .sum::<f64>();
                let mut res = None;
                if update.rectangles.len() == 1 {
                    res = Some(update.rectangles[0].dimensions.clone());
                }
                info!("Received update with {} rectangles, {} pixels, {:.2} MiB, resolution={:?}", update.rectangles.len(), pixels, (pixels * 4.0) / 1024.0 / 1024.0, res);
                // return Ok(());

                write_half.framebuffer_update_request(
                        true,
                        Position { x: 0, y: 0 },
                        resolution.clone(),
                    ).await?;



            }
            _ => {
                info!("Received unknown message");
            }
        }

        if count > 10 {
            info!("Received {} updates, exiting", count);
            return Ok(());
        }
        // Process the update here...
    }
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

    stream.set_nodelay(true)?;

    // Create client configuration
    let config = VncClientConfig {
        version: ProtoVersion::Rfb38,
        shared: false,
    };

    // Initialize the VNC client
    let mut client = Client::init_conn(config, stream).await?;

    let initial_resolution = client.get_initial_resolution().clone();

    let (mut read_half, mut write_half) = client.split();

    // Tell qemu vnc server to send continuous updates
    let encodings = vec![
        rfb::encodings::EncodingType::Raw,
        rfb::encodings::EncodingType::DesktopSizePseudo,
        rfb::encodings::EncodingType::ContinuousUpdates,
    ];
    write_half.set_encodings(encodings).await?;

    // Start handling continuous updates
    handle_continuous_updates(&mut read_half, &mut write_half, initial_resolution).await?;

    // Request framebuffer updates 5 times
    //for i in 1..=5 {
    //    info!("Update request #{}", i);
    //    request_framebuffer(&mut client, i, initial_resolution.clone()).await?;
    //}

    Ok(())
}
