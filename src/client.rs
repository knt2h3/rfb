use std::net::SocketAddr;
use log::{error, info};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use crate::rfb::{ClientInit, ClientMessage, PixelFormat, ProtoVersion, ProtocolError, ReadMessage, SecurityResult, SecurityType, SecurityTypes, WriteMessage};

/// Immutable state
pub struct VncClientConfig {
    pub version: ProtoVersion,
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
    // pub fn new(config: VncClientConfig) -> Self {
    //     Self { config }
    // }

    pub async fn init_conn(config: VncClientConfig, conn: C) -> Result<Self, HandshakeError> {
        let mut client = Client {
            config,
            conn,
        };

        client.rfb_handshake().await?;
        client.rfb_init().await;

        Ok(client)
    }

    // SetPixelFormat(PixelFormat),
    // SetEncodings(Vec<EncodingType>),
    // FramebufferUpdateRequest(FramebufferUpdateRequest),
    // KeyEvent(KeyEvent),
    // PointerEvent(PointerEvent),
    // ClientCutText(String),

    pub async fn set_pixel_format(&mut self, pixel_format: PixelFormat) -> Result<(), ProtocolError> {
        let msg = ClientMessage::SetPixelFormat(pixel_format);

        ClientMessage::read_from(&mut self.conn).await?;

        todo!()
    }

    async fn rfb_handshake(
        &mut self,
        // s: &mut (impl AsyncRead + AsyncWrite + Unpin + Send),
    ) -> Result<(), HandshakeError> {
        // ProtocolVersion handshake
        let server_version = ProtoVersion::read_from(&mut self.conn).await?;
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
        // info!("Rx [{:?}]: Server SupportedSecurityTypes={:?}", addr, supported_types);

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

        assert!(matches!(res, SecurityResult::Success));

        Ok(())
    }

    async fn rfb_init(
        &mut self,
        // s: &mut (impl AsyncRead + AsyncWrite + Unpin + Send),
    ) {
        let init = ClientInit {
            // we're not willing to share the screen with other clients
            shared: false
        };

        // init.write_to(&mut stream).await?;

        todo!()
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