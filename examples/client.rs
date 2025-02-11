use std::net::ToSocketAddrs;
use clap::Parser;
use log::{info, warn, LevelFilter};
use rfb::{
    client::{Client, VncClientConfig},
    rfb::{ProtoVersion, Position, Resolution, PixelFormat, ColorSpecification, ColorFormat},
};
use image::{ImageBuffer, Rgb};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// VNC server address (e.g. "localhost:5900")
    #[arg(value_name = "HOST:PORT")]
    address: String,
}

async fn request_framebuffer(
    client: &mut Client<tokio::net::TcpStream>,
    frame_number: usize
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Requesting framebuffer update");
    let update = client.framebuffer_update_request(
        false,
        Position { x: 0, y: 0 },
        Resolution { width: 1024, height: 768 },
    ).await?;

    // Create an image buffer to store the pixels
    let mut img = ImageBuffer::new(1024, 768);

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
                        if img_x < 1024 && img_y < 768 {
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

    // Request framebuffer updates 5 times
    for i in 1..=5 {
        info!("Update request #{}", i);
        request_framebuffer(&mut client, i).await?;
    }

    Ok(())
}
