use std::fs::read_to_string;
use std::sync::Arc;

use openssl::hash::DigestBytes;
use rcgen::{Certificate, CertificateParams, KeyPair};
use tokio_rustls::rustls::ServerConfig;

use super::CryptoBuildError;

/* Destiption:
 *      CA Certificate is used to generate self signeed tls certificates for
 * the domains
 *
 * There are two CA's:
 *      1. Trusted      : user generated and trusted
 *      2. Untrusted    : per session generated
 *
 * TODO:
 *      Use HashMap instead of Vec
 *      https://github.com/sfackler/rust-openssl/pull/2299
 */

pub struct CA {
    cert: Certificate,
    store: Vec<(DigestBytes, Arc<ServerConfig>)>,
}

impl CA {
    pub fn trusted(key_pair: &KeyPair) -> Result<CA, CryptoBuildError> {
        let mut cert_path = std::env::current_dir()?;
        cert_path.push("ca/ca.crt");
        let cert_str = read_to_string(cert_path)?;
        let cert_params = CertificateParams::from_ca_cert_pem(&cert_str)?;
        let cert = cert_params.self_signed(key_pair)?;
        Ok(CA {
            cert,
            store: Vec::new(),
        })
    }

    // Per session CA Certificate
    pub fn untrusted(key_pair: &KeyPair) -> Result<CA, CryptoBuildError> {
        let cert_params = CertificateParams::default();
        let cert = cert_params.self_signed(key_pair)?;
        Ok(CA {
            cert,
            store: Vec::new(),
        })
    }

    pub fn cert(&self) -> &Certificate {
        &self.cert
    }

    pub fn store(&self) -> &Vec<(DigestBytes, Arc<ServerConfig>)> {
        &self.store
    }

    pub fn add_config(
        &mut self,
        digest: DigestBytes,
        config: Arc<ServerConfig>,
    ) {
        self.store.push((digest, config));
    }
}
