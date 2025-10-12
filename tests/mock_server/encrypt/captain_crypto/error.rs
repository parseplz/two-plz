use std::{env, io};

use thiserror::Error;
use tokio_rustls::rustls::server::VerifierBuilderError;
use tokio_rustls::rustls::{self};

#[derive(Debug, Error)]
pub enum CertError {
    #[error("rcgen| {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("rustls| {0}")]
    Rustls(#[from] rustls::Error),
}

#[derive(Debug, Error)]
pub enum CryptoBuildError {
    #[error("home var| {0}")]
    Var(#[from] env::VarError),
    #[error("verifierbuild| {0}")]
    VerifierBuild(#[from] VerifierBuilderError),
    #[error("read file| {0}")]
    Read(#[from] io::Error),
    #[error("rcgen| {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("unknown private key type")]
    UnknownPrivateKeyType,
}
