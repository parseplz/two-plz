use http::uri::Scheme;

use crate::hpack::BytesStr;

#[derive(Debug, Default)]
pub struct Authority(BytesStr);

#[derive(Debug, Default)]
pub struct PathAndQuery(BytesStr);

#[derive(Debug)]
pub struct Uri {
    authority: Authority,
    scheme: Scheme,
    path_and_query: PathAndQuery,
}

// ===== Builder =====
#[derive(Default)]
pub struct UriBuilder {
    pub authority: Option<Authority>,
    pub scheme: Option<Scheme>, // TODO! OWN variant
    path_and_query: Option<PathAndQuery>,
}

impl UriBuilder {
    pub fn authority(mut self, a: BytesStr) -> Self {
        self.authority = Some(Authority(a));
        self
    }

    pub fn path(mut self, p: BytesStr) -> Self {
        self.path_and_query = Some(PathAndQuery(p));
        self
    }

    pub fn build(mut self) -> Uri {
        Uri {
            authority: self.authority.unwrap_or_default(),
            scheme: self.scheme.unwrap(),
            path_and_query: self.path_and_query.unwrap_or_default(),
        }
    }
}
