use log::{info, debug};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt};
use futures::future::BoxFuture;
use futures::FutureExt;
use crate::{encodings::EncodingType, keysym::KeySym, rfb::{ClientInit, ClientMessage, FramebufferUpdateEncoded, FramebufferUpdateRequest, KeyEvent, MouseButtons, PixelFormat, PointerEvent, Position, ProtoVersion, ProtocolError, ReadMessage, RectangleEncoded, Resolution, Screen, SecurityResult, SecurityType, SecurityTypes, ServerInit, SetDesktopSize, WriteMessage}};

/// Immutable state
pub struct VncClientConfig {
    pub version: ProtoVersion,
    /// Whether we're willing to share the screen with other clients
    pub shared: bool,
    // pub sec_types: SecurityTypes,
    // pub name: String,
}

pub struct Client<C> {
    #[allow(dead_code)] // stored for parity with the constructors; nothing reads it yet
    config: VncClientConfig,
    conn: C,
    server_init: ServerInit,
}

pub struct ClientReadHalf<C> {
    conn: tokio::io::ReadHalf<C>,
    /// Persistent zlib inflate stream for the Zlib encoding (RFB 7.7.6): a single
    /// stream spans every Zlib rectangle of the connection, so the decompressor
    /// must be retained across `read_message` calls.
    zlib: flate2::Decompress,
}

pub struct ClientWriteHalf<C> {
    conn: tokio::io::WriteHalf<C>,
}

impl<C> Client<C>
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub async fn init_conn(config: VncClientConfig, mut conn: C) -> Result<Self, HandshakeError> {
        rfb_handshake(&mut conn, &config).await?;
        let server_init = rfb_init(&mut conn, &config).await?;

        let client = Client {
            config,
            conn,
            server_init,
        };

        Ok(client)
    }

    pub async fn create_from_initialized_conn(config: VncClientConfig, conn: C, server_init: ServerInit) -> Result<Self, HandshakeError> {
        let client = Client {
            config,
            conn,
            server_init,
        };

        Ok(client)
    }

    pub fn get_initial_resolution(&self) -> &Resolution {
        &self.server_init.initial_res
    }

    pub fn get_pixel_format(&self) -> &PixelFormat {
        &self.server_init.pixel_format
    }

    pub fn split(self) -> (ClientReadHalf<C>, ClientWriteHalf<C>) {
        let (rh, wh) = tokio::io::split(self.conn);
        (
            ClientReadHalf {
                conn: rh,
                // `true`: expect a zlib header (RFC 1950) — VNC Zlib uses zlib format.
                zlib: flate2::Decompress::new(true),
            },
            ClientWriteHalf { conn: wh },
        )
    }
}

impl<C> ClientWriteHalf<C>
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub async fn set_pixel_format(&mut self, pixel_format: PixelFormat) -> Result<(), ProtocolError> {
        let msg = ClientMessage::SetPixelFormat(pixel_format);
        msg.write_to(&mut self.conn).await
    }

    pub async fn set_encodings(&mut self, encodings: Vec<EncodingType>) -> Result<(), ProtocolError> {
        let msg = ClientMessage::SetEncodings(encodings);
        msg.write_to(&mut self.conn).await
    }

    pub async fn framebuffer_update_request(&mut self, incremental: bool, position: Position, resolution: Resolution) -> Result<(), ProtocolError> {
        let msg = ClientMessage::FramebufferUpdateRequest(FramebufferUpdateRequest { incremental, position, resolution });
        msg.write_to(&mut self.conn).await
    }

    pub async fn key_event(&mut self, is_pressed: bool, keysym_raw: u32) -> Result<(), ProtocolError> {
        let unused = KeySym::Ascii(ascii::AsciiChar::A);
        let msg = ClientMessage::KeyEvent(KeyEvent { is_pressed, keysym: unused, keysym_raw });
        msg.write_to(&mut self.conn).await
    }

    pub async fn pointer_event(&mut self, position: Position, pressed: MouseButtons) -> Result<(), ProtocolError> {
        let msg = ClientMessage::PointerEvent(PointerEvent { position, pressed });
        msg.write_to(&mut self.conn).await
    }

    pub async fn client_cut_text(&mut self, text: String) -> Result<(), ProtocolError> {
        let msg = ClientMessage::ClientCutText(text);
        msg.write_to(&mut self.conn).await
    }

    /// Ask the server for a `width` x `height` desktop, as one screen. Only
    /// after the server has sent an ExtendedDesktopSize rectangle.
    pub async fn set_desktop_size(&mut self, width: u16, height: u16) -> Result<(), ProtocolError> {
        let resolution = Resolution { width, height };
        let screen = Screen { id: 0, position: Position { x: 0, y: 0 }, dimensions: resolution, flags: 0 };
        let msg = ClientMessage::SetDesktopSize(SetDesktopSize { resolution, screens: vec![screen] });
        msg.write_to(&mut self.conn).await
    }
}

