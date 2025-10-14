use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::Connection;
use crate::preface::PrefaceErrorState;
use crate::proto::config::ConnectionConfig;
use crate::proto::connection::{ClientConnection, ClientHandler};
use crate::{
    Settings, StreamId,
    preface::{PrefaceConn, PrefaceError, PrefaceState},
    proto,
};
use std::time::Duration;

#[derive(PartialEq, Clone)]
pub enum Role {
    Client,
    Server,
}

impl Role {
    pub fn is_server(&self) -> bool {
        matches!(self, Self::Server)
    }

    pub fn is_client(&self) -> bool {
        matches!(self, Self::Client)
    }

    pub fn init_stream_id(&self) -> StreamId {
        if self.is_server() {
            2.into()
        } else {
            1.into()
        }
    }
}

pub struct Builder {
    /// connection level flow control window size.
    pub initial_connection_window_size: Option<u32>,

    /// Maximum number of locally reset streams due to protocol error across
    /// the lifetime of the connection.
    ///
    /// When this gets exceeded, we issue GOAWAYs.
    pub local_max_error_reset_streams: Option<usize>,

    /// Time to keep locally reset streams around before reaping.
    pub reset_stream_duration: Duration,

    /// Maximum number of locally reset streams to keep at a time.
    pub reset_stream_max: usize,

    pub role: Role,

    /// Initial `Settings` frame to send as part of the handshake.
    pub settings: Settings,
}

impl Builder {
    fn new(role: Role) -> Self {
        let mut settings = Settings::default();
        settings.set_enable_push(false);
        Builder {
            role,
            initial_connection_window_size: None,
            local_max_error_reset_streams: None,
            reset_stream_duration: Duration::from_secs(
                proto::DEFAULT_RESET_STREAM_SECS,
            ),
            reset_stream_max: proto::DEFAULT_RESET_STREAM_MAX,
            settings,
        }
    }

    pub fn server() -> Self {
        Builder::new(Role::Server)
    }

    pub fn client() -> Self {
        Builder::new(Role::Client)
    }

    // ===== Flow Control =====

    // connection level
    pub fn initial_connection_window_size(&mut self, size: u32) -> &mut Self {
        self.initial_connection_window_size = Some(size);
        self
    }

    // stream level
    pub fn initial_window_size(&mut self, size: u32) -> &mut Self {
        self.settings
            .set_initial_window_size(Some(size));
        self
    }

    // ====== Settings frame =====

    /// Indicates the size (in octets) of the largest HTTP/2 frame payload that
    /// the configured client is able to accept.
    ///
    /// The sender may send data frames that are **smaller** than this value,
    /// but any data larger than `max` will be broken up into multiple `DATA`
    /// frames.
    ///
    /// The value **must** be between 16,384 and 16,777,215. The default value
    /// is 16,384.
    pub fn max_frame_size(&mut self, max: u32) -> &mut Self {
        self.settings
            .set_max_frame_size(Some(max));
        self
    }

    /// Sets the max size of received header frames.
    ///
    /// This advisory setting informs a peer of the maximum size of header list
    /// that the sender is prepared to accept, in octets. The value is based on
    /// the uncompressed size of header fields, including the length of the name
    /// and value in octets plus an overhead of 32 octets for each header field.
    ///
    /// This setting is also used to limit the maximum amount of data that is
    /// buffered to decode HEADERS frames.
    pub fn max_header_list_size(&mut self, max: u32) -> &mut Self {
        self.settings
            .set_max_header_list_size(Some(max));
        self
    }

    /// Sets the maximum number of concurrent streams.
    ///
    /// The maximum concurrent streams setting only controls the maximum number
    /// of streams that can be initiated by the remote peer. In other words,
    /// when this setting is set to 100, this does not limit the number of
    /// concurrent streams that can be created by the caller.
    ///
    /// It is recommended that this value be no smaller than 100, so as to not
    /// unnecessarily limit parallelism. However, any value is legal, including
    /// 0. If `max` is set to 0, then the remote will not be permitted to
    /// initiate streams.
    ///
    /// Note that streams in the reserved state, i.e., push promises that have
    /// been reserved but the stream has not started, do not count against this
    /// setting.
    ///
    /// Also note that if the remote *does* exceed the value set here, it is not
    /// a protocol level error. Instead, the `h2` library will immediately reset
    /// the stream.
    ///
    /// See [Section 5.1.2] in the HTTP/2 spec for more details.
    ///
    /// [Section 5.1.2]: https://http2.github.io/http2-spec/#rfc.section.5.1.2
    pub fn max_concurrent_streams(&mut self, max: u32) -> &mut Self {
        self.settings
            .set_max_concurrent_streams(Some(max));
        self
    }

