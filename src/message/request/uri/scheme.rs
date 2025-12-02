use crate::hpack::BytesStr;

// Require the scheme to not be too long in order to enable further
// optimizations later.
// const MAX_SCHEME_LEN: usize = 64;

pub enum OwnScheme<T = Box<BytesStr>> {
    None,
    Standard(Protocol),
    Other(T),
}

impl OwnScheme {
    pub const HTTP: OwnScheme = OwnScheme::Standard(Protocol::Http);
    pub const HTTPS: OwnScheme = OwnScheme::Standard(Protocol::Https);
    pub const EMPTY: OwnScheme = OwnScheme::None;

    pub fn as_str(&self) -> &str {
        use self::OwnScheme::*;
        use self::Protocol::*;

        match self {
            Standard(Http) => "http",
            Standard(Https) => "https",
            Other(v) => &v[..],
            None => unreachable!(),
        }
    }
}

impl PartialEq for OwnScheme {
    fn eq(&self, other: &OwnScheme) -> bool {
        use self::OwnScheme::*;
        use self::Protocol::*;

        match (self, other) {
            (&Standard(Http), &Standard(Http)) => true,
            (&Standard(Https), &Standard(Https)) => true,
            (&Other(ref a), &Other(ref b)) => a == b,
            (&None, &None) => unreachable!(),
            _ => false,
        }
    }
}

impl From<&[u8]> for OwnScheme {
    fn from(value: &[u8]) -> Self {
        match value {
            b"http" => Protocol::Http.into(),
            b"https" => Protocol::Https.into(),
            _ => {
                // TODO: needed ?
                //if s.len() > MAX_SCHEME_LEN {
                //    return Err(ErrorKind::SchemeTooLong.into());
                //}
                OwnScheme::Other(BytesStr::unchecked_from_slice(value).into())
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Protocol {
    Http,
    Https,
}

impl Protocol {
    pub(super) fn len(&self) -> usize {
        match *self {
            Protocol::Http => 4,
            Protocol::Https => 5,
        }
    }
}

impl From<Protocol> for OwnScheme {
    fn from(src: Protocol) -> Self {
        OwnScheme::Standard(src)
    }
}
