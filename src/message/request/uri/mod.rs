use authority::Authority;
use bytes::Bytes;
mod authority;
mod path;
mod scheme;
use crate::hpack::BytesStr;
use path::PathAndQuery;
pub use scheme::Scheme;

#[derive(Default, Debug)]
pub struct Uri {
    pub authority: Option<Authority>,
    pub scheme: Option<Scheme>, // TODO! OWN variant
    pub path_and_query: Option<PathAndQuery>,
}

impl Uri {
    pub fn new(
        authority: Option<Authority>,
        scheme: Option<Scheme>,
        path_and_query: Option<PathAndQuery>,
    ) -> Uri {
        Uri {
            authority,
            scheme,
            path_and_query,
        }
    }

    pub fn authority(mut self, a: BytesStr) -> Self {
        self.authority = Some(Authority::new(a));
        self
    }

    pub fn path(mut self, p: BytesStr) -> Self {
        self.path_and_query = Some(PathAndQuery::new(p));
        self
    }

    pub fn scheme(mut self, s: Scheme) -> Self {
        self.scheme = Some(s);
        self
    }
}