    /// Sets the header table size.
    ///
    /// This setting informs the peer of the maximum size of the header compression
    /// table used to encode header blocks, in octets. The encoder may select any value
    /// equal to or less than the header table size specified by the sender.
    ///
    /// The default value is 4,096.
    pub fn header_table_size(&mut self, size: u32) -> &mut Self {
        self.settings
            .set_header_table_size(Some(size));
        self
    }

    // ===== Reset =====

    /// Sets the maximum number of concurrent locally reset streams.
    ///
    /// When a stream is explicitly reset, the HTTP/2 specification requires
    /// that any further frames received for that stream must be ignored for
    /// "some time".
    ///
    /// In order to satisfy the specification, internal state must be maintained
    /// to implement the behavior. This state grows linearly with the number of
    /// streams that are locally reset.
    ///
    /// The `max_concurrent_reset_streams` setting configures sets an upper
    /// bound on the amount of state that is maintained. When this max value is
    /// reached, the oldest reset stream is purged from memory.
    ///
    /// Once the stream has been fully purged from memory, any additional frames
    /// received for that stream will result in a connection level protocol
    /// error, forcing the connection to terminate.
    ///
    /// The default value is currently 50.
    pub fn max_concurrent_reset_streams(&mut self, max: usize) -> &mut Self {
        self.reset_stream_max = max;
        self
    }

    /// Sets the duration to remember locally reset streams.
    ///
    /// When a stream is explicitly reset, the HTTP/2 specification requires
    /// that any further frames received for that stream must be ignored for
    /// "some time".
    ///
    /// In order to satisfy the specification, internal state must be maintained
    /// to implement the behavior. This state grows linearly with the number of
    /// streams that are locally reset.
    ///
    /// The `reset_stream_duration` setting configures the max amount of time
    /// this state will be maintained in memory. Once the duration elapses, the
    /// stream state is purged from memory.
    ///
    /// Once the stream has been fully purged from memory, any additional frames
    /// received for that stream will result in a connection level protocol
    /// error, forcing the connection to terminate.
    ///
    /// The default value is currently 1 second.
    pub fn reset_stream_duration(&mut self, dur: Duration) -> &mut Self {
        self.reset_stream_duration = dur;
        self
    }

    /// Sets the maximum number of local resets due to protocol errors made by
    /// the remote end.
    ///
    /// Invalid frames and many other protocol errors will lead to resets being
    /// generated for those streams.
    /// Too many of these often indicate a malicious client, and there are
    /// attacks which can abuse this to DOS servers. This limit protects
    /// against these DOS attacks by limiting the amount of resets we can be
    /// forced to generate.
    ///
    /// When the number of local resets exceeds this threshold, the client will
    /// close the connection.
    ///
    /// If you really want to disable this, supply [`Option::None`] here.
    /// Disabling this is not recommended and may expose you to DOS attacks.
    ///
    /// The default value is currently 1024, but could change.
    pub fn max_local_error_reset_streams(
        &mut self,
        max: Option<usize>,
    ) -> &mut Self {
        self.local_max_error_reset_streams = max;
        self
    }

    // TODO
    // Sets the first stream ID to something other than 1.
    //pub fn initial_stream_id(&mut self, stream_id: u32) -> &mut Self {
    //    self.stream_id = stream_id.into();
    //    assert!(self.stream_id.is_client_initiated(), "stream id must be odd");
    //    self
    //}
    //

    pub async fn handshake<T>(
        mut self,
        io: T,
    ) -> Result<(Builder, PrefaceState<T>), PrefaceError>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        let mut state =
            PrefaceState::new(io, self.role.clone(), self.settings.clone());
        // TODO: Generic state runner
        loop {
            state = state.next().await?;
            if state.is_ended() {
                break;
            }
        }
        Ok((self, state))
    }
}

pub struct ClientBuilder {
    builder: Builder,
}

impl ClientBuilder {
    pub fn new() -> Self {
        ClientBuilder {
            builder: Builder::new(Role::Client),
        }
    }

    pub async fn handshake<T>(
        self,
        io: T,
    ) -> Result<(ClientConnection<T>, ClientHandler), PrefaceError>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        let (builder, state) = self.builder.handshake(io).await?;
        let mut preface = PrefaceConn::try_from(state)?;
        let remote_settings = preface.take_remote_settings();
        let config = ConnectionConfig::from((builder, remote_settings));
        Ok(Connection::new(preface.role, config, preface.stream))
    }
}

pub struct ServerBuilder {
    builder: Builder,
}

impl ServerBuilder {
    pub fn new() -> Self {
        ServerBuilder {
            builder: Builder::new(Role::Server),
        }
    }

    pub async fn handshake<T>(
        self,
        io: T,
    ) -> Result<(ClientConnection<T>, ClientHandler), PrefaceError>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        let (builder, state) = self.builder.handshake(io).await?;
        let mut preface = PrefaceConn::try_from(state)?;
        let remote_settings = preface.take_remote_settings();
        let config = ConnectionConfig::from((builder, remote_settings));
        Ok(Connection::new(preface.role, config, preface.stream))
    }
}
