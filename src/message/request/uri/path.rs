use crate::hpack::BytesStr;

#[derive(Debug)]
pub struct PathAndQuery(BytesStr);

impl PathAndQuery {
    pub fn new(input: BytesStr) -> Self {
        PathAndQuery(input)
    }

    pub(super) fn slash() -> Self {
        PathAndQuery(BytesStr::from_static("/"))
    }

    pub(super) fn star() -> Self {
        PathAndQuery(BytesStr::from_static("*"))
    }

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

impl From<&[u8]> for PathAndQuery {
    fn from(value: &[u8]) -> Self {
        PathAndQuery(BytesStr::unchecked_from_slice(value))
    }
}