pub enum ServerMessage {
    FramebufferUpdate(FramebufferUpdateEncoded),
    // SetColorMapEntries(SetColorMapEntries),
    Bell,
    ServerCutText(CutText),
}

#[derive(Debug)]
pub struct CutText {
    pub text: Vec<u8>,
}

#[async_trait::async_trait]
impl ReadMessage for ServerMessage {
    fn read_from<'a, R>(stream: &'a mut R) -> BoxFuture<'a, Result<Self, ProtocolError>>
    where
        R: AsyncRead + Unpin + Send,
    {
        async move {
            let msg_type = stream.read_u8().await?;
            match msg_type {
                // FramebufferUpdate
                0 => {
                    // We cannot use the FramebufferUpdateEncoded::read_from method here, because it will read the message type as well
                    // let update = FramebufferUpdateEncoded::read_from(stream).await?;
                    // Ok(ServerMessage::FramebufferUpdate(update))
                    stream.read_u8().await?;  // padding

                    let n_rect = stream.read_u16().await?;
                    let mut rectangles = Vec::with_capacity(n_rect as usize);

                    for _ in 0..n_rect {
                        let rect = RectangleEncoded::read_from(stream).await?;
                        rectangles.push(rect);
                    }

                    Ok(ServerMessage::FramebufferUpdate(FramebufferUpdateEncoded { rectangles }))
                }
                // 1 => {
                //     let entries = SetColorMapEntries::read_from(stream).await?;
                //     Ok(ServerMessage::SetColorMapEntries(entries))
                // }
                2 => Ok(ServerMessage::Bell),
                3 => {
                    // https://vncdotool.readthedocs.io/en/0.8.0/rfbproto.html#servercuttext
                    // let text = CutText::read_from(stream).await?;
                    // three bytes of padding
                    let mut padding = [0_u8; 3];
                    stream.read_exact(&mut padding).await?;

                    let length = stream.read_u32().await?;
                    let mut text = vec![0_u8; length as usize];
                    stream.read_exact(&mut text).await?;

                    let cut_text = CutText { text };

                    Ok(ServerMessage::ServerCutText(cut_text))
                }
                _ => Err(ProtocolError::UnknownServerMessageType(msg_type)),
            }
        }.boxed()
    }
}

impl<C> ClientReadHalf<C>
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub async fn read_message(&mut self) -> Result<ServerMessage, ProtocolError> {
        // Stateful read: framebuffer rectangles may use the Zlib encoding, whose
        // inflate stream (`self.zlib`) is persistent across rectangles, so we can't
        // use the stateless `ServerMessage::read_from`.
        let msg_type = self.conn.read_u8().await?;
        match msg_type {
            // FramebufferUpdate
            0 => {
                self.conn.read_u8().await?; // padding
                let n_rect = self.conn.read_u16().await?;
                let mut rectangles = Vec::with_capacity(n_rect as usize);
                for _ in 0..n_rect {
                    let rect = read_rectangle(&mut self.conn, &mut self.zlib).await?;
                    rectangles.push(rect);
                }
                Ok(ServerMessage::FramebufferUpdate(FramebufferUpdateEncoded {
                    rectangles,
                }))
            }
            // Bell
            2 => Ok(ServerMessage::Bell),
            // ServerCutText
            3 => {
                let mut padding = [0_u8; 3];
                self.conn.read_exact(&mut padding).await?;
                let length = self.conn.read_u32().await?;
                let mut text = vec![0_u8; length as usize];
                self.conn.read_exact(&mut text).await?;
                Ok(ServerMessage::ServerCutText(CutText { text }))
            }
            _ => Err(ProtocolError::UnknownServerMessageType(msg_type)),
        }
    }
}

