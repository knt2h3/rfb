use std::net::SocketAddr;
use log::{error, info, debug};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use crate::{encodings::EncodingType, keysym::KeySym, rfb::{ClientInit, ClientMessage, FramebufferUpdateEncoded, FramebufferUpdateRequest, KeyEvent, MouseButtons, PixelFormat, PointerEvent, Position, ProtoVersion, ProtocolError, ReadMessage, Resolution, SecurityResult, SecurityType, SecurityTypes, ServerInit, WriteMessage}};

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
}

impl<C> Client<C>
where
    C: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub async fn init_conn(config: VncClientConfig, conn: C) -> Result<Self, HandshakeError> {
        let mut client = Client {
            config,
            conn,
        };

        client.rfb_handshake().await?;
        client.rfb_init().await;

        Ok(client)
    }

    pub async fn set_pixel_format(&mut self, pixel_format: PixelFormat) -> Result<(), ProtocolError> {
        let msg = ClientMessage::SetPixelFormat(pixel_format);
        msg.write_to(&mut self.conn).await
    }

    pub async fn set_encodings(&mut self, encodings: Vec<EncodingType>) -> Result<(), ProtocolError> {
        let msg = ClientMessage::SetEncodings(encodings);
        msg.write_to(&mut self.conn).await
    }

    pub async fn framebuffer_update_request(&mut self, incremental: bool, position: Position, resolution: Resolution) -> Result<FramebufferUpdateEncoded, ProtocolError> {
        let msg = ClientMessage::FramebufferUpdateRequest(FramebufferUpdateRequest { incremental, position, resolution });
        msg.write_to(&mut self.conn).await?;

        let encoded = FramebufferUpdateEncoded::read_from(&mut self.conn).await?;
        Ok(encoded)
    }

    pub async fn key_event(&mut self, is_pressed: bool, keysym: KeySym, keysym_raw: u32) -> Result<(), ProtocolError> {
        let msg = ClientMessage::KeyEvent(KeyEvent { is_pressed, keysym, keysym_raw });
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

    pub fn split(self) {
        let (rh, wh) = tokio::io::split(self.conn);
        // (Client { config: self.config, conn: rh }, Client { config: self.config, conn: wh })
    }

    async fn rfb_handshake(
        &mut self,
        // s: &mut (impl AsyncRead + AsyncWrite + Unpin + Send),
    ) -> Result<(), HandshakeError> {
        // ProtocolVersion handshake
        let server_version = ProtoVersion::read_from(&mut self.conn).await?;
        debug!("Rx: ServerVersion={:?}", server_version);
        // info!("Rx [{:?}]: ServerVersion={:?}", addr, server_version);
        // info!("Tx [{:?}]: ProtoVersion={:?}", addr, self.config.version);
        self.config.version.write_to(&mut self.conn).await?;

        /*
        if client_version < self.config.version {
            let err_str = format!(
                "[{:?}] unsupported client version={:?} (server version: {:?})",
                addr, client_version, self.config.version
            );
            error!("{}", err_str);
            return Err(HandshakeError::IncompatibleVersions {
                client: client_version,
                server: self.config.version,
            });
        }
         */

        // Security Handshake
        // let supported_types = self.config.sec_types.clone();
        let supported_types = SecurityTypes::read_from(&mut self.conn).await?;
        info!("Rx: Server SupportedSecurityTypes={:?}", supported_types);

        // info!("Tx [{:?}]: SecurityTypes={:?}", addr, supported_types);
        // supported_types.write_to(s).await?;
        // let client_choice = SecurityType::read_from(s).await?;

        let client_choice = SecurityType::None;
        client_choice.write_to(&mut self.conn).await?;

        /*
        info!("Rx [{:?}]: SecurityType Choice={:?}", addr, client_choice);
        if !self.config.sec_types.0.contains(&client_choice) {
            info!("Tx [{:?}]: SecurityResult=Failure", addr);
            let failure = SecurityResult::Failure("unsupported security type".to_string());
            failure.write_to(s).await?;
            let err_str = format!("invalid security choice={:?}", client_choice);
            error!("{}", err_str);
            return Err(HandshakeError::IncompatibleSecurityTypes {
                choice: client_choice,
                offer: self.config.sec_types.clone(),
            });
        }
         */


        let res = SecurityResult::read_from(&mut self.conn).await?;

        // let res = SecurityResult::Success;
        // info!("Tx: SecurityResult=Success");
        // res.write_to(s).await?;

        info!("Rx: SecurityResult={:?}", res);
        assert!(matches!(res, SecurityResult::Success));

        Ok(())
    }

    async fn rfb_init(
        &mut self,
        // s: &mut (impl AsyncRead + AsyncWrite + Unpin + Send),
    ) -> Result<(), ProtocolError> {
        let init = ClientInit {
            shared: self.config.shared,
        };

        init.write_to(&mut self.conn).await?;

        let server_init = ServerInit::read_from(&mut self.conn).await?;
        info!("Rx: ServerInit={:?}", server_init);

        Ok(())
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