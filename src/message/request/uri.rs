use http::uri::Scheme;

use crate::hpack::BytesStr;

#[derive(Debug)]
pub struct Authority(BytesStr);

impl Authority {
    pub fn into_inner(self) -> BytesStr {
        self.0
    }

    #[inline]
    pub(crate) fn as_str(&self) -> &str {
        &self.0[..]
    }
}

#[derive(Debug)]
pub struct PathAndQuery(BytesStr);

impl PathAndQuery {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_inner(self) -> BytesStr {
        self.0
    }

    #[inline]
    pub(crate) fn as_str(&self) -> &str {
        let ret = &self.0[..];
        if ret.is_empty() {
            return "/";
        }
        ret
    }
}

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
        self.authority = Some(Authority(a));
        self
    }

    pub fn path(mut self, p: BytesStr) -> Self {
        self.path_and_query = Some(PathAndQuery(p));
        self
    }

    pub fn scheme(mut self, s: Scheme) -> Self {
        self.scheme = Some(s);
        self
    }
}