/// Read one encoded rectangle, decoding it where the client (rather than the
/// browser) is the right place to do so. Zlib rectangles are inflated here (using
/// the connection's persistent `zlib` stream) and presented downstream as `Raw`
/// pixels; everything else is passed through unchanged.
async fn read_rectangle<R>(
    stream: &mut R,
    zlib: &mut flate2::Decompress,
) -> Result<RectangleEncoded, ProtocolError>
where
    R: AsyncRead + Unpin + Send,
{
    let x = stream.read_u16().await?;
    let y = stream.read_u16().await?;
    let width = stream.read_u16().await?;
    let height = stream.read_u16().await?;
    let encoding_type = EncodingType::from(stream.read_i32().await?);

    let position = Position { x, y };
    let dimensions = Resolution { width, height };
    // The pixel format negotiated for this client is 32bpp (4 bytes/pixel); the Raw
    // path assumes the same.
    let raw_len = width as usize * height as usize * 4;

    match encoding_type {
        EncodingType::Raw => {
            let mut data = vec![0u8; raw_len];
            stream.read_exact(&mut data).await?;
            Ok(RectangleEncoded {
                position,
                dimensions,
                encoding_type,
                data,
            })
        }
        EncodingType::Zlib => {
            // 4-byte length, then that many bytes fed into the persistent stream.
            let len = stream.read_u32().await? as usize;
            let mut compressed = vec![0u8; len];
            stream.read_exact(&mut compressed).await?;
            let data = zlib_inflate(zlib, &compressed, raw_len)?;
            // Normalize to Raw: `data` is now raw pixels, so the wasm/JS layers
            // treat compressed and uncompressed rectangles identically.
            Ok(RectangleEncoded {
                position,
                dimensions,
                encoding_type: EncodingType::Raw,
                data,
            })
        }
        EncodingType::CopyRect => {
            // 2-byte source x, 2-byte source y. The client copies the on-screen
            // region; no pixel data on the wire.
            let mut data = vec![0u8; 4];
            stream.read_exact(&mut data).await?;
            Ok(RectangleEncoded {
                position,
                dimensions,
                encoding_type,
                data,
            })
        }
        EncodingType::QemuExtendedKeyEvents => Ok(RectangleEncoded {
            position,
            dimensions,
            encoding_type,
            data: Vec::new(),
        }),
        EncodingType::ExtendedDesktopSize => {
            let number_of_screens = stream.read_u8().await?;
            let mut padding = [0u8; 3];
            stream.read_exact(&mut padding).await?;
            let mut data = vec![0u8; number_of_screens as usize * 16];
            stream.read_exact(&mut data).await?;
            Ok(RectangleEncoded {
                position,
                dimensions,
                encoding_type,
                data,
            })
        }
        other => Err(ProtocolError::UnsupportedEncoding(other.into())),
    }
}

/// Inflate one Zlib rectangle into exactly `expected` raw bytes, advancing the
/// persistent zlib stream.
fn zlib_inflate(
    zlib: &mut flate2::Decompress,
    mut input: &[u8],
    expected: usize,
) -> Result<Vec<u8>, ProtocolError> {
    use flate2::{FlushDecompress, Status};

    let mut out = vec![0u8; expected];
    let mut written = 0usize;
    while written < expected {
        let in_before = zlib.total_in();
        let out_before = zlib.total_out();
        let status = zlib
            .decompress(input, &mut out[written..], FlushDecompress::Sync)
            .map_err(|e| ProtocolError::Decompress(e.to_string()))?;
        let consumed = (zlib.total_in() - in_before) as usize;
        let produced = (zlib.total_out() - out_before) as usize;
        input = &input[consumed..];
        written += produced;

        if matches!(status, Status::StreamEnd) {
            break;
        }
        if produced == 0 && consumed == 0 {
            return Err(ProtocolError::Decompress(
                "zlib made no progress decoding rectangle".to_string(),
            ));
        }
    }

    if written != expected {
        return Err(ProtocolError::Decompress(format!(
            "zlib produced {written} of {expected} expected bytes"
        )));
    }
    Ok(out)
}

#[derive(Debug, Error)]
pub enum HandshakeError {
    #[error("incompatible protocol versions (client = {client:?}, server = {server:?})")]
    IncompatibleVersions {
        client: ProtoVersion,
        server: ProtoVersion,
    },

