use std::net::SocketAddr;
use log::{error, info, debug};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt};
use futures::future::BoxFuture;
use futures::FutureExt;
use crate::{encodings::EncodingType, keysym::KeySym, rfb::{ClientInit, ClientMessage, CutText, FramebufferUpdate, FramebufferUpdateEncoded, FramebufferUpdateRequest, KeyEvent, MouseButtons, PixelFormat, PointerEvent, Position, ProtoVersion, ProtocolError, ReadMessage, Resolution, SecurityResult, SecurityType, SecurityTypes, ServerInit, SetColorMapEntries, WriteMessage, _ServerMessage}};

/// Immutable state
pub struct VncClientConfig {
    pub version: ProtoVersion,
    /// Whether we're willing to share the screen with other clients
    pub shared: bool,
    // pub sec_types: SecurityTypes,
    // pub name: String,
}

pub struct Client<C> {
    config: VncClientConfig,
    conn: C,
    server_init: ServerInit,
}

pub struct ClientReadHalf<C> {
    conn: tokio::io::ReadHalf<C>,
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

    pub fn get_initial_resolution(&self) -> &Resolution {
        &self.server_init.initial_res
    }

    pub fn get_pixel_format(&self) -> &PixelFormat {
        &self.server_init.pixel_format
    }

    pub fn split(self) -> (ClientReadHalf<C>, ClientWriteHalf<C>) {
        let (rh, wh) = tokio::io::split(self.conn);
        (ClientReadHalf { conn: rh }, ClientWriteHalf { conn: wh })
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
}

pub enum ServerMessage {
    FramebufferUpdate(FramebufferUpdateEncoded),
    // SetColorMapEntries(SetColorMapEntries),
    Bell,
    // ServerCutText(CutText),
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
                0 => {
                    let update = FramebufferUpdateEncoded::read_from(stream).await?;
                    Ok(ServerMessage::FramebufferUpdate(update))
                }
                // 1 => {
                //     let entries = SetColorMapEntries::read_from(stream).await?;
                //     Ok(ServerMessage::SetColorMapEntries(entries))
                // }
                2 => Ok(ServerMessage::Bell),
                // 3 => {
                //     let text = CutText::read_from(stream).await?;
                //     Ok(ServerMessage::ServerCutText(text))
                // }
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
        let msg = ServerMessage::read_from(&mut self.conn).await?;
        Ok(msg)
    }
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