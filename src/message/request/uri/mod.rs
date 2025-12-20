mod scheme;
use crate::hpack::BytesStr;
pub use scheme::Scheme;

#[derive(Default, Debug)]
pub struct Uri {
    pub authority: Option<BytesStr>,
    pub scheme: Option<Scheme>, // TODO! OWN variant
    pub path_and_query: Option<BytesStr>,
}

impl Uri {
    pub fn new(
        authority: Option<BytesStr>,
        scheme: Option<Scheme>,
        path_and_query: Option<BytesStr>,
    ) -> Uri {
        Uri {
            authority,
            scheme,
            path_and_query,
        }
    }

    pub fn set_authority(mut self, a: BytesStr) -> Self {
        self.authority = Some(a);
        self
    }

    pub fn set_path(mut self, p: BytesStr) -> Self {
        self.path_and_query = Some(p);
        self
    }

    pub fn set_scheme(mut self, s: Scheme) -> Self {
        self.scheme = Some(s);
        self
    }
}
