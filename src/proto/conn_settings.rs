use std::sync::Arc;

use crate::{
    codec::framed_read::HttpFramedRead,
    frame::{self, Settings},
    preface::{Preface, PrefaceFramed},
    proto::{buffer::Buffer, store::Store},
};

use bytes::{BufMut, BytesMut};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf},
    sync::Mutex,
};
use tokio_util::codec::{
    FramedRead as TokioFramedRead, LengthDelimitedCodec, length_delimited,
};

pub struct ConnSettings {
    client_curr_settings: Settings,
    server_curr_settings: Settings,
    client_settings_to_ack: Option<Settings>,
    server_settings_to_ack: Option<Settings>,
}

impl ConnSettings {
    fn extract_settings(
        settings: &mut Option<frame::Settings>,
        is_acked: bool,
    ) -> (frame::Settings, Option<frame::Settings>) {
        if is_acked {
            (
                settings
                    .take()
                    .expect("Settings must be present when acknowledged"),
                None,
            )
        } else {
            (frame::Settings::default(), settings.take())
        }
    }
}

impl<T, E> From<&mut PrefaceFramed<T, E>> for ConnSettings
where
    T: AsyncRead + AsyncWrite + Unpin,
    E: AsyncRead + AsyncWrite + Unpin,
{
    fn from(preface: &mut PrefaceFramed<T, E>) -> ConnSettings {
        let (client_curr_settings, client_settings_to_ack) =
            Self::extract_settings(
                &mut preface.client_settings,
                preface.is_client_settings_ack,
            );
        let (server_curr_settings, server_settings_to_ack) =
            Self::extract_settings(
                &mut preface.server_settings,
                preface.is_server_settings_ack,
            );
        Self {
            client_curr_settings,
            server_curr_settings,
            client_settings_to_ack,
            server_settings_to_ack,
        }
    }
}
