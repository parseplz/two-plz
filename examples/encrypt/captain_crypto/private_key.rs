use std::fs::read_to_string;

use rustls_pemfile::{Item, read_one_from_slice};
use rustls_pki_types::PrivateKeyDer;

use super::CryptoBuildError;

// https://github.com/rustls/pemfile/issues/39

// Convert &str (PRIVATE_KEY) to PrivateKeyDer
pub fn str_to_private(
    pk: &str,
) -> Result<PrivateKeyDer<'static>, CryptoBuildError> {
    let new_key = read_one_from_slice(pk.as_bytes())
        .unwrap()
        .unwrap()
        .0;
    if let Item::Pkcs8Key(val) = new_key {
        Ok(val.into())
    } else {
        Err(CryptoBuildError::UnknownPrivateKeyType)
    }
}

/* Description:
 *      Function to read from file and convert to PrivateKeyDer
 *
 * Steps:
 *      Read from file, $HOME/.config/zxc/private.key
 *
 * Error:
 *      CryptoBuildError::Var [1]
 *      CryptoBuildError::Read [2]
 */

pub fn read_private() -> Result<String, CryptoBuildError> {
    let mut path = std::env::current_dir()?;
    path.push("./examples/ca/private.key");
    Ok(read_to_string(path)?)
}