    #[error(
        "incompatible security types (client choice = {choice:?}, server offered = {offer:?})"
    )]
    IncompatibleSecurityTypes {
        choice: SecurityType,
        offer: SecurityTypes,
    },

    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

async fn rfb_handshake(conn: &mut (impl AsyncRead + AsyncWrite + Unpin + Send), config: &VncClientConfig) -> Result<(), HandshakeError> {
    // ProtocolVersion handshake
    let server_version = ProtoVersion::read_from(conn).await?;
    debug!("Rx: ServerVersion={:?}", server_version);
    config.version.write_to(conn).await?;

    // Security Handshake
    let supported_types = SecurityTypes::read_from(conn).await?;
    info!("Rx: Server SupportedSecurityTypes={:?}", supported_types);

    let client_choice = SecurityType::None;
    client_choice.write_to(conn).await?;

    let res = SecurityResult::read_from(conn).await?;

    info!("Rx: SecurityResult={:?}", res);
    assert!(matches!(res, SecurityResult::Success));

    Ok(())
}

async fn rfb_init(conn: &mut (impl AsyncRead + AsyncWrite + Unpin + Send), config: &VncClientConfig) -> Result<ServerInit, ProtocolError> {
    let init = ClientInit {
        shared: config.shared,
    };

    init.write_to(conn).await?;

    let server_init = ServerInit::read_from(conn).await?;
    info!("Rx: ServerInit={:?}", server_init);

    Ok(server_init)
}
#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    /// Compress `pixels` into the persistent `compress` stream and frame them as
    /// one on-wire Zlib rectangle (x, y, w, h, encoding=6, length, data).
    fn zlib_rect_wire(
        compress: &mut Compress,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        pixels: &[u8],
    ) -> Vec<u8> {
        let mut compressed = Vec::new();
        let mut input = pixels;
        loop {
            let in_before = compress.total_in();
            let out_before = compress.total_out();
            let mut buf = vec![0u8; pixels.len() + 1024];
            compress
                .compress(input, &mut buf, FlushCompress::Sync)
                .unwrap();
            let consumed = (compress.total_in() - in_before) as usize;
            let produced = (compress.total_out() - out_before) as usize;
            compressed.extend_from_slice(&buf[..produced]);
            input = &input[consumed..];
            if input.is_empty() && produced < buf.len() {
                break;
            }
        }

        let mut wire = Vec::new();
        wire.extend_from_slice(&x.to_be_bytes());
        wire.extend_from_slice(&y.to_be_bytes());
        wire.extend_from_slice(&w.to_be_bytes());
        wire.extend_from_slice(&h.to_be_bytes());
        wire.extend_from_slice(&6i32.to_be_bytes()); // EncodingType::Zlib
        wire.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        wire.extend_from_slice(&compressed);
        wire
    }

    // A Zlib rectangle must inflate to the original pixels, and the inflate stream
    // must persist across rectangles (the 2nd rectangle's compressed bytes depend
    // on the 1st's dictionary). Regression guard for the Zlib decoder.
    #[tokio::test]
    async fn zlib_rectangles_round_trip_over_persistent_stream() {
        let (w, h) = (8u16, 8u16);
        let n = w as usize * h as usize * 4;
        let pixels1: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        let pixels2: Vec<u8> = (0..n).map(|i| ((i * 7 + 3) % 251) as u8).collect();

        let mut compress = Compress::new(Compression::default(), true);
        let wire1 = zlib_rect_wire(&mut compress, 0, 0, w, h, &pixels1);
        let wire2 = zlib_rect_wire(&mut compress, 8, 8, w, h, &pixels2);

        let mut zlib = flate2::Decompress::new(true);

        let mut r1: &[u8] = &wire1;
        let rect1 = read_rectangle(&mut r1, &mut zlib).await.unwrap();
        // Zlib rectangles are normalized to Raw for the downstream layers.
        assert_eq!(rect1.encoding_type, EncodingType::Raw);
        assert_eq!(rect1.dimensions.width, w);
        assert_eq!(rect1.data, pixels1);

        let mut r2: &[u8] = &wire2;
        let rect2 = read_rectangle(&mut r2, &mut zlib).await.unwrap();
        assert_eq!(rect2.position.x, 8);
        assert_eq!(rect2.data, pixels2);
    }
}
